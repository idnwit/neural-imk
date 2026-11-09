//! Built-in word lists (`data/*.txt`, one word per line, `#` comments).

use nimk_types::Lang;

/// Word lists the generator samples from. Built-in lists are hand-written
/// for this project (no third-party corpus).
#[derive(Debug, Clone)]
pub struct Corpus {
    /// Common Vietnamese words/syllables with diacritics (`tiếng`, `được`).
    pub vi: Vec<String>,
    /// Common English words, including tech vocabulary (`deploy`, `commit`).
    pub en: Vec<String>,
    /// English words whose raw keys are also valid Telex (`has` -> `há`,
    /// `for` -> `fỏ`, `user` -> `uẻ`); over-sampled because they are the hard cases.
    pub en_ambiguous: Vec<String>,
    /// Frequent English words, most frequent first; sampled Zipf-like.
    /// Each is also in `en` or `en_ambiguous`.
    pub en_freq: Vec<String>,
    /// Frequent Vietnamese words, most frequent first; each is also in `vi`.
    pub vi_freq: Vec<String>,
    /// Fixed phrases typed in runs; each is a non-empty list of
    /// `(language, word)`. Vietnamese tokens are valid syllables, English
    /// tokens lowercase ASCII.
    pub phrases: Vec<Vec<(Lang, String)>>,
    /// Terminal commands (`git`, `cargo`).
    pub cmds: Vec<String>,
    /// Terminal subcommands (`status`, `push`).
    pub subs: Vec<String>,
    /// Terminal flags with their dashes (`-la`, `--help`).
    pub flags: Vec<String>,
    /// Path components (`src`, `usr`).
    pub dirs: Vec<String>,
    /// Slash-command names typed after `/` (`help`, `clear`).
    pub slashes: Vec<String>,
    /// File names with extension (`lib.rs`).
    pub files: Vec<String>,
}

const VI: &str = include_str!("../data/vi.txt");
const EN: &str = include_str!("../data/en.txt");
const EN_AMBIGUOUS: &str = include_str!("../data/en_ambiguous.txt");
const EN_FREQ: &str = include_str!("../data/en_freq.txt");
const VI_FREQ: &str = include_str!("../data/vi_freq.txt");
const PHRASES: &str = include_str!("../data/phrases.txt");
const COMMANDS: &str = include_str!("../data/commands.txt");

impl Corpus {
    /// Built-in corpus: >= 1500 Vietnamese, >= 1500 English and >= 150
    /// ambiguous English words.
    ///
    /// Vietnamese entries are single syllables in the tone placement of
    /// `nimk_telex` ("new style", `hoà`); `en` and `en_ambiguous` are
    /// disjoint, lowercase ASCII letters only.
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            vi: parse(VI),
            en: parse(EN),
            en_ambiguous: parse(EN_AMBIGUOUS),
            en_freq: parse(EN_FREQ),
            vi_freq: parse(VI_FREQ),
            phrases: parse(PHRASES).iter().map(|l| parse_phrase(l)).collect(),
            cmds: tagged(COMMANDS, "c:"),
            subs: tagged(COMMANDS, "s:"),
            flags: tagged(COMMANDS, "f:"),
            dirs: tagged(COMMANDS, "d:"),
            slashes: tagged(COMMANDS, "r:"),
            files: tagged(COMMANDS, "x:"),
        }
    }

    /// A corpus with no words at all.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            vi: Vec::new(),
            en: Vec::new(),
            en_ambiguous: Vec::new(),
            en_freq: Vec::new(),
            vi_freq: Vec::new(),
            phrases: Vec::new(),
            cmds: Vec::new(),
            subs: Vec::new(),
            flags: Vec::new(),
            dirs: Vec::new(),
            slashes: Vec::new(),
            files: Vec::new(),
        }
    }
}

/// Parses `e:word v:syllable ...` into language-tagged tokens; unknown
/// tags are skipped.
fn parse_phrase(line: &str) -> Vec<(Lang, String)> {
    line.split_whitespace()
        .filter_map(|t| {
            if let Some(w) = t.strip_prefix("e:") {
                Some((Lang::En, w.to_owned()))
            } else {
                t.strip_prefix("v:").map(|w| (Lang::Vi, w.to_owned()))
            }
        })
        .collect()
}

/// Entries of a data file starting with `tag`, tag removed.
fn tagged(data: &str, tag: &str) -> Vec<String> {
    parse(data)
        .iter()
        .filter_map(|l| l.strip_prefix(tag).map(str::to_owned))
        .collect()
}

/// Splits a data file into words, skipping blank lines and `#` comments.
fn parse(data: &str) -> Vec<String> {
    data.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use nimk_telex::{encode, is_valid_syllable, transform};

    use super::*;

    fn unique(words: &[String]) -> bool {
        words.iter().collect::<HashSet<_>>().len() == words.len()
    }

    #[test]
    fn sizes_and_uniqueness() {
        let c = Corpus::builtin();
        assert!(c.vi.len() >= 1500, "vi {}", c.vi.len());
        assert!(c.en.len() >= 1500, "en {}", c.en.len());
        assert!(c.en_ambiguous.len() >= 150, "amb {}", c.en_ambiguous.len());
        assert!(unique(&c.vi) && unique(&c.en) && unique(&c.en_ambiguous));
        let en: HashSet<_> = c.en.iter().collect();
        assert!(
            c.en_ambiguous.iter().all(|w| !en.contains(w)),
            "en and en_ambiguous overlap"
        );
    }

    #[test]
    fn vi_words_are_valid_and_round_trip() {
        for w in &Corpus::builtin().vi {
            assert!(is_valid_syllable(w), "{w}");
            assert_eq!(&transform(&encode(w)).text, w, "{w}");
        }
    }

    #[test]
    fn en_words_are_plain_ascii() {
        let c = Corpus::builtin();
        for w in c.en.iter().chain(&c.en_ambiguous) {
            assert!(
                !w.is_empty() && w.bytes().all(|b| b.is_ascii_lowercase()),
                "{w}"
            );
        }
    }

    #[test]
    fn ambiguous_membership_matches_telex() {
        let c = Corpus::builtin();
        for w in &c.en_ambiguous {
            let t = transform(w);
            assert!(
                t.has_telex && t.valid_vi,
                "{w} -> {} is not ambiguous",
                t.text
            );
        }
        for w in &c.en {
            let t = transform(w);
            assert!(
                !(t.has_telex && t.valid_vi),
                "{w} -> {} belongs in en_ambiguous",
                t.text
            );
        }
    }

    #[test]
    fn frequency_lists_and_phrases_are_consistent() {
        let c = Corpus::builtin();
        let en: HashSet<_> = c.en.iter().chain(&c.en_ambiguous).collect();
        let vi: HashSet<_> = c.vi.iter().collect();
        assert!(c.en_freq.len() >= 50 && c.vi_freq.len() >= 50);
        assert!(unique(&c.en_freq) && unique(&c.vi_freq));
        assert!(c.en_freq.iter().all(|w| en.contains(w)));
        assert!(c.vi_freq.iter().all(|w| vi.contains(w)));
        // The frequent function words the real logs show.
        for w in [
            "to", "in", "a", "of", "the", "so", "is", "it", "on", "me", "no", "an",
        ] {
            assert!(c.en_freq.iter().any(|f| f == w), "{w}");
        }
        assert!(c.phrases.len() >= 80);
        assert!(c.phrases.iter().all(|p| !p.is_empty()));
        for (lang, w) in c.phrases.iter().flatten() {
            match lang {
                Lang::Vi => assert!(is_valid_syllable(w), "{w}"),
                Lang::En => assert!(w.bytes().all(|b| b.is_ascii_lowercase()), "{w}"),
            }
        }
        for lang in [Lang::En, Lang::Vi] {
            assert!(c.phrases.iter().any(|p| p[0].0 == lang));
        }
        assert!(c.cmds.len() >= 10 && c.subs.len() >= 10);
        assert!(c.flags.iter().all(|f| f.starts_with('-')));
        assert!(c.dirs.len() >= 5 && c.slashes.len() >= 5 && c.files.len() >= 5);
    }
}
