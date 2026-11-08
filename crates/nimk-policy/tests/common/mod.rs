//! Shared helpers for the action-model tests: key builders, a tiny RNG and a
//! toy key-by-key typing dataset (no dependency on nimk-synth/nimk-telex).
#![allow(dead_code)]

use nimk_policy::{ImeLearner, ImeModel, decide};
use nimk_types::{
    Action, ActionMask, CaretRel, EditAction, Example, KeyEvent, KeyKind, Lang, Modifiers,
    Observation, RECENT_WINDOW, keycode,
};

/// splitmix64.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    pub fn chance(&mut self, p: f64) -> bool {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }

    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            v.swap(i, self.below(i + 1));
        }
    }
}

pub fn key(keycode: u16, chars: &str, mods: u32) -> KeyEvent {
    KeyEvent {
        kind: KeyKind::KeyDown,
        keycode,
        chars: chars.to_owned(),
        mods: Modifiers(mods),
        timestamp: 0.0,
    }
}

pub fn letter(c: char) -> KeyEvent {
    key(0, &c.to_string(), 0)
}

pub fn space() -> KeyEvent {
    key(keycode::SPACE, " ", 0)
}

pub fn backspace() -> KeyEvent {
    key(keycode::BACKSPACE, "\u{7f}", 0)
}

pub fn ret() -> KeyEvent {
    key(keycode::RETURN, "\r", 0)
}

pub fn cmd_c() -> KeyEvent {
    key(8, "c", Modifiers::COMMAND)
}

/// Toy vocabulary: `(raw Telex keys, language, telex(raw) is a valid syllable)`.
pub const WORDS: &[(&str, Lang, bool)] = &[
    ("tieengs", Lang::Vi, true),
    ("vieejt", Lang::Vi, true),
    ("dduwowngf", Lang::Vi, true),
    ("nguwowif", Lang::Vi, true),
    ("nhaf", Lang::Vi, true),
    ("cuar", Lang::Vi, true),
    ("khoong", Lang::Vi, true),
    ("ddi", Lang::Vi, true),
    ("hocj", Lang::Vi, true),
    ("laf", Lang::Vi, true),
    ("muoons", Lang::Vi, true),
    ("bieets", Lang::Vi, true),
    ("ddeepj", Lang::Vi, true),
    ("truwowngf", Lang::Vi, true),
    ("vieen", Lang::Vi, true),
    ("chaof", Lang::Vi, true),
    ("banj", Lang::Vi, true),
    ("tooi", Lang::Vi, true),
    ("yeeu", Lang::Vi, true),
    ("nuwowcs", Lang::Vi, true),
    ("thaays", Lang::Vi, true),
    ("ngayf", Lang::Vi, true),
    ("mootj", Lang::Vi, true),
    ("ddaau", Lang::Vi, true),
    ("vowis", Lang::Vi, true),
    ("cungx", Lang::Vi, true),
    ("ddeemf", Lang::Vi, true),
    ("nhaanj", Lang::Vi, true),
    ("has", Lang::En, true),
    ("for", Lang::En, true),
    ("test", Lang::En, true),
    ("with", Lang::En, false),
    ("user", Lang::En, true),
    ("the", Lang::En, true),
    ("string", Lang::En, false),
    ("class", Lang::En, false),
    ("code", Lang::En, false),
    ("push", Lang::En, false),
    ("merge", Lang::En, false),
    ("build", Lang::En, false),
    ("file", Lang::En, false),
    ("data", Lang::En, false),
    ("rust", Lang::En, false),
    ("branch", Lang::En, false),
    ("commit", Lang::En, false),
    ("debug", Lang::En, false),
    ("server", Lang::En, false),
    ("client", Lang::En, false),
    ("error", Lang::En, false),
    ("value", Lang::En, false),
    ("type", Lang::En, false),
    ("print", Lang::En, false),
    ("query", Lang::En, false),
    ("model", Lang::En, false),
];

/// Every 4th word is held out (both languages represented).
pub fn split_words() -> (Vec<usize>, Vec<usize>) {
    (0..WORDS.len()).partition(|i| i % 4 != 3)
}

/// Crude test-only stand-in for the Telex tool's "did it change" flag.
fn telex_changes(prefix: &str) -> bool {
    let doubled = ["aa", "ee", "oo", "dd"].iter().any(|d| prefix.contains(d));
    let tone = prefix.len() > 1 && prefix.ends_with(['s', 'f', 'r', 'x', 'j']);
    doubled || prefix.contains('w') || tone
}

/// Builds observations/examples while "typing" sessions of words.
pub struct Typist {
    recent: Vec<KeyEvent>,
    raw: String,
    word: usize,
    prev: Vec<Lang>,
    bundle: &'static str,
    caret: Option<CaretRel>,
    pub out: Vec<Example>,
}

impl Typist {
    pub fn new(bundle: &'static str, caret: Option<CaretRel>) -> Self {
        Self {
            recent: Vec::new(),
            raw: String::new(),
            word: 0,
            prev: Vec::new(),
            bundle,
            caret,
            out: Vec::new(),
        }
    }

    fn mode(&self) -> Lang {
        self.prev.first().copied().unwrap_or(Lang::En)
    }

    fn obs(&self, ev: &KeyEvent) -> Observation {
        let (_, lang, valid) = WORDS[self.word];
        let full = WORDS[self.word].0;
        let changed = telex_changes(&self.raw);
        let printable = !ev.mods.is_shortcut()
            && ev
                .chars
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_graphic());
        let start = self.recent.len().saturating_sub(RECENT_WINDOW);
        Observation {
            key: Some(ev.clone()),
            recent: self.recent[start..].to_vec(),
            raw: self.raw.clone(),
            vi: if changed {
                format!("{}\u{301}", self.raw)
            } else {
                self.raw.clone()
            },
            valid_vi: !self.raw.is_empty() && valid && self.raw == full,
            viable_vi: !self.raw.is_empty() && (valid || lang == Lang::Vi),
            shown: self.mode(),
            mode: self.mode(),
            prev_langs: self.prev.iter().copied().take(3).collect(),
            bundle_id: self.bundle.to_owned(),
            caret: self.caret,
            secs_since_switch: f32::INFINITY,
            mask: ActionMask {
                append: printable,
                pop: !self.raw.is_empty(),
                commit: !self.raw.is_empty(),
            },
        }
    }

    fn step(&mut self, ev: KeyEvent, edit: EditAction, render_weight: f32) {
        let lang = WORDS[self.word].1;
        let obs = self.obs(&ev);
        self.out.push(Example {
            obs,
            target: Action { edit, render: lang },
            edit_weight: 1.0,
            render_weight,
        });
        match edit {
            EditAction::Append => self.raw.push_str(&ev.chars),
            EditAction::Pop => {
                self.raw.pop();
            }
            EditAction::Commit => {
                self.raw.clear();
                self.prev.insert(0, lang);
            }
            EditAction::Pass => {}
        }
        self.recent.push(ev);
    }

    /// Types word `w` key by key, with occasional typos, shortcuts and
    /// stray keys around it.
    pub fn word(&mut self, w: usize, rng: &mut Rng) {
        self.word = w;
        let chars: Vec<char> = WORDS[w].0.chars().collect();
        let typo_at = rng.chance(0.15).then(|| rng.below(chars.len()));
        for (i, &c) in chars.iter().enumerate() {
            if typo_at == Some(i) {
                let t = (b'a' + rng.below(26) as u8) as char;
                self.step(letter(t), EditAction::Append, 1.0);
                self.step(backspace(), EditAction::Pop, 1.0);
            }
            if i > 0 && rng.chance(0.03) {
                self.step(cmd_c(), EditAction::Pass, 1.0);
            }
            self.step(letter(c), EditAction::Append, 1.0);
        }
        let end = match rng.below(20) {
            0 | 1 => key(43, ",", 0),
            2 => key(47, ".", 0),
            3 => ret(),
            _ => space(),
        };
        self.step(end, EditAction::Commit, 1.0);
        // Keys with an empty buffer: nothing to edit, no render label.
        if rng.chance(0.08) {
            self.step(backspace(), EditAction::Pass, 0.0);
        }
        if rng.chance(0.05) {
            self.step(cmd_c(), EditAction::Pass, 0.0);
        }
        if rng.chance(0.05) {
            self.step(space(), EditAction::Pass, 0.0);
        }
    }
}

/// `sessions` sessions of 6..12 words drawn from `words`.
pub fn dataset(words: &[usize], sessions: usize, seed: u64) -> Vec<Example> {
    let mut rng = Rng(seed);
    let mut out = Vec::new();
    let bundles = ["com.apple.TextEdit", "com.microsoft.VSCode"];
    for _ in 0..sessions {
        let caret = CaretRel::new(rng.below(10) as f32 / 10.0, rng.below(10) as f32 / 10.0);
        let mut t = Typist::new(bundles[rng.below(2)], caret);
        for _ in 0..6 + rng.below(7) {
            t.word(words[rng.below(words.len())], &mut rng);
        }
        out.append(&mut t.out);
    }
    out
}

/// Accuracy report on a dataset.
#[derive(Debug, Clone, Copy)]
pub struct Eval {
    /// Masked-argmax edit accuracy over all steps.
    pub edit: f64,
    /// Render accuracy at commit steps (the rendering that gets committed).
    pub render_commit: f64,
    /// Render accuracy over every step with a render label.
    pub render_all: f64,
}

pub fn evaluate(model: &dyn ImeModel, data: &[Example]) -> Eval {
    let (mut e_ok, mut c_ok, mut c_n, mut r_ok, mut r_n) = (0, 0, 0, 0, 0);
    for ex in data {
        let a = decide(&model.score(&ex.obs), ex.obs.mask);
        e_ok += usize::from(a.edit == ex.target.edit);
        if ex.render_weight > 0.0 {
            r_n += 1;
            r_ok += usize::from(a.render == ex.target.render);
            if ex.target.edit == EditAction::Commit {
                c_n += 1;
                c_ok += usize::from(a.render == ex.target.render);
            }
        }
    }
    let f = |a: usize, b: usize| a as f64 / b.max(1) as f64;
    Eval {
        edit: f(e_ok, data.len()),
        render_commit: f(c_ok, c_n),
        render_all: f(r_ok, r_n),
    }
}

/// Trains `epochs` passes of shuffled minibatches; returns per-batch losses.
pub fn train(
    learner: &mut dyn ImeLearner,
    data: &[Example],
    epochs: usize,
    batch: usize,
    seed: u64,
) -> Vec<f32> {
    let mut rng = Rng(seed);
    let mut order: Vec<usize> = (0..data.len()).collect();
    let mut losses = Vec::new();
    let mut buf = Vec::with_capacity(batch);
    for _ in 0..epochs {
        rng.shuffle(&mut order);
        for chunk in order.chunks(batch) {
            buf.clear();
            buf.extend(chunk.iter().map(|&i| data[i].clone()));
            losses.push(learner.update(&buf));
        }
    }
    losses
}

pub fn mean(v: &[f32]) -> f32 {
    v.iter().sum::<f32>() / v.len().max(1) as f32
}

/// A random observation (for neutrality checks).
pub fn random_obs(rng: &mut Rng) -> Observation {
    let w = rng.below(WORDS.len());
    let mut t = Typist::new("com.example.app", None);
    t.word(w, rng);
    let i = rng.below(t.out.len());
    t.out.swap_remove(i).obs
}
