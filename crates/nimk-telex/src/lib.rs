//! Telex transformation and Vietnamese syllable validation.
//!
//! Pure functions, no state: the composer keeps the raw keys and calls
//! [`transform`] after every key.
//!
//! The Telex engine is implemented here (no `vi-rs`) so that [`encode`] is
//! an exact inverse and the rules below are fixed by tests:
//!
//! | Keys | Effect |
//! | --- | --- |
//! | `s f r x j` | sắc, huyền, hỏi, ngã, nặng (needs a vowel before it) |
//! | `z` | removes the tone |
//! | `aa ee oo` | `â ê ô` (on the rightmost vowel, across a final: `tieng`+`e`) |
//! | `aw ow uw`, `uow` | `ă ơ ư`, `ươ` (`quow` -> `quơ`) |
//! | `dd` | `đ`, word-initial only |
//!
//! Repeating a transform key that is in effect undoes it and types the key
//! (`ass` -> `as`, `aaa` -> `aa`); that transform is then off for the word.
//! A lone `w` is a literal `w` (no `w` -> `ư`).
//!
//! Tones use "new style" placement: `hoà`, `hoè`, `thuý` (second vowel of
//! open `oa oă oe ue uy`), `múa`, `tài` (first vowel of other open pairs),
//! a marked vowel always wins (`đường`), and with a final consonant the
//! last vowel (`toán`).
#![forbid(unsafe_code)]

mod letters;
mod syllable;
mod telex;

use letters::{Mark, Tone, mark_of, plain, split_tone, to_lower};

/// Result of interpreting a raw key sequence as Telex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transformed {
    /// Telex rendering, e.g. `tieengs` -> `tiếng`. Equals the input if no
    /// transform applies.
    pub text: String,
    /// Whether any Telex transform key took effect (tone, `aa`, `dd`, `w`...).
    /// Defined as `text != raw`, so `false` means both renderings are equal.
    pub has_telex: bool,
    /// Whether `text` is a complete, valid Vietnamese syllable (case-insensitive).
    pub valid_vi: bool,
    /// Whether `text` is a valid syllable or a prefix that further typing
    /// could complete into one (see [`is_viable_prefix`]). Use this mid-word
    /// and `valid_vi` at commit.
    pub viable_vi: bool,
}

/// Interprets one word's raw keys as Telex.
///
/// Case is preserved per letter (`Vieejt` -> `Việt`). Input is a single word:
/// no spaces or punctuation.
///
/// # Examples
///
/// ```
/// let t = nimk_telex::transform("dduwowngf");
/// assert_eq!(t.text, "đường");
/// assert!(t.has_telex && t.valid_vi);
/// ```
#[must_use]
pub fn transform(raw: &str) -> Transformed {
    let mut composer = telex::Composer::default();
    for key in raw.chars() {
        composer.push(key);
    }
    let text = composer.render();
    let has_telex = text != raw;
    let valid_vi = is_valid_syllable(&text);
    let viable_vi = valid_vi || is_viable_prefix(&text);
    Transformed {
        text,
        has_telex,
        valid_vi,
        viable_vi,
    }
}

/// Inverse of [`transform`]: the canonical Telex keys that type `word`
/// (tone key last, e.g. `tiếng` -> `tieengs`, `đường` -> `dduwowngf`).
/// Words without diacritics are returned unchanged. Invariant (tested):
/// `transform(&encode(w)).text == w` for every valid syllable `w`.
///
/// Mark keys and the tone key are lowercase; letters keep their case.
/// A literal double vowel is typed with three keys (`xoong` -> `xooong`).
/// Exception: `uơ` without `q` (`thuở`) has no Telex spelling here, since
/// `uow` always gives `ươ`.
#[must_use]
pub fn encode(word: &str) -> String {
    let mut out = String::with_capacity(word.len() + 4);
    let mut tone = Tone::Level;
    let mut prev: Option<char> = None;
    for c in word.chars() {
        let upper = c.is_uppercase();
        let (base, t) = split_tone(to_lower(c));
        if t != Tone::Level {
            tone = t;
        }
        let p = plain(base);
        let typed = if upper { letters::to_upper(p) } else { p };
        out.push(typed);
        match mark_of(base) {
            Some(Mark::Hat) => out.push(p),
            Some(Mark::Breve | Mark::Horn) => out.push('w'),
            Some(Mark::Stroke) => out.push('d'),
            None if matches!(p, 'a' | 'e' | 'o') && prev == Some(p) => out.push(typed),
            None => {}
        }
        prev = mark_of(base).is_none().then_some(p);
    }
    if let Some(k) = tone.key() {
        out.push(k);
    }
    out
}

/// Returns true if `word` (already rendered, with diacritics) is a valid
/// Vietnamese syllable: optional initial consonant, a valid rhyme, and a tone
/// compatible with the final consonant (`c ch p t` take only sắc or nặng).
///
/// Tone position is not checked, so both `hoà` and `hòa` are valid.
#[must_use]
pub fn is_valid_syllable(word: &str) -> bool {
    let mut bases = Vec::with_capacity(word.len());
    let mut tone = Tone::Level;
    for c in word.chars() {
        let (base, t) = split_tone(to_lower(c));
        if t != Tone::Level {
            if tone != Tone::Level {
                return false;
            }
            tone = t;
        }
        bases.push(base);
    }
    syllable::is_valid(&bases, tone)
}

/// Returns true if `word` is a prefix of some valid Vietnamese syllable
/// (case-insensitive), i.e. further typing could still make it valid.
///
/// Tones are ignored and letters are compared without marks, since Telex
/// can add both later (`vie` -> `viê`, `tieng` -> `tiếng`). Complete valid
/// syllables and the empty string are viable.
///
/// # Examples
///
/// ```
/// assert!(nimk_telex::is_viable_prefix("Viêt"));
/// assert!(!nimk_telex::is_viable_prefix("str"));
/// ```
#[must_use]
pub fn is_viable_prefix(word: &str) -> bool {
    let bases: Vec<char> = word.chars().map(|c| split_tone(to_lower(c)).0).collect();
    syllable::is_viable(&bases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn check(raw: &str, text: &str, has_telex: bool, valid_vi: bool) {
        let t = transform(raw);
        assert_eq!(
            (t.text.as_str(), t.has_telex, t.valid_vi),
            (text, has_telex, valid_vi),
            "raw {raw}"
        );
        if t.valid_vi {
            assert!(t.viable_vi, "raw {raw}: valid implies viable");
        }
    }

    #[test]
    fn viability() {
        for w in ["str", "cl", "fo", "wh", "ing", "tyê", "ce"] {
            assert!(!is_viable_prefix(w), "{w}");
        }
        for w in [
            "", "q", "ngh", "vie", "viê", "Viêt", "tieng", "gi", "khuy", "đươ", "xoo",
        ] {
            assert!(is_viable_prefix(w), "{w}");
        }
        let has = transform("has");
        assert!(has.valid_vi && has.viable_vi);
        for raw in ["viee", "Vieet"] {
            let t = transform(raw);
            assert!(t.viable_vi && !t.valid_vi, "{raw} -> {}", t.text);
        }
        assert!(!transform("string").viable_vi);
    }

    #[test]
    fn table() {
        check("tieengs", "tiếng", true, true);
        check("tiengse", "tiếng", true, true);
        check("Vieejt", "Việt", true, true);
        check("dduwowngf", "đường", true, true);
        check("duongwf", "dường", true, true);
        check("has", "há", true, true);
        check("for", "fỏ", true, false);
        check("user", "uẻ", true, false);
        check("string", "string", false, false);
        check("class", "clas", true, false);
        check("test", "tét", true, true);
        check("mod", "mod", false, false);
        check("hoaf", "hoà", true, true);
        check("muas", "múa", true, true);
        check("quows", "quớ", true, true);
        check("HAS", "HÁ", true, true);
        check("DDuwowngf", "Đường", true, true);
        check("an", "an", false, true);
    }

    #[test]
    fn undo_and_tone_changes() {
        check("ass", "as", true, false);
        check("asss", "ass", true, false);
        check("aaa", "aa", true, false);
        check("ddd", "dd", true, false);
        check("uww", "uw", true, false);
        check("asf", "à", true, true);
        check("asz", "a", true, true);
        check("z", "z", false, false);
        check("add", "add", false, false);
        check("w", "w", false, false);
    }

    #[test]
    fn encode_examples() {
        assert_eq!(encode("tiếng"), "tieengs");
        assert_eq!(encode("đường"), "dduwowngf");
        assert_eq!(encode("Việt"), "Vieetj");
        assert_eq!(encode("xoong"), "xooong");
        assert_eq!(encode("deploy"), "deploy");
        assert_eq!(transform(&encode("Đường")).text, "Đường");
    }

    #[test]
    fn syllable_validity() {
        assert!(is_valid_syllable("Tiếng"));
        assert!(is_valid_syllable("hòa") && is_valid_syllable("hoà"));
        assert!(!is_valid_syllable("táà"));
        assert!(!is_valid_syllable("tàt"));
        assert!(!is_valid_syllable(""));
    }
}
