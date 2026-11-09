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
//!
//! A `w` that modifies no preceding `a o u` and comes before any vowel of
//! the word (word start, or after the initial consonants) is `ư`: `w` -> `ư`,
//! `nhw` -> `như`, `dwowngf` -> `dường`, `twowng` -> `tương`. `ww` -> `w`
//! (and the transform is off for the word). After a vowel that cannot take
//! a horn (`new`, `show`'s `h`...) `w` stays literal; `when` -> `ưhen`
//! (not viable, so it is treated as English).
//!
//! Tones use modern placement by default: `hoà`, `hoè`, `thuý` (second vowel
//! of open `oa oă oe ue uy`), `múa`, `tài` (first vowel of other open
//! pairs), a marked vowel always wins (`đường`), and with a final consonant
//! the last vowel (`toán`). [`ToneStyle::Old`] gives `hòa`, `thủy`.
//!
//! v2 (docs/10-telex.md): marks and `dd` may come late (`theme` -> `thêm`,
//! `dungd` -> `đung`), `ưo` completes to `ươ` (`tuwong`), a mark is applied
//! only when the result is a viable syllable, and an undo locks the word as
//! raw (`assf` -> `asf`).
#![forbid(unsafe_code)]

mod encode;
mod letters;
mod syllable;

pub use syllable::ToneStyle;
mod telex;

use letters::{Tone, split_tone, to_lower};

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
    transform_with(raw, ToneStyle::Modern)
}

/// [`transform`] with an explicit tone placement style (`hoà` or `hòa`).
#[must_use]
pub fn transform_with(raw: &str, style: ToneStyle) -> Transformed {
    let mut composer = telex::Composer::new(style);
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
/// (marks right after their letter, tone key last, e.g. `tiếng` ->
/// `tieengs`, `đường` -> `dduwowngf`). Words without diacritics are returned
/// unchanged. Invariant (tested): `transform(&encode(w)).text == w` for every
/// valid syllable `w`.
///
/// Mark keys and the tone key are lowercase; letters keep their case.
/// A literal double vowel is typed with three keys and the tone before the
/// third (`xoóng` -> `xoosong`). `uơ` is typeable after `h kh th qu` only
/// (`thuở` -> `thuowr`); elsewhere `uow` gives `ươ`.
#[must_use]
pub fn encode(word: &str) -> String {
    encode::keys(word, encode::Style::default())
}

/// Other realistic key orders that type `word` (at most 8, deterministic
/// order, never containing [`encode`]'s): tone right after the vowels
/// (`toasn`), late marks (`theme` = thêm, `vietej`, `tuongw`), a lone `w`
/// for an initial `ư` (`nhw`), a late `d` (`dungd`), and their combinations.
/// Empty when there is no alternative. Invariant (tested): every variant
/// transforms to `word`.
#[must_use]
pub fn encode_variants(word: &str) -> Vec<String> {
    encode::variants(word, |keys| transform(keys).text == word)
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

/// Telex rendering of a raw buffer and its flags.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BufferState {
    /// Vietnamese rendering.
    pub vi: String,
    /// Complete valid Vietnamese syllable.
    pub valid_vi: bool,
    /// Could still become a valid syllable.
    pub viable_vi: bool,
}

/// The Telex stage: raw keys to Vietnamese rendering.
#[derive(Debug, Clone, Copy, Default)]
pub struct TelexTransform;

impl nimk_types::Transform for TelexTransform {
    type Input = str;
    type Output = BufferState;
    fn apply(&self, raw: &str) -> BufferState {
        let tr = transform(raw);
        BufferState {
            vi: tr.text,
            valid_vi: tr.valid_vi,
            viable_vi: tr.viable_vi,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telex_transform_matches_transform() {
        use nimk_types::Transform;
        for raw in ["vieejt", "has", "", "ddaay"] {
            let tr = transform(raw);
            let st = TelexTransform.apply(raw);
            assert_eq!(
                (st.vi.as_str(), st.valid_vi, st.viable_vi),
                (tr.text.as_str(), tr.valid_vi, tr.viable_vi)
            );
        }
    }

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
        check("status", "status", false, false);
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
        check("ww", "w", true, false);
        check("www", "ww", true, false);
    }

    #[test]
    fn lone_w() {
        check("w", "ư", true, true);
        check("W", "Ư", true, true);
        check("tw", "tư", true, true);
        check("nhw", "như", true, true);
        check("nhwng", "nhưng", true, true);
        check("twowng", "tương", true, true);
        check("dwowngf", "dường", true, true);
        check("dduwowngf", "đường", true, true);
        check("ddwowngf", "đường", true, true);
        check("NHWNG", "NHƯNG", true, true);
        check("Nhwngj", "Nhựng", true, true);
        check("ws", "ứ", true, true);
        check("mwa", "mưa", true, true);
        check("Tww", "Tw", true, false);
        check("twww", "tww", true, false);
        check("aw", "ă", true, false);
        check("ow", "ơ", true, true);
        check("uw", "ư", true, true);
        check("uow", "ươ", true, false);
        check("new", "new", false, false);
        // `w` after a non-viable onset stays literal (R6: marks need a viable result).
        check("show", "show", false, false);
        let when = transform("when");
        assert_eq!(when.text, "ưhen");
        assert!(when.has_telex && !when.viable_vi && !when.valid_vi);
        assert!(transform("w").viable_vi);
    }

    #[test]
    fn variants() {
        let has = |w: &str, k: &str| {
            let v = encode_variants(w);
            assert!(v.iter().any(|x| x == k), "{w}: {v:?} lacks {k}");
        };
        has("như", "nhw");
        has("đường", "ddwowngf");
        has("Ư", "W");
        has("thêm", "theme");
        has("việt", "vietej");
        has("tương", "tuongw");
        has("đung", "dungd");
        has("toán", "toasn");
        has("hoàn", "hoafn");
        assert!(encode_variants("deploy").is_empty());
        assert!(encode_variants("aư").is_empty());
        for w in ["đường", "thuở", "Việt", "nguời", "quốc"] {
            let v = encode_variants(w);
            assert!(v.len() <= 8 && !v.contains(&encode(w)), "{w}: {v:?}");
            assert_eq!(v, encode_variants(w), "deterministic");
        }
    }

    #[test]
    fn v2_flexible_typing() {
        check("thuowr", "thuở", true, true);
        check("thuowng", "thương", true, true);
        check("tuwong", "tương", true, true);
        check("dungd", "đung", true, true);
        check("toio", "tôi", true, true);
        // Undo locks the rest of the word as raw.
        check("assf", "asf", true, false);
        check("tooss", "tôs", true, false);
        check("bacr", "bacr", false, false);
        check("giw", "gư", true, true);
        let old = transform_with("hoaf", ToneStyle::Old);
        assert_eq!(old.text, "hòa");
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
