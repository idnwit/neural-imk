//! Code-switching sentence generator producing `KeyEvent` scripts.

use nimk_types::{Action, EditAction, KeyEvent, KeyKind, Lang, Modifiers, keycode};

use crate::Corpus;
use crate::keymap::{Rng, key_for};

/// Sentence length range (words), inclusive.
const SENTENCE_WORDS: (usize, usize) = (4, 12);
/// Probability of a comma after a non-final word.
const P_COMMA: f32 = 0.08;
/// Probability a sentence ends its paragraph (Return instead of Space).
const P_PARAGRAPH_END: f32 = 0.15;
/// Session clock start, seconds since boot.
const START_SECS: f64 = 1000.0;

/// How languages mix within a session.
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
}

impl Default for MixProfile {
    fn default() -> Self {
        Self {
            p_vi_sentence: 0.7,
            p_en_in_vi: 0.15,
            p_vi_in_en: 0.05,
            p_ambiguous: 0.4,
            p_typo: 0.0,
            p_shortcut: 0.0,
        }
    }
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

/// One word of a script.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptWord {
    /// Word id the engine will assign (0, 1, 2... per session, one per
    /// committed non-empty word).
    pub word_id: u64,
    /// Intended language.
    pub lang: Lang,
    /// Text the user intends to see (`tiếng` or `has`).
    pub text: String,
    /// Raw keys typed for it (`tieengs` or `has`).
    pub raw: String,
}

/// One synthetic typing session.
#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    /// Session id (`synth-<seed>-<n>`).
    pub session: String,
    /// Every key in order, including word boundaries (space, punctuation).
    /// Contains no hotkeys.
    pub keys: Vec<KeyEvent>,
    /// Words in order with their labels.
    pub words: Vec<ScriptWord>,
    /// Target [`Action`] for every key, same index as `keys`.
    pub targets: Vec<Action>,
}

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
    };
    let has_words =
        !(corpus.vi.is_empty() && corpus.en.is_empty() && corpus.en_ambiguous.is_empty());
    while has_words && w.words.len() < cfg.words_per_session {
        let remaining = cfg.words_per_session - w.words.len();
        let span = SENTENCE_WORDS.1 - SENTENCE_WORDS.0 + 1;
        let len = (SENTENCE_WORDS.0 + w.rng.below(span)).min(remaining);
        let last_sentence = len == remaining;
        w.sentence(&cfg.mix, corpus, len, last_sentence);
    }
    Script {
        session: format!("synth-{}-{n}", cfg.seed),
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
}

impl Writer {
    fn sentence(&mut self, mix: &MixProfile, corpus: &Corpus, len: usize, last: bool) {
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
            let text = if i == 0 { capitalize(&word) } else { word };
            let raw = match lang {
                Lang::Vi => nimk_telex::encode(&text),
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
                if self.rng.chance(P_COMMA) {
                    self.key(',', 0, EditAction::Commit);
                    self.key(' ', 0, EditAction::Pass);
                } else {
                    self.key(' ', 0, EditAction::Commit);
                }
                if mix.p_shortcut > 0.0 && self.rng.chance(mix.p_shortcut) {
                    self.shortcut();
                }
            } else {
                self.end_sentence(last);
            }
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
        self.key(punct, 0, EditAction::Commit);
        let paragraph = last || self.rng.chance(P_PARAGRAPH_END);
        self.key(if paragraph { '\r' } else { ' ' }, 0, EditAction::Pass);
        // Thinking pause before the next sentence.
        self.t += self.interval * 3.0 * self.rng.unit();
    }

    /// Picks a word in `lang`, falling back to the other language if empty.
    fn pick(&mut self, lang: Lang, mix: &MixProfile, corpus: &Corpus) -> Option<(Lang, String)> {
        let first = self.pick_in(lang, mix, corpus);
        first.or_else(|| self.pick_in(lang.flip(), mix, corpus))
    }

    fn pick_in(&mut self, lang: Lang, mix: &MixProfile, corpus: &Corpus) -> Option<(Lang, String)> {
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

    /// Types a word's letters as `Append`, with optional typo + Backspace.
    fn type_word(&mut self, raw: &str, lang: Lang, p_typo: f32) {
        self.last_lang = lang;
        for c in raw.chars() {
            if p_typo > 0.0 && self.rng.chance(p_typo) {
                let wrong = self.wrong_letter(c);
                self.key(wrong, 0, EditAction::Append);
                self.backspace();
            }
            self.key(c, 0, EditAction::Append);
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
            EditAction::Pop,
        );
    }

    /// A Cmd+key app shortcut (never edits the buffer).
    fn shortcut(&mut self) {
        let c = char::from(b"cvazx"[self.rng.below(5)]);
        self.key(c, Modifiers::COMMAND, EditAction::Pass);
    }

    fn advance(&mut self) {
        self.t += self.interval * (0.5 + self.rng.unit());
    }

    /// Emits one `KeyDown` for `c` with extra modifier bits.
    fn key(&mut self, c: char, extra_mods: u32, edit: EditAction) {
        self.advance();
        let (keycode, shift) = key_for(c);
        let bits = extra_mods | if shift { Modifiers::SHIFT } else { 0 };
        self.push(keycode, c.to_string(), Modifiers(bits), edit);
    }

    fn push(&mut self, keycode: u16, chars: String, mods: Modifiers, edit: EditAction) {
        self.keys.push(KeyEvent {
            kind: KeyKind::KeyDown,
            keycode,
            chars,
            mods,
            timestamp: self.t,
        });
        self.targets.push(Action {
            edit,
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

#[cfg(test)]
#[path = "generate_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "targets_tests.rs"]
mod targets_tests;
