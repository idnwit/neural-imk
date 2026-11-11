//! Code-switching sentence generator producing `KeyEvent` scripts.

use nimk_types::{Action, KeyEvent, KeyKind, Lang, Modifiers, keycode};

use crate::keymap::{Rng, key_for};
use crate::track::{Step, Tracker, classify};
use crate::{Corpus, SYNTHETIC_BUNDLE_ID};

/// Sentence length range (words), inclusive.
const SENTENCE_WORDS: (usize, usize) = (4, 12);
/// Probability of a comma after a non-final word.
const P_COMMA: f32 = 0.08;
/// Probability a sentence ends its paragraph (Return instead of Space).
const P_PARAGRAPH_END: f32 = 0.15;
/// Session clock start, seconds since boot.
const START_SECS: f64 = 1000.0;

/// Bundle id of chat-flavoured sessions.
pub const BUNDLE_CHAT: &str = "synthetic.chat";
/// Bundle id of terminal-flavoured sessions.
pub const BUNDLE_TERMINAL: &str = "synthetic.terminal";
/// Bundle id of IDE-flavoured sessions.
pub const BUNDLE_IDE: &str = "synthetic.ide";
/// Probability a slot in a terminal session is a shell command line.
const P_TERMINAL_LINE: f32 = 0.35;
/// Probability a slot in an IDE session is a code or comment line.
const P_IDE_LINE: f32 = 0.55;

/// How languages mix within a session.
///
/// [`MixProfile::default`] is the realistic profile (language runs, fixed
/// phrases, Zipf-like frequent words, app flavours); [`MixProfile::legacy`]
/// reproduces the original independent per-word swaps for reproducibility.
#[derive(Debug, Clone, PartialEq)]
pub struct MixProfile {
    /// Probability a sentence is mainly Vietnamese.
    pub p_vi_sentence: f32,
    /// In a Vietnamese sentence, probability each word is swapped for English.
    pub p_en_in_vi: f32,
    /// In an English sentence, probability each word is swapped for Vietnamese.
    pub p_vi_in_en: f32,
    /// Fraction of English insertions drawn from `Corpus::en_ambiguous`.
    pub p_ambiguous: f32,
    /// Per-letter probability of typing a wrong letter, then Backspace, then
    /// the right letter (targets `Append`, `Pop`, `Append`). 0 disables.
    pub p_typo: f32,
    /// Per-word-gap probability of a Cmd+key shortcut between words (target
    /// `Pass`). 0 disables.
    pub p_shortcut: f32,
    /// Probability a Vietnamese word is typed in a random alternative key
    /// order ([`nimk_telex::encode_variants`]: tone after the vowel, late
    /// marks such as `theme` for thêm, lone `w`, late `d`) instead of the
    /// canonical one. The RNG is only consulted for words that have a
    /// variant. 0 disables.
    pub p_variant: f32,
    /// Mean length (words) of a Vietnamese run. `> 0` switches the
    /// generator to the run model: a two-state chain over words that carries
    /// across sentences (geometric run lengths, `p_vi_sentence` picks the
    /// first state, `p_en_in_vi` / `p_vi_in_en` are ignored). 0 keeps the
    /// legacy independent swaps.
    pub mean_vi_run: f32,
    /// Mean length (words) of an English run (run model only).
    pub mean_en_run: f32,
    /// Run model: probability a unit starts a fixed phrase (when one fits
    /// the current run language) instead of a single word.
    pub p_phrase: f32,
    /// Probability a word is drawn Zipf-like from the frequent-word list
    /// (`Corpus::en_freq` / `vi_freq`) instead of uniformly. 0 disables.
    pub p_freq: f32,
    /// Probability a session is terminal-flavoured (commands, flags, paths
    /// between chat sentences, bundle [`BUNDLE_TERMINAL`]).
    pub p_terminal: f32,
    /// Probability a session is IDE-flavoured (identifiers and comments,
    /// bundle [`BUNDLE_IDE`]); the rest are chat ([`BUNDLE_CHAT`]).
    pub p_ide: f32,
}

impl MixProfile {
    /// Realistic profile, calibrated to the first real-log measurement
    /// (about 1.3k words: 70% Vietnamese, runs 4.4 / 1.9 words, 69% adjacent
    /// words in the same language; see docs/research/real-log-stats.md). The
    /// chain means (5.0 / 1.5) are set so the realized stats, which phrases
    /// and flavours stretch, land on those numbers.
    #[must_use]
    pub fn realistic() -> Self {
        Self {
            p_vi_sentence: 0.7,
            p_en_in_vi: 0.15,
            p_vi_in_en: 0.05,
            p_ambiguous: 0.4,
            p_typo: 0.0,
            p_shortcut: 0.0,
            p_variant: 0.5,
            mean_vi_run: 5.0,
            mean_en_run: 1.5,
            p_phrase: 0.15,
            p_freq: 0.5,
            p_terminal: 0.3,
            p_ide: 0.1,
        }
    }

    /// The pre-realism profile: per-sentence main language with independent
    /// word swaps, uniform sampling, one app. Same seed gives the same
    /// scripts as before runs, phrases and flavours existed.
    #[must_use]
    pub fn legacy() -> Self {
        Self {
            mean_vi_run: 0.0,
            mean_en_run: 0.0,
            p_phrase: 0.0,
            p_freq: 0.0,
            p_terminal: 0.0,
            p_ide: 0.0,
            ..Self::realistic()
        }
    }
}

impl Default for MixProfile {
    fn default() -> Self {
        Self::realistic()
    }
}

/// App flavour of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavor {
    Chat,
    Terminal,
    Ide,
}

/// Generator settings. Same config + seed = same output.
#[derive(Debug, Clone, PartialEq)]
pub struct SynthConfig {
    /// RNG seed.
    pub seed: u64,
    /// Number of sessions.
    pub sessions: usize,
    /// Words per session.
    pub words_per_session: usize,
    /// Language mixing.
    pub mix: MixProfile,
    /// Mean seconds between keys (timestamps are synthetic but plausible).
    pub key_interval_secs: f64,
}

impl Default for SynthConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            sessions: 100,
            words_per_session: 500,
            mix: MixProfile::default(),
            key_interval_secs: 0.12,
        }
    }
}

pub use nimk_tune::{Script, ScriptWord};

/// Generates `cfg.sessions` scripts deterministically.
///
/// Each session is a run of sentences. A sentence is mainly Vietnamese with
/// probability `p_vi_sentence`, and each word is swapped to the other
/// language with `p_en_in_vi` / `p_vi_in_en`. Every English word (base or
/// inserted) is drawn from `en_ambiguous` with probability `p_ambiguous`.
/// Sentences start capitalized and end with `.`, `?` or `!`; words are
/// separated by Space (sometimes `,` + Space); a paragraph, and always the
/// session, ends with Return. Vietnamese words are typed as
/// [`nimk_telex::encode`] keys. Empty corpus lists fall back to the other
/// language; a fully empty corpus yields sessions without words.
///
/// `targets[i]` labels `keys[i]`: letters inside a word are `Append`
/// (render = word language); the boundary key ending a word is `Commit`
/// (render = that word's language); boundary keys with an empty buffer (the
/// space after a comma or sentence punctuation, Return) and shortcuts are
/// `Pass` (render = previous word's language, sticky). With `p_typo > 0` a
/// wrong letter (`Append`) is followed by Backspace (`Pop`) and the right
/// letter. With `p_typo == 0` and `p_shortcut == 0` no extra randomness is
/// consumed, so keys are identical to earlier versions for the same seed.
#[must_use]
pub fn generate(cfg: &SynthConfig, corpus: &Corpus) -> Vec<Script> {
    (0..cfg.sessions).map(|n| session(cfg, corpus, n)).collect()
}

/// A hand-written script: `words` are `(language, text, raw keys)`, typed
/// as one sentence with Space after each word (the last one included, so
/// every word gets its `Commit` key). For probes and focused evaluations.
#[must_use]
pub fn script_of(words: &[(Lang, &str, &str)]) -> Script {
    let mut w = Writer {
        rng: Rng::new(0),
        t: START_SECS,
        interval: 0.12,
        keys: Vec::new(),
        words: Vec::new(),
        targets: Vec::new(),
        last_lang: Lang::Vi,
        track: Tracker::new(),
        state: Lang::Vi,
        limit: usize::MAX,
    };
    for &(lang, text, raw) in words {
        w.type_word(raw, lang, 0.0);
        w.words.push(ScriptWord {
            word_id: w.words.len() as u64,
            lang,
            text: text.to_owned(),
            raw: raw.to_owned(),
        });
        w.key(' ', 0);
    }
    Script {
        session: "synth-script".to_owned(),
        bundle_id: SYNTHETIC_BUNDLE_ID.to_owned(),
        keys: w.keys,
        words: w.words,
        targets: w.targets,
    }
}

/// Builds session `n` with its own RNG stream.
fn session(cfg: &SynthConfig, corpus: &Corpus, n: usize) -> Script {
    let stream = (n as u64)
        .wrapping_add(1)
        .wrapping_mul(0xD1B5_4A32_D192_ED03);
    let mut w = Writer {
        rng: Rng::new(cfg.seed ^ stream),
        t: START_SECS,
        interval: cfg.key_interval_secs,
        keys: Vec::new(),
        words: Vec::with_capacity(cfg.words_per_session),
        targets: Vec::new(),
        last_lang: Lang::Vi,
        track: Tracker::new(),
        state: Lang::Vi,
        limit: cfg.words_per_session,
    };
    let mix = &cfg.mix;
    let flavor = if mix.p_terminal + mix.p_ide > 0.0 {
        let r = w.rng.unit() as f32;
        if r < mix.p_terminal {
            Flavor::Terminal
        } else if r < mix.p_terminal + mix.p_ide {
            Flavor::Ide
        } else {
            Flavor::Chat
        }
    } else {
        Flavor::Chat
    };
    if mix.mean_vi_run > 0.0 {
        w.state = if w.rng.chance(mix.p_vi_sentence) {
            Lang::Vi
        } else {
            Lang::En
        };
    }
    let has_words =
        !(corpus.vi.is_empty() && corpus.en.is_empty() && corpus.en_ambiguous.is_empty());
    // Line flavours need their vocabulary; without it a session is chat.
    let flavor = if corpus.cmds.is_empty() || corpus.en.is_empty() {
        Flavor::Chat
    } else {
        flavor
    };
    while has_words && w.words.len() < cfg.words_per_session {
        let remaining = cfg.words_per_session - w.words.len();
        let span = SENTENCE_WORDS.1 - SENTENCE_WORDS.0 + 1;
        let len = (SENTENCE_WORDS.0 + w.rng.below(span)).min(remaining);
        let last_sentence = len == remaining;
        let line = match flavor {
            Flavor::Chat => false,
            Flavor::Terminal => w.rng.chance(P_TERMINAL_LINE),
            Flavor::Ide => w.rng.chance(P_IDE_LINE),
        };
        if line {
            w.line(flavor, mix, corpus);
        } else {
            w.sentence(mix, corpus, len, last_sentence);
        }
    }
    let bundle_id = match flavor {
        _ if mix.p_terminal + mix.p_ide == 0.0 => SYNTHETIC_BUNDLE_ID,
        Flavor::Chat => BUNDLE_CHAT,
        Flavor::Terminal => BUNDLE_TERMINAL,
        Flavor::Ide => BUNDLE_IDE,
    };
    Script {
        session: format!("synth-{}-{n}", cfg.seed),
        bundle_id: bundle_id.to_owned(),
        keys: w.keys,
        words: w.words,
        targets: w.targets,
    }
}

/// Accumulates one session's keys and words.
struct Writer {
    rng: Rng,
    t: f64,
    interval: f64,
    keys: Vec<KeyEvent>,
    words: Vec<ScriptWord>,
    targets: Vec<Action>,
    /// Language of the most recent word (sticky render for `Pass`).
    last_lang: Lang,
    /// The engine's state machine (ADR 0012), advanced by every key.
    track: Tracker,
    /// Current run language (run model).
    state: Lang,
    /// Words per session; line flavours stop emitting words at this count.
    limit: usize,
}

impl Writer {
    fn sentence(&mut self, mix: &MixProfile, corpus: &Corpus, len: usize, last: bool) {
        if mix.mean_vi_run > 0.0 {
            return self.sentence_runs(mix, corpus, len, last);
        }
        let main = if self.rng.chance(mix.p_vi_sentence) {
            Lang::Vi
        } else {
            Lang::En
        };
        let p_swap = match main {
            Lang::Vi => mix.p_en_in_vi,
            Lang::En => mix.p_vi_in_en,
        };
        for i in 0..len {
            let lang = if self.rng.chance(p_swap) {
                main.flip()
            } else {
                main
            };
            let Some((lang, word)) = self.pick(lang, mix, corpus) else {
                return;
            };
            self.put(mix, lang, word, (i, len), last, true);
        }
    }

    /// Run model: words and phrases follow a two-state chain whose state
    /// persists across sentences.
    fn sentence_runs(&mut self, mix: &MixProfile, corpus: &Corpus, len: usize, last: bool) {
        let mut i = 0;
        while i < len {
            let mean = match self.state {
                Lang::Vi => mix.mean_vi_run,
                Lang::En => mix.mean_en_run,
            };
            if self.rng.chance(1.0 / mean.max(1.0)) {
                self.state = self.state.flip();
            }
            let phrase = if mix.p_phrase > 0.0 && self.rng.chance(mix.p_phrase) {
                self.pick_phrase(corpus)
            } else {
                None
            };
            if let Some(tokens) = phrase {
                for (lang, word) in tokens.into_iter().take(len - i) {
                    self.state = lang;
                    self.put(mix, lang, word, (i, len), last, false);
                    i += 1;
                }
            } else {
                let Some((lang, word)) = self.pick(self.state, mix, corpus) else {
                    return;
                };
                self.state = lang;
                self.put(mix, lang, word, (i, len), last, true);
                i += 1;
            }
        }
    }

    /// A random phrase starting in the current run language.
    fn pick_phrase(&mut self, corpus: &Corpus) -> Option<Vec<(Lang, String)>> {
        let fits: Vec<&Vec<(Lang, String)>> = corpus
            .phrases
            .iter()
            .filter(|p| p.first().is_some_and(|t| t.0 == self.state))
            .collect();
        if fits.is_empty() {
            return None;
        }
        fits.get(self.rng.below(fits.len())).map(|p| (*p).clone())
    }

    /// Types word `idx.0` of a sentence of `idx.1` words, then its separator
    /// (or the sentence end after the last one).
    fn put(
        &mut self,
        mix: &MixProfile,
        lang: Lang,
        word: String,
        idx: (usize, usize),
        last: bool,
        comma_ok: bool,
    ) {
        let (i, len) = idx;
        let text = if i == 0 { capitalize(&word) } else { word };
        let raw = match lang {
            Lang::Vi => self.encode_vi(&text, mix.p_variant),
            Lang::En => text.clone(),
        };
        self.type_word(&raw, lang, mix.p_typo);
        self.words.push(ScriptWord {
            word_id: self.words.len() as u64,
            lang,
            text,
            raw,
        });
        if i + 1 < len {
            if comma_ok && self.rng.chance(P_COMMA) {
                self.key(',', 0);
                self.key(' ', 0);
            } else {
                self.key(' ', 0);
            }
            if mix.p_shortcut > 0.0 && self.rng.chance(mix.p_shortcut) {
                self.shortcut();
            }
        } else {
            self.end_sentence(last);
        }
    }

    fn end_sentence(&mut self, last: bool) {
        let r = self.rng.unit();
        let punct = if r < 0.8 {
            '.'
        } else if r < 0.92 {
            '?'
        } else {
            '!'
        };
        self.key(punct, 0);
        let paragraph = last || self.rng.chance(P_PARAGRAPH_END);
        self.key(if paragraph { '\r' } else { ' ' }, 0);
        // Thinking pause before the next sentence.
        self.t += self.interval * 3.0 * self.rng.unit();
    }

    /// Telex keys for a Vietnamese word: the canonical form, or with
    /// probability `p_variant` a random variant when it has any.
    fn encode_vi(&mut self, text: &str, p_variant: f32) -> String {
        let variants = nimk_telex::encode_variants(text);
        if !variants.is_empty() && self.rng.chance(p_variant) {
            let i = self.rng.below(variants.len());
            if let Some(v) = variants.get(i) {
                return v.clone();
            }
        }
        nimk_telex::encode(text)
    }

    /// Picks a word in `lang`, falling back to the other language if empty.
    fn pick(&mut self, lang: Lang, mix: &MixProfile, corpus: &Corpus) -> Option<(Lang, String)> {
        let first = self.pick_in(lang, mix, corpus);
        first.or_else(|| self.pick_in(lang.flip(), mix, corpus))
    }

    fn pick_in(&mut self, lang: Lang, mix: &MixProfile, corpus: &Corpus) -> Option<(Lang, String)> {
        if mix.p_freq > 0.0 {
            let freq = match lang {
                Lang::Vi => &corpus.vi_freq,
                Lang::En => &corpus.en_freq,
            };
            if !freq.is_empty() && self.rng.chance(mix.p_freq) {
                let i = self.zipf(freq.len());
                return freq.get(i).map(|w| (lang, w.clone()));
            }
        }
        let list = match lang {
            Lang::Vi => &corpus.vi,
            Lang::En if corpus.en.is_empty() => &corpus.en_ambiguous,
            Lang::En if corpus.en_ambiguous.is_empty() => &corpus.en,
            Lang::En if self.rng.chance(mix.p_ambiguous) => &corpus.en_ambiguous,
            Lang::En => &corpus.en,
        };
        if list.is_empty() {
            return None;
        }
        let i = self.rng.below(list.len());
        list.get(i).map(|w| (lang, w.clone()))
    }

    /// Index in `0..n` with probability proportional to `1 / (rank + 1)`.
    fn zipf(&mut self, n: usize) -> usize {
        let total: f64 = (1..=n).map(|r| 1.0 / r as f64).sum();
        let mut u = self.rng.unit() * total;
        for r in 0..n {
            u -= 1.0 / (r + 1) as f64;
            if u < 0.0 {
                return r;
            }
        }
        n.saturating_sub(1)
    }

    /// Types a word's letters as `Append`, with optional typo + Backspace.
    fn type_word(&mut self, raw: &str, lang: Lang, p_typo: f32) {
        if !self.track.is_raw() {
            self.last_lang = lang;
        }
        for c in raw.chars() {
            if p_typo > 0.0 && !self.track.is_raw() && self.rng.chance(p_typo) {
                let wrong = self.wrong_letter(c);
                self.key(wrong, 0);
                self.backspace();
            }
            self.key(c, 0);
        }
    }

    /// A lowercase letter different from `c` (case-insensitive).
    fn wrong_letter(&mut self, c: char) -> char {
        let i = self.rng.below(26) as u8;
        let mut w = char::from(b'a' + i);
        if w == c.to_ascii_lowercase() {
            w = char::from(b'a' + (i + 1) % 26);
        }
        w
    }

    fn backspace(&mut self) {
        self.advance();
        self.push(
            keycode::BACKSPACE,
            "\u{7f}".to_owned(),
            Modifiers::default(),
        );
    }

    /// A Cmd+key app shortcut (never edits the buffer).
    fn shortcut(&mut self) {
        let c = char::from(b"cvazx"[self.rng.below(5)]);
        self.key(c, Modifiers::COMMAND);
    }

    fn advance(&mut self) {
        self.t += self.interval * (0.5 + self.rng.unit());
    }

    /// Emits one `KeyDown` for `c` with extra modifier bits. The target
    /// edit comes from the engine state machine.
    fn key(&mut self, c: char, extra_mods: u32) {
        self.advance();
        let (keycode, shift) = key_for(c);
        let bits = extra_mods | if shift { Modifiers::SHIFT } else { 0 };
        self.push(keycode, c.to_string(), Modifiers(bits));
    }

    fn push(&mut self, keycode: u16, chars: String, mods: Modifiers) {
        let event = KeyEvent {
            kind: KeyKind::KeyDown,
            keycode,
            chars,
            mods,
            timestamp: self.t,
        };
        let step: Step = classify(&event).map_or(Step::Pass, |c| self.track.step(c));
        self.keys.push(event);
        self.targets.push(Action {
            edit: step.edit(),
            render: self.last_lang,
        });
    }
}

/// Uppercases the first character.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

mod flavor;

#[cfg(test)]
#[path = "generate_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "targets_tests.rs"]
mod targets_tests;
