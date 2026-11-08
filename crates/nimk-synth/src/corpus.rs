//! Built-in word lists (`data/*.txt`, one word per line, `#` comments).

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
}

const VI: &str = include_str!("../data/vi.txt");
const EN: &str = include_str!("../data/en.txt");
const EN_AMBIGUOUS: &str = include_str!("../data/en_ambiguous.txt");

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
        }
    }
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
}
