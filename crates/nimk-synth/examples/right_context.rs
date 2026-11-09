//! Offline experiment: how much does RIGHT context (the next word's keys)
//! help the language decision for the pending word, compared with the
//! left-only context the engine uses today?
//!
//! Not part of the product: nothing here is used by the engine, the bundled
//! models or the CLI. See `docs/research/right-context-experiment.md`.
//!
//! ```sh
//! cargo run --release -p nimk-synth --example right_context -- [--sessions 200] [--words 500]
//!     [--seed 42] [--epochs 3] [--runs 2] [--legacy 1]
//! ```
//!
//! Data: `generate(SynthConfig { seed, sessions, words, ..default })`, the
//! last 10% of scripts held out, exactly like `nimk-synth pretrain` (same
//! seed/sessions/words = same held-out scripts). One example per word.
//!
//! Variants (same GRU, same token vocabulary, same training budget):
//! - `word`  : `SEP raw boundary` + the word's Telex flags.
//! - `left`  : today's commit-time view: the last 24 tokens of
//!   `history ++ SEP ++ raw ++ boundary` + Telex flags + `prev_langs`
//!   (teacher labels in training, the model's own decisions at eval).
//! - `left+right`: `left` ++ `RSEP ++ keys up to the end of the next word`
//!   + the next word's Telex flags.
//! - `right` : `word` ++ the right part (no history, no `prev_langs`).
//!
//! Plus Bayes-optimal accuracies computed from the generator's own
//! probabilities (upper bounds for each context set).
#![allow(clippy::print_stdout, clippy::needless_range_loop, missing_docs)]

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use burn::module::{Module, ModuleMapper, Param};
use burn::nn::gru::{Gru, GruConfig};
use burn::nn::{Embedding, EmbeddingConfig, Initializer, Linear, LinearConfig};
use burn::optim::{AdamConfig, GradientsParams};
use burn::tensor::activation::{relu, sigmoid};
use burn::tensor::{Device, Int, Tensor, TensorData};
use nimk_synth::{Corpus, MixProfile, Script, SynthConfig, generate, held_out};
use nimk_types::Lang;

// ---------------------------------------------------------------- tokens

const PAD: u8 = 128;
const SEP: u8 = 129;
const RSEP: u8 = 130;
const SPACE: u8 = 131;
const RETURN: u8 = 132;
const OTHER: u8 = 133;
const VOCAB: usize = 134;
/// Today's encoder window (`nimk_policy::encode::SEQ_LEN`).
const LEFT_LEN: usize = 24;
/// Right part: `RSEP` + boundary residue + next word, capped.
const RIGHT_LEN: usize = 16;
const SEQ: usize = LEFT_LEN + RIGHT_LEN;
const SCALARS: usize = 10;

fn char_tok(c: char) -> u8 {
    match c {
        ' ' => SPACE,
        '\r' | '\n' => RETURN,
        '\x21'..='\x7e' => c as u8,
        _ => OTHER,
    }
}

// ---------------------------------------------------------------- words

#[derive(Debug, Clone)]
struct Word {
    lower: String,
    lang: Lang,
    /// Telex text != raw and a valid syllable (the eval's definition).
    ambiguous: bool,
    /// Raw keys the generator can emit for both languages.
    collision: bool,
    /// `[valid_vi, viable_vi, telex_changed]`.
    flags: [f32; 3],
    /// Token span of the raw keys in the script's stream.
    start: usize,
    end: usize,
    /// Sentence index within the script.
    sentence: usize,
}

struct Doc {
    stream: Vec<u8>,
    words: Vec<Word>,
}

fn telex_flags(raw: &str) -> ([f32; 3], bool) {
    let t = nimk_telex::transform(raw);
    let b = |v: bool| if v { 1.0 } else { 0.0 };
    (
        [b(t.valid_vi), b(t.viable_vi), b(t.text != raw)],
        t.text != raw && t.valid_vi,
    )
}

/// Splits a script's key stream into words (letter runs) and checks they
/// line up with `script.words`.
fn doc_of(script: &Script, collisions: &HashSet<String>) -> Result<Doc, String> {
    let chars: Vec<char> = script
        .keys
        .iter()
        .map(|k| k.chars.chars().next().unwrap_or('\u{0}'))
        .collect();
    let stream: Vec<u8> = chars.iter().map(|&c| char_tok(c)).collect();
    let mut words = Vec::with_capacity(script.words.len());
    let mut i = 0;
    let mut sentence = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_alphabetic() {
            i += 1;
        }
        let raw: String = chars[start..i].iter().collect();
        let sw = script
            .words
            .get(words.len())
            .ok_or_else(|| format!("{}: more letter runs than words", script.session))?;
        if sw.raw != raw {
            return Err(format!("{}: run {raw} != word {}", script.session, sw.raw));
        }
        let (flags, ambiguous) = telex_flags(&raw);
        let lower = raw.to_lowercase();
        words.push(Word {
            collision: collisions.contains(&lower),
            lower,
            lang: sw.lang,
            ambiguous,
            flags,
            start,
            end: i,
            sentence,
        });
        if matches!(chars.get(i), Some('.' | '?' | '!')) {
            sentence += 1;
        }
    }
    if words.len() != script.words.len() {
        return Err(format!("{}: word count mismatch", script.session));
    }
    Ok(Doc { stream, words })
}

// ---------------------------------------------------------------- features

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    Word,
    Left,
    LeftRight,
    Right,
}

impl Variant {
    const ALL: [Self; 4] = [Self::Word, Self::Left, Self::LeftRight, Self::Right];

    fn name(self) -> &'static str {
        match self {
            Self::Word => "word",
            Self::Left => "left",
            Self::LeftRight => "left+right",
            Self::Right => "right",
        }
    }

    fn left(self) -> bool {
        matches!(self, Self::Left | Self::LeftRight)
    }

    fn right(self) -> bool {
        matches!(self, Self::LeftRight | Self::Right)
    }
}

#[derive(Clone, Copy)]
struct Features {
    tokens: [u8; SEQ],
    scalars: [f32; SCALARS],
}

/// Features of word `i`; `prev` are the languages of the previous words
/// (most recent last), used only by the left variants.
fn features(doc: &Doc, i: usize, v: Variant, prev: &[Lang]) -> Features {
    let w = &doc.words[i];
    // Boundary key that commits the word today (`,` `.` space...).
    let boundary_end = (w.end + 1).min(doc.stream.len());
    let mut main: Vec<u8> = Vec::with_capacity(SEQ);
    if v.left() {
        main.extend_from_slice(&doc.stream[..w.start]);
    }
    main.push(SEP);
    main.extend_from_slice(&doc.stream[w.start..boundary_end]);
    // Today's encoder keeps the newest LEFT_LEN tokens.
    let main = &main[main.len().saturating_sub(LEFT_LEN)..];
    let mut right: Vec<u8> = Vec::new();
    let next = doc.words.get(i + 1);
    if v.right() {
        right.push(RSEP);
        if let Some(n) = next {
            right.extend_from_slice(&doc.stream[boundary_end..n.end]);
        }
        right.truncate(RIGHT_LEN);
    }
    let mut tokens = [PAD; SEQ];
    let all: Vec<u8> = main.iter().chain(&right).copied().collect();
    tokens[SEQ - all.len()..].copy_from_slice(&all);

    let mut s = [0.0f32; SCALARS];
    s[..3].copy_from_slice(&w.flags);
    if v.left() {
        for (k, lang) in prev.iter().rev().take(3).enumerate() {
            s[3 + k] = if *lang == Lang::Vi { 1.0 } else { -1.0 };
        }
    }
    if v.right()
        && let Some(n) = next
    {
        s[6..9].copy_from_slice(&n.flags);
        s[9] = 1.0;
    }
    Features { tokens, scalars: s }
}

// ---------------------------------------------------------------- network

const TOK_DIM: usize = 32;
const HIDDEN: usize = 64;
const MLP: usize = 64;

#[derive(Module, Debug)]
struct Net {
    tok: Embedding,
    gru: Gru,
    hidden: Linear,
    render: Linear,
}

impl Net {
    fn init(seed: u64, device: &Device) -> Self {
        let z = Initializer::Zeros;
        let net = Self {
            tok: EmbeddingConfig::new(VOCAB, TOK_DIM)
                .with_initializer(z.clone())
                .init(device),
            gru: GruConfig::new(TOK_DIM, HIDDEN, true)
                .with_initializer(z.clone())
                .init(device),
            hidden: LinearConfig::new(HIDDEN + SCALARS, MLP)
                .with_initializer(z.clone())
                .init(device),
            render: LinearConfig::new(MLP, 1).with_initializer(z).init(device),
        };
        net.map(&mut SeededInit::new(seed))
    }

    /// Render logits `[B]`.
    fn forward(&self, rows: &[Features], device: &Device) -> Tensor<1> {
        let b = rows.len();
        let mut tokens = Vec::with_capacity(b * SEQ);
        let mut scalars = Vec::with_capacity(b * SCALARS);
        for r in rows {
            tokens.extend(r.tokens.iter().map(|&t| i64::from(t)));
            scalars.extend_from_slice(&r.scalars);
        }
        let tokens = Tensor::<2, Int>::from_data(TensorData::new(tokens, [b, SEQ]), device);
        let scalars = Tensor::<2>::from_data(TensorData::new(scalars, [b, SCALARS]), device);
        let states = self.gru.forward(self.tok.forward(tokens), None);
        let last = states.slice([0..b, SEQ - 1..SEQ]).reshape([b, HIDDEN]);
        let z = relu(self.hidden.forward(Tensor::cat(vec![last, scalars], 1)));
        self.render.forward(z).reshape([b])
    }
}

/// Deterministic init, as in `nimk_policy::gru_net::SeededInit`.
struct SeededInit {
    state: u64,
    path: Vec<String>,
}

impl SeededInit {
    fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9e37_79b9_7f4a_7c15,
            path: Vec::new(),
        }
    }

    fn next_unit(&mut self) -> f32 {
        ((splitmix(&mut self.state) >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }
}

impl ModuleMapper for SeededInit {
    fn enter_module(&mut self, name: &str, _container_type: &str) {
        self.path.push(name.to_owned());
    }

    fn exit_module(&mut self, _name: &str, _container_type: &str) {
        self.path.pop();
    }

    fn map_float<const D: usize>(&mut self, param: Param<Tensor<D>>) -> Param<Tensor<D>> {
        let head = self.path.iter().any(|p| p == "render");
        let embedding = self.path.iter().any(|p| p == "tok");
        param.map(|t| {
            let dims = t.shape().dims::<D>();
            let n: usize = dims.iter().product();
            let scale = if head || D == 1 {
                0.0
            } else if embedding {
                0.5
            } else {
                (6.0 / dims.iter().sum::<usize>().max(1) as f32).sqrt()
            };
            let values: Vec<f32> = (0..n).map(|_| self.next_unit() * scale).collect();
            Tensor::<D>::from_data(TensorData::new(values, dims), &t.device())
                .set_require_grad(t.is_require_grad())
        })
    }
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Same regularizer spirit as `GruAction`: 10% of tokens -> `OTHER`.
fn dropout(f: &mut Features, rng: &mut u64) {
    for t in &mut f.tokens {
        if *t != PAD && splitmix(rng).is_multiple_of(10) {
            *t = OTHER;
        }
    }
}

fn train(docs: &[Doc], v: Variant, epochs: usize, seed: u64) -> Result<Net, String> {
    let device = Device::flex().autodiff();
    let mut net = Net::init(seed, &device);
    let mut optim = AdamConfig::new().init();
    let mut examples: Vec<(Features, f32)> = Vec::new();
    for doc in docs {
        let mut prev = Vec::new();
        for (i, w) in doc.words.iter().enumerate() {
            examples.push((features(doc, i, v, &prev), w.lang.as_target()));
            prev.push(w.lang);
        }
    }
    let mut rng = seed ^ 0xd1b5_4a32_d192_ed03;
    let batch = 128;
    for epoch in 0..epochs {
        // Fisher-Yates.
        for i in (1..examples.len()).rev() {
            let j = (splitmix(&mut rng) % (i as u64 + 1)) as usize;
            examples.swap(i, j);
        }
        let mut total = 0.0f64;
        let mut n = 0usize;
        for chunk in examples.chunks(batch) {
            let mut rows: Vec<Features> = chunk.iter().map(|e| e.0).collect();
            for r in &mut rows {
                dropout(r, &mut rng);
            }
            let b = rows.len();
            let y = Tensor::<1>::from_data(
                TensorData::new(chunk.iter().map(|e| e.1).collect::<Vec<f32>>(), [b]),
                &device,
            );
            let x = net.forward(&rows, &device);
            let bce = x.clone().clamp_min(0.0) - x.clone() * y
                + x.abs().neg().exp().add_scalar(1.0).log();
            let loss = bce.mean();
            total += f64::from(loss.clone().into_scalar::<f32>());
            n += 1;
            let grads = GradientsParams::from_grads(loss.backward(), &net);
            net = optim.step(3e-3, net, grads);
        }
        println!(
            "  [{}] epoch {epoch}: loss {:.4}",
            v.name(),
            total / n.max(1) as f64
        );
    }
    Ok(net.valid())
}

/// Sequential held-out decoding: `prev_langs` are the model's own decisions.
/// Returns `p_vi` per word, per doc.
fn predict(net: &Net, docs: &[Doc], v: Variant) -> Result<Vec<Vec<f32>>, String> {
    let device = Device::flex();
    let mut out: Vec<Vec<f32>> = docs
        .iter()
        .map(|d| Vec::with_capacity(d.words.len()))
        .collect();
    let mut prev: Vec<Vec<Lang>> = vec![Vec::new(); docs.len()];
    let max = docs.iter().map(|d| d.words.len()).max().unwrap_or(0);
    for i in 0..max {
        let idx: Vec<usize> = (0..docs.len())
            .filter(|&d| i < docs[d].words.len())
            .collect();
        let rows: Vec<Features> = idx
            .iter()
            .map(|&d| features(&docs[d], i, v, &prev[d]))
            .collect();
        let p = sigmoid(net.forward(&rows, &device))
            .into_data()
            .try_into_vec::<f32>()
            .map_err(|e| format!("{e:?}"))?;
        for (&d, &p) in idx.iter().zip(&p) {
            out[d].push(p);
            prev[d].push(Lang::from_p_vi(p));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- bayes

/// Generator likelihoods `P(raw | lang)` over lowercase raw keys.
struct Likelihood {
    vi: HashMap<String, f64>,
    en: HashMap<String, f64>,
    mix: MixProfile,
}

impl Likelihood {
    fn new(corpus: &Corpus, mix: &MixProfile) -> Self {
        let mut vi = HashMap::new();
        let nv = corpus.vi.len() as f64;
        for w in &corpus.vi {
            let variants = nimk_telex::encode_variants(w);
            let canon = nimk_telex::encode(w).to_lowercase();
            if variants.is_empty() {
                *vi.entry(canon).or_insert(0.0) += 1.0 / nv;
            } else {
                let pv = f64::from(mix.p_variant);
                *vi.entry(canon).or_insert(0.0) += (1.0 - pv) / nv;
                for v in &variants {
                    *vi.entry(v.to_lowercase()).or_insert(0.0) += pv / variants.len() as f64 / nv;
                }
            }
        }
        let mut en = HashMap::new();
        let pa = f64::from(mix.p_ambiguous);
        for w in &corpus.en {
            *en.entry(w.clone()).or_insert(0.0) += (1.0 - pa) / corpus.en.len() as f64;
        }
        for w in &corpus.en_ambiguous {
            *en.entry(w.clone()).or_insert(0.0) += pa / corpus.en_ambiguous.len() as f64;
        }
        Self {
            vi,
            en,
            mix: mix.clone(),
        }
    }

    fn collisions(&self) -> HashSet<String> {
        self.vi
            .keys()
            .filter(|k| self.en.contains_key(*k))
            .cloned()
            .collect()
    }

    fn p(&self, raw: &str, lang: Lang) -> f64 {
        let m = match lang {
            Lang::Vi => &self.vi,
            Lang::En => &self.en,
        };
        m.get(raw).copied().unwrap_or(0.0)
    }

    /// `P(word lang | sentence main)`.
    fn p_lang(&self, lang: Lang, main: Lang) -> f64 {
        let swap = f64::from(match main {
            Lang::Vi => self.mix.p_en_in_vi,
            Lang::En => self.mix.p_vi_in_en,
        });
        if lang == main { 1.0 - swap } else { swap }
    }

    /// Bayes `P(Vi)` for word `i` given the raws of `ctx` (same sentence).
    fn posterior(&self, words: &[Word], i: usize, ctx: &[usize]) -> f64 {
        let mut score = [0.0f64; 2];
        for (k, lang) in [Lang::Vi, Lang::En].into_iter().enumerate() {
            for main in [Lang::Vi, Lang::En] {
                let prior = if main == Lang::Vi {
                    f64::from(self.mix.p_vi_sentence)
                } else {
                    1.0 - f64::from(self.mix.p_vi_sentence)
                };
                let mut s = prior * self.p_lang(lang, main) * self.p(&words[i].lower, lang);
                for &j in ctx {
                    let r = &words[j].lower;
                    s *= self.p_lang(Lang::Vi, main) * self.p(r, Lang::Vi)
                        + self.p_lang(Lang::En, main) * self.p(r, Lang::En);
                }
                score[k] += s;
            }
        }
        let z = score[0] + score[1];
        if z > 0.0 { score[0] / z } else { 0.5 }
    }
}

#[derive(Clone, Copy)]
enum BayesCtx {
    None,
    Left,
    LeftRight1,
    Sentence,
}

fn bayes(lk: &Likelihood, docs: &[Doc], ctx: BayesCtx) -> Vec<Vec<f32>> {
    docs.iter()
        .map(|d| {
            (0..d.words.len())
                .map(|i| {
                    let s = d.words[i].sentence;
                    let same = |j: &usize| d.words[*j].sentence == s && *j != i;
                    let range: Vec<usize> = match ctx {
                        BayesCtx::None => Vec::new(),
                        BayesCtx::Left => (0..i).filter(same).collect(),
                        BayesCtx::LeftRight1 => {
                            (0..(i + 2).min(d.words.len())).filter(same).collect()
                        }
                        BayesCtx::Sentence => (0..d.words.len()).filter(same).collect(),
                    };
                    lk.posterior(&d.words, i, &range) as f32
                })
                .collect()
        })
        .collect()
}

// ---------------------------------------------------------------- HMM study

/// What-if: languages follow a 2-state Markov chain (code-switch spans)
/// instead of the generator's independent per-word swaps. `q` scales the
/// switch rate: `q = 1` is i.i.d. words (61% VI), smaller `q` = longer
/// runs. Words are sampled like the generator (variants, ambiguous EN).
/// Returns `(same-language rate of neighbours, filter, lag-1, full)` accuracy
/// on collision words, Bayes-optimal under the chain.
fn hmm_study(lk: &Likelihood, corpus: &Corpus, q: f64, n: usize, seed: u64) -> [f64; 4] {
    let pi_vi = 0.61;
    // trans[from][to], index 0 = VI, 1 = EN.
    let leave = [q * (1.0 - pi_vi), q * pi_vi];
    let trans = [[1.0 - leave[0], leave[0]], [leave[1], 1.0 - leave[1]]];
    let mut rng = seed;
    let unit = |rng: &mut u64| (splitmix(rng) >> 11) as f64 / (1u64 << 53) as f64;
    let mix = &lk.mix;
    let mut state = usize::from(unit(&mut rng) >= pi_vi);
    let mut raws: Vec<String> = Vec::with_capacity(n);
    let mut langs: Vec<usize> = Vec::with_capacity(n);
    for _ in 0..n {
        let raw = if state == 0 {
            let w = &corpus.vi[(splitmix(&mut rng) % corpus.vi.len() as u64) as usize];
            let variants = nimk_telex::encode_variants(w);
            if !variants.is_empty() && unit(&mut rng) < f64::from(mix.p_variant) {
                variants[(splitmix(&mut rng) % variants.len() as u64) as usize].clone()
            } else {
                nimk_telex::encode(w)
            }
        } else {
            let list = if unit(&mut rng) < f64::from(mix.p_ambiguous) {
                &corpus.en_ambiguous
            } else {
                &corpus.en
            };
            list[(splitmix(&mut rng) % list.len() as u64) as usize].clone()
        };
        raws.push(raw);
        langs.push(state);
        state = usize::from(unit(&mut rng) >= trans[state][0]);
    }
    let emit = |t: usize| [lk.p(&raws[t], Lang::Vi), lk.p(&raws[t], Lang::En)];
    let norm = |v: [f64; 2]| {
        let z = v[0] + v[1];
        if z > 0.0 {
            [v[0] / z, v[1] / z]
        } else {
            [0.5, 0.5]
        }
    };
    // Forward (filter) and backward passes.
    let mut alpha = vec![[0.0; 2]; n];
    let mut prior = [pi_vi, 1.0 - pi_vi];
    for t in 0..n {
        let e = emit(t);
        alpha[t] = norm([prior[0] * e[0], prior[1] * e[1]]);
        prior = [
            alpha[t][0] * trans[0][0] + alpha[t][1] * trans[1][0],
            alpha[t][0] * trans[0][1] + alpha[t][1] * trans[1][1],
        ];
    }
    let mut beta = vec![[1.0; 2]; n];
    for t in (0..n.saturating_sub(1)).rev() {
        let e = emit(t + 1);
        let b =
            |l: usize| trans[l][0] * e[0] * beta[t + 1][0] + trans[l][1] * e[1] * beta[t + 1][1];
        beta[t] = norm([b(0), b(1)]);
    }
    let (mut same, mut cnt, mut hits) = (0usize, 0usize, [0usize; 3]);
    for t in 0..n {
        if t + 1 < n {
            same += usize::from(langs[t] == langs[t + 1]);
        }
        if !(lk.vi.contains_key(&raws[t]) && lk.en.contains_key(&raws[t])) {
            continue;
        }
        cnt += 1;
        let lag1 = if t + 1 < n {
            let e = emit(t + 1);
            let la = |l: usize| alpha[t][l] * (trans[l][0] * e[0] + trans[l][1] * e[1]);
            [la(0), la(1)]
        } else {
            alpha[t]
        };
        let full = [alpha[t][0] * beta[t][0], alpha[t][1] * beta[t][1]];
        for (k, p) in [alpha[t], lag1, full].into_iter().enumerate() {
            let guess = usize::from(p[1] > p[0]);
            hits[k] += usize::from(guess == langs[t]);
        }
    }
    let r = |h: usize| h as f64 / cnt.max(1) as f64;
    [
        same as f64 / (n - 1) as f64,
        r(hits[0]),
        r(hits[1]),
        r(hits[2]),
    ]
}

// ---------------------------------------------------------------- scoring

const PROBE: [&str; 9] = ["in", "an", "me", "no", "ten", "hang", "theme", "to", "then"];

#[derive(Default, Clone)]
struct Score {
    all: (u64, u64),
    amb: (u64, u64),
    coll: (u64, u64),
    /// Collision words whose next word is in the same sentence.
    coll_mid: (u64, u64),
    vi_as_en: u64,
    en_as_vi: u64,
    probe: HashMap<&'static str, (u64, u64)>,
}

fn add(c: &mut (u64, u64), ok: bool) {
    c.0 += 1;
    c.1 += u64::from(ok);
}

fn rate(c: (u64, u64)) -> f64 {
    if c.0 == 0 {
        0.0
    } else {
        c.1 as f64 / c.0 as f64
    }
}

fn score(docs: &[Doc], preds: &[Vec<f32>]) -> Score {
    let mut s = Score::default();
    for (d, p) in docs.iter().zip(preds) {
        for (i, (w, &p)) in d.words.iter().zip(p).enumerate() {
            let guess = Lang::from_p_vi(p);
            let ok = guess == w.lang;
            add(&mut s.all, ok);
            if w.ambiguous {
                add(&mut s.amb, ok);
            }
            if w.collision {
                add(&mut s.coll, ok);
                if d.words.get(i + 1).is_some_and(|n| n.sentence == w.sentence) {
                    add(&mut s.coll_mid, ok);
                }
            }
            if !ok {
                match w.lang {
                    Lang::Vi => s.vi_as_en += 1,
                    Lang::En => s.en_as_vi += 1,
                }
            }
            if let Some(name) = PROBE.iter().find(|n| **n == w.lower) {
                add(s.probe.entry(name).or_default(), ok);
            }
        }
    }
    s
}

fn print_header() {
    println!(
        "| method | overall | ambiguous | collision | collision, mid-sentence | VI->EN | EN->VI |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- |");
}

fn print_row(name: &str, s: &Score) {
    println!(
        "| {name} | {:.4} | {:.4} | {:.4} | {:.4} | {} | {} |",
        rate(s.all),
        rate(s.amb),
        rate(s.coll),
        rate(s.coll_mid),
        s.vi_as_en,
        s.en_as_vi
    );
}

fn print_probe(rows: &[(String, Score)]) {
    print!("| word |");
    for (n, _) in rows {
        print!(" {n} |");
    }
    println!();
    print!("| --- |");
    for _ in rows {
        print!(" --- |");
    }
    println!();
    for w in PROBE {
        let n = rows
            .first()
            .and_then(|r| r.1.probe.get(w))
            .map_or(0, |c| c.0);
        print!("| `{w}` (n={n}) |");
        for (_, s) in rows {
            print!(
                " {:.3} |",
                rate(s.probe.get(w).copied().unwrap_or_default())
            );
        }
        println!();
    }
}

// ---------------------------------------------------------------- latency

fn latency(net: &Net) {
    let device = Device::flex();
    let f = Features {
        tokens: [b'a'; SEQ],
        scalars: [0.0; SCALARS],
    };
    let mut times: Vec<f64> = (0..2000)
        .map(|_| {
            let t = Instant::now();
            let _ = sigmoid(net.forward(&[f], &device)).into_data();
            t.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    times.sort_by(f64::total_cmp);
    println!(
        "latency, 1 row x {SEQ} tokens: median {:.0} us, p99 {:.0} us",
        times[times.len() / 2],
        times[times.len() * 99 / 100]
    );
}

// ---------------------------------------------------------------- main

struct Args {
    synth: SynthConfig,
    epochs: usize,
    runs: u64,
}

fn args() -> Result<Args, String> {
    let mut a = Args {
        synth: SynthConfig {
            sessions: 200,
            // Chat sessions only: the example's word segmentation predates
            // raw command text (ADR 0012). `--legacy` restores the profile
            // the Bayes rows were derived for.
            mix: MixProfile {
                p_terminal: 0.0,
                p_ide: 0.0,
                ..MixProfile::default()
            },
            ..SynthConfig::default()
        },
        epochs: 3,
        runs: 2,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let v = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        let bad = |_| format!("bad value for {flag}: {v}");
        match flag.as_str() {
            "--legacy" => a.synth.mix = MixProfile::legacy(),
            "--sessions" => a.synth.sessions = v.parse().map_err(bad)?,
            "--words" => a.synth.words_per_session = v.parse().map_err(bad)?,
            "--seed" => a.synth.seed = v.parse().map_err(bad)?,
            "--epochs" => a.epochs = v.parse().map_err(bad)?,
            "--runs" => a.runs = v.parse().map_err(bad)?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(a)
}

fn main() -> Result<(), String> {
    let a = args()?;
    let corpus = Corpus::builtin();
    let lk = Likelihood::new(&corpus, &a.synth.mix);
    let collisions = lk.collisions();
    let scripts = generate(&a.synth, &corpus);
    let held = held_out(&scripts, 0.1);
    let train_scripts = &scripts[..scripts.len() - held.len()];
    let to_docs = |s: &[Script]| -> Result<Vec<Doc>, String> {
        s.iter().map(|s| doc_of(s, &collisions)).collect()
    };
    let train_docs = to_docs(train_scripts)?;
    let test_docs = to_docs(held)?;

    let mut coll_list: Vec<&String> = collisions.iter().collect();
    coll_list.sort();
    println!(
        "{} train / {} held-out scripts; {} collision raws: {:?}",
        train_docs.len(),
        test_docs.len(),
        coll_list.len(),
        coll_list
    );
    let test_words: Vec<&Word> = test_docs.iter().flat_map(|d| &d.words).collect();
    let misses = test_words
        .iter()
        .filter(|w| lk.p(&w.lower, w.lang) == 0.0)
        .count();
    println!(
        "held-out words {}, ambiguous {}, collision {} (of which ambiguous {}), likelihood misses {misses}",
        test_words.len(),
        test_words.iter().filter(|w| w.ambiguous).count(),
        test_words.iter().filter(|w| w.collision).count(),
        test_words
            .iter()
            .filter(|w| w.collision && w.ambiguous)
            .count()
    );
    // How often is the next word in the same sentence / same language?
    let (mut same_sent, mut same_lang, mut pairs) = (0u64, 0u64, 0u64);
    for d in &test_docs {
        for w in d.words.windows(2) {
            pairs += 1;
            if w[0].sentence == w[1].sentence {
                same_sent += 1;
                same_lang += u64::from(w[0].lang == w[1].lang);
            }
        }
    }
    println!(
        "next word in same sentence {:.3}; same language given same sentence {:.3}",
        same_sent as f64 / pairs as f64,
        same_lang as f64 / same_sent.max(1) as f64
    );

    let mut probe_rows: Vec<(String, Score)> = Vec::new();
    println!("\n## Bayes-optimal (generator probabilities)\n");
    print_header();
    for (name, ctx) in [
        ("bayes word-only", BayesCtx::None),
        ("bayes left (whole sentence prefix)", BayesCtx::Left),
        ("bayes left + next word", BayesCtx::LeftRight1),
        ("bayes whole sentence", BayesCtx::Sentence),
    ] {
        let s = score(&test_docs, &bayes(&lk, &test_docs, ctx));
        print_row(name, &s);
        probe_rows.push((name.to_owned(), s));
    }

    println!("\n## What-if: Markov code-switch spans (Bayes, collision words)\n");
    println!("| q | same-lang neighbours | left (filter) | left + next word | whole stream |");
    println!("| --- | --- | --- | --- | --- |");
    for q in [1.0, 0.6, 0.3, 0.15, 0.05] {
        let [same, f, l1, full] = hmm_study(&lk, &corpus, q, 200_000, 7);
        println!("| {q} | {same:.3} | {f:.4} | {l1:.4} | {full:.4} |");
    }
    if a.runs == 0 {
        return Ok(());
    }

    println!("\n## GRU word-level models\n");
    let mut results: Vec<(Variant, Vec<Score>)> = Vec::new();
    let mut last_net = None;
    for v in Variant::ALL {
        let mut scores = Vec::new();
        for run in 0..a.runs {
            let t = Instant::now();
            let net = train(&train_docs, v, a.epochs, 0x5eed + run)?;
            let preds = predict(&net, &test_docs, v)?;
            let s = score(&test_docs, &preds);
            println!(
                "  [{}] run {run}: overall {:.4} amb {:.4} coll {:.4} ({:.0} s)",
                v.name(),
                rate(s.all),
                rate(s.amb),
                rate(s.coll),
                t.elapsed().as_secs_f64()
            );
            scores.push(s);
            if v == Variant::LeftRight {
                last_net = Some(net);
            }
        }
        results.push((v, scores));
    }
    println!();
    print_header();
    for (v, scores) in &results {
        for (run, s) in scores.iter().enumerate() {
            print_row(&format!("gru {} run {run}", v.name()), s);
        }
        if let Some(s) = scores.first() {
            probe_rows.push((format!("gru {}", v.name()), s.clone()));
        }
    }
    println!("\n## Probe words (accuracy, run 0)\n");
    print_probe(&probe_rows);
    if let Some(net) = last_net {
        println!();
        latency(&net);
    }
    Ok(())
}
