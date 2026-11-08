//! Code-switching sentence generator producing `KeyEvent` scripts.

use nimk_types::{KeyEvent, KeyKind, Lang, Modifiers};

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
}

impl Default for MixProfile {
    fn default() -> Self {
        Self {
            p_vi_sentence: 0.7,
            p_en_in_vi: 0.15,
            p_vi_in_en: 0.05,
            p_ambiguous: 0.4,
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
    }
}

/// Accumulates one session's keys and words.
struct Writer {
    rng: Rng,
    t: f64,
    interval: f64,
    keys: Vec<KeyEvent>,
    words: Vec<ScriptWord>,
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
            self.type_str(&raw);
            self.words.push(ScriptWord {
                word_id: self.words.len() as u64,
                lang,
                text,
                raw,
            });
            if i + 1 < len {
                if self.rng.chance(P_COMMA) {
                    self.type_str(",");
                }
                self.type_str(" ");
            } else {
                self.end_sentence(last);
            }
        }
    }

    fn end_sentence(&mut self, last: bool) {
        let r = self.rng.unit();
        let punct = if r < 0.8 {
            "."
        } else if r < 0.92 {
            "?"
        } else {
            "!"
        };
        self.type_str(punct);
        let paragraph = last || self.rng.chance(P_PARAGRAPH_END);
        self.type_str(if paragraph { "\r" } else { " " });
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

    /// Emits one `KeyDown` per character, advancing the clock.
    fn type_str(&mut self, s: &str) {
        for c in s.chars() {
            self.t += self.interval * (0.5 + self.rng.unit());
            let (keycode, shift) = key_for(c);
            let mods = if shift {
                Modifiers(Modifiers::SHIFT)
            } else {
                Modifiers::default()
            };
            self.keys.push(KeyEvent {
                kind: KeyKind::KeyDown,
                keycode,
                chars: c.to_string(),
                mods,
                timestamp: self.t,
            });
        }
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
