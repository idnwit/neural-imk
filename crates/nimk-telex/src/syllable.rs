//! Syllable structure: initial consonant, vowel nucleus, final consonant.
//! Used for tone placement and validity checks. Inputs are lowercase,
//! toneless letters (marks such as `ê`/`đ` included).

use crate::letters::{Tone, is_vi_letter, is_vowel, mark_of, plain};

/// Initial consonants, longest first so prefix matching is greedy.
const INITIALS: [&str; 27] = [
    "ngh", "ng", "nh", "ch", "gh", "gi", "kh", "ph", "qu", "th", "tr", "b", "c", "d", "đ", "g",
    "h", "k", "l", "m", "n", "p", "r", "s", "t", "v", "x",
];

/// Final consonants (empty final is handled separately).
const FINALS: [&str; 8] = ["c", "ch", "m", "n", "ng", "nh", "p", "t"];

/// Whether a vowel group may stand without / with a final consonant.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Closure {
    /// Only without a final (`ai`, `ưu`).
    Open,
    /// Only with a final (`ă`, `iê`).
    Closed,
    /// Either (`a`, `oa`).
    Both,
}

/// Vowel groups (nucleus plus glides) of Vietnamese rhymes.
const GROUPS: [(&str, Closure); 54] = [
    ("a", Closure::Both),
    ("ă", Closure::Closed),
    ("â", Closure::Closed),
    ("e", Closure::Both),
    ("ê", Closure::Both),
    ("i", Closure::Both),
    ("o", Closure::Both),
    ("ô", Closure::Both),
    ("ơ", Closure::Both),
    ("u", Closure::Both),
    ("ư", Closure::Both),
    ("y", Closure::Open),
    ("ai", Closure::Open),
    ("ao", Closure::Open),
    ("au", Closure::Open),
    ("ay", Closure::Open),
    ("âu", Closure::Open),
    ("ây", Closure::Open),
    ("eo", Closure::Open),
    ("êu", Closure::Open),
    ("ia", Closure::Open),
    ("iê", Closure::Closed),
    ("iu", Closure::Open),
    ("oa", Closure::Both),
    ("oă", Closure::Closed),
    ("oe", Closure::Both),
    ("oi", Closure::Open),
    ("ôi", Closure::Open),
    ("ơi", Closure::Open),
    ("oo", Closure::Closed),
    ("ua", Closure::Open),
    ("uâ", Closure::Closed),
    ("uê", Closure::Both),
    ("ui", Closure::Open),
    ("uô", Closure::Closed),
    ("uơ", Closure::Open),
    ("uy", Closure::Both),
    ("ưa", Closure::Open),
    ("ưi", Closure::Open),
    ("ươ", Closure::Closed),
    ("ưu", Closure::Open),
    ("yê", Closure::Closed),
    ("iêu", Closure::Open),
    ("oai", Closure::Open),
    ("oay", Closure::Open),
    ("oeo", Closure::Open),
    ("uây", Closure::Open),
    ("uôi", Closure::Open),
    ("uya", Closure::Open),
    ("uyê", Closure::Closed),
    ("uyu", Closure::Open),
    ("ươi", Closure::Open),
    ("ươu", Closure::Open),
    ("yêu", Closure::Open),
];

/// Groups that take `ch`/`nh` finals.
const PALATAL_OK: [&str; 6] = ["a", "ê", "i", "oa", "uê", "uy"];
/// Groups that never take `ng`/`c` finals.
const VELAR_BAD: [&str; 4] = ["i", "ê", "uy", "uê"];
/// Pairs whose tone sits on the second vowel in an open syllable
/// ("new style": `hoà`, `hoè`, `thuý`).
const SECOND_VOWEL_PAIRS: [&str; 5] = ["oa", "oă", "oe", "ue", "uy"];

fn starts_with(letters: &[char], pat: &str) -> bool {
    pat.chars()
        .enumerate()
        .all(|(i, p)| letters.get(i) == Some(&p))
}

fn matches(letters: &[char], pat: &str) -> bool {
    letters.len() == pat.chars().count() && starts_with(letters, pat)
}

/// Index just past the run of vowels starting at `from`.
fn vowel_run_end(letters: &[char], from: usize) -> usize {
    let mut end = from;
    while end < letters.len() && is_vowel(letters[end]) {
        end += 1;
    }
    end
}

/// Length of the initial consonant at the start of `letters`; 0 if none.
/// `gi` and `qu` count as initials only when a vowel follows.
pub(crate) fn initial_len(letters: &[char]) -> usize {
    for pat in INITIALS {
        if !starts_with(letters, pat) {
            continue;
        }
        let len = pat.chars().count();
        if (pat == "gi" || pat == "qu") && !letters.get(len).is_some_and(|c| is_vowel(*c)) {
            continue;
        }
        return len;
    }
    0
}

/// Index of the letter that carries the tone, or `None` if there is no vowel.
///
/// Rules ("new style" placement): a marked vowel wins (the last one in
/// `ươ`); otherwise with a final consonant the last vowel; in an open
/// syllable the second vowel of `oa oă oe ue uy`, the first of other pairs,
/// and the middle of triples.
pub(crate) fn tone_position(letters: &[char]) -> Option<usize> {
    let from = initial_len(letters);
    let start = (from..letters.len()).find(|&i| is_vowel(letters[i]))?;
    let end = vowel_run_end(letters, start);
    let run = &letters[start..end];
    if let Some(p) = run.iter().rposition(|c| mark_of(*c).is_some()) {
        return Some(start + p);
    }
    let has_final = end < letters.len();
    let pos = match (run.len(), has_final) {
        (1, _) => start,
        (_, true) => end - 1,
        (2, false) if SECOND_VOWEL_PAIRS.iter().any(|p| matches(run, p)) => start + 1,
        (2, false) => start,
        _ => start + 1,
    };
    Some(pos)
}

/// Returns true if `letters` (lowercase, toneless) with `tone` is a valid syllable.
pub(crate) fn is_valid(letters: &[char], tone: Tone) -> bool {
    if letters.is_empty() || !letters.iter().all(|c| is_vi_letter(*c)) {
        return false;
    }
    let ini_len = initial_len(letters);
    let initial = &letters[..ini_len];
    let end = vowel_run_end(letters, ini_len);
    let nucleus = &letters[ini_len..end];
    let fin = &letters[end..];
    let Some(&first) = nucleus.first() else {
        return false;
    };
    if !fin.is_empty() && !FINALS.iter().any(|f| matches(fin, f)) {
        return false;
    }
    if !initial_fits(initial, first) {
        return false;
    }
    // `qu`/`gi` absorb a glide: `qua` rhymes like `oa`, `quy` like `uy`, `giê` like `iê`.
    let mut group: Vec<char> = Vec::with_capacity(nucleus.len() + 1);
    if matches(initial, "qu") {
        group.push(if matches!(first, 'a' | 'ă' | 'e') {
            'o'
        } else {
            'u'
        });
    } else if matches(initial, "gi") && first == 'ê' {
        group.push('i');
    }
    group.extend_from_slice(nucleus);
    rhyme_fits(initial, &group, fin, tone)
}

/// Returns true if `letters` (lowercase, toneless) is a prefix of some valid
/// syllable. Letters are compared without marks, because Telex can still add
/// a mark later (`tieng` + `e` -> `tiêng`); the tone is ignored.
pub(crate) fn is_viable(letters: &[char]) -> bool {
    if !letters.iter().all(|c| is_vi_letter(*c)) {
        return false;
    }
    let typed: Vec<char> = letters.iter().map(|c| plain(*c)).collect();
    let mut cand: Vec<char> = Vec::with_capacity(8);
    for ini in core::iter::once("").chain(INITIALS) {
        cand.clear();
        cand.extend(ini.chars());
        if !prefix_compatible(&typed, &cand) {
            continue;
        }
        if typed.len() <= cand.len() {
            return true;
        }
        let ini_len = cand.len();
        for (group, _) in GROUPS {
            cand.truncate(ini_len);
            cand.extend(group.chars());
            if !prefix_compatible(&typed, &cand) {
                continue;
            }
            let group_end = cand.len();
            for fin in core::iter::once("").chain(FINALS) {
                cand.truncate(group_end);
                cand.extend(fin.chars());
                let fits = typed.len() <= cand.len() && prefix_compatible(&typed, &cand);
                if fits && (is_valid(&cand, Tone::Acute) || is_valid(&cand, Tone::Level)) {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether the overlapping part of `typed` (plain letters) and `cand` agree.
fn prefix_compatible(typed: &[char], cand: &[char]) -> bool {
    typed.iter().zip(cand).all(|(t, c)| *t == plain(*c))
}

/// Spelling constraints between the initial and the first nucleus letter.
fn initial_fits(initial: &[char], first: char) -> bool {
    let front = matches!(first, 'i' | 'e' | 'ê' | 'y');
    if matches(initial, "k") {
        front
    } else if matches(initial, "gh") || matches(initial, "ngh") {
        matches!(first, 'i' | 'e' | 'ê')
    } else if matches(initial, "c") {
        !front
    } else if matches(initial, "g") {
        !matches!(first, 'e' | 'ê')
    } else if matches(initial, "ng") {
        !matches!(first, 'e' | 'ê' | 'i')
    } else if matches(initial, "qu") {
        !matches!(first, 'u' | 'ư' | 'o')
    } else {
        true
    }
}

/// Constraints between the vowel group, the final and the tone.
fn rhyme_fits(initial: &[char], group: &[char], fin: &[char], tone: Tone) -> bool {
    let Some(&(name, closure)) = GROUPS.iter().find(|(g, _)| matches(group, g)) else {
        return false;
    };
    let ok_closure = match closure {
        Closure::Open => fin.is_empty(),
        Closure::Closed => !fin.is_empty(),
        Closure::Both => true,
    };
    if !ok_closure {
        return false;
    }
    if name.starts_with("yê") && !initial.is_empty() {
        return false;
    }
    if name.starts_with("iê") && initial.is_empty() {
        return false;
    }
    let is = |f: &str| matches(fin, f);
    if (is("ch") || is("nh")) && !PALATAL_OK.contains(&name) {
        return false;
    }
    if (is("ng") || is("c")) && VELAR_BAD.contains(&name) {
        return false;
    }
    if name == "oo" && !(is("ng") || is("c")) {
        return false;
    }
    let stop = is("c") || is("ch") || is("p") || is("t");
    !stop || tone.fits_stop_final()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn initials() {
        assert_eq!(initial_len(&chars("nghieng")), 3);
        assert_eq!(initial_len(&chars("gin")), 1);
        assert_eq!(initial_len(&chars("gia")), 2);
        assert_eq!(initial_len(&chars("qua")), 2);
        assert_eq!(initial_len(&chars("an")), 0);
    }

    #[test]
    fn tone_positions() {
        let cases = [
            ("hoa", 2),
            ("mua", 1),
            ("toan", 2),
            ("tiêng", 2),
            ("đươc", 2),
            ("khuyu", 3),
            ("oai", 1),
            ("gia", 2),
            ("gi", 1),
            ("ue", 1),
        ];
        for (w, p) in cases {
            assert_eq!(tone_position(&chars(w)), Some(p), "{w}");
        }
        assert_eq!(tone_position(&chars("str")), None);
    }

    #[test]
    fn validity() {
        let ok = [
            "tiêng", "viêt", "đương", "nghiêng", "quyêt", "gi", "gin", "giêng", "xoong", "khuya",
        ];
        for w in ok {
            let tone = if w.ends_with('t') {
                Tone::Dot
            } else {
                Tone::Level
            };
            assert!(is_valid(&chars(w), tone), "{w}");
        }
        for w in ["ki", "kia", "ong", "ăn", "uya"] {
            assert!(is_valid(&chars(w), Tone::Acute), "{w}");
        }
        for w in ["ce", "nge", "ghă", "str", "fo", "tyên", "iên", "ă", "ing"] {
            assert!(!is_valid(&chars(w), Tone::Acute), "{w}");
        }
        assert!(!is_valid(&chars("tat"), Tone::Grave));
        assert!(is_valid(&chars("tat"), Tone::Acute));
    }
}
