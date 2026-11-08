//! Inverse of the Telex engine: realistic key orders that type a word.
//!
//! [`keys`] renders one key order from a [`Style`]; [`variants`] tries the
//! styles in a fixed order of realism and keeps those that really transform
//! back to the word, so the property `transform(variant) == word` holds by
//! construction.

use crate::letters::{self, Mark, Tone, mark_of, plain, split_tone, to_lower};

/// Most variants [`variants`] returns.
pub(crate) const MAX_VARIANTS: usize = 8;

/// A key-order style (all `false` = the canonical order).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Style {
    /// Tone key right after the first vowel run (`toasn`), not at the end.
    pub tone_early: bool,
    /// Mark keys (`aa ee oo w`) after the letters they modify (`theme`).
    pub late_marks: bool,
    /// An initial `ư` typed with a lone `w` (`nhw`).
    pub lone_w: bool,
    /// The stroke of an initial `đ` typed as a late `d` (`dungd`).
    pub late_d: bool,
}

/// Styles by decreasing realism: single changes first, then pairs.
const ORDER: [Style; 15] = [
    s(true, false, false, false),
    s(false, true, false, false),
    s(false, false, true, false),
    s(false, false, false, true),
    s(true, true, false, false),
    s(true, false, true, false),
    s(false, true, true, false),
    s(true, false, false, true),
    s(false, true, false, true),
    s(false, false, true, true),
    s(true, true, true, false),
    s(true, true, false, true),
    s(true, false, true, true),
    s(false, true, true, true),
    s(true, true, true, true),
];

const fn s(tone_early: bool, late_marks: bool, lone_w: bool, late_d: bool) -> Style {
    Style {
        tone_early,
        late_marks,
        lone_w,
        late_d,
    }
}

/// Key order of `word` in `style`.
pub(crate) fn keys(word: &str, style: Style) -> String {
    let mut out = String::with_capacity(word.len() + 4);
    let mut late: Vec<char> = Vec::new();
    let mut tone = Tone::Level;
    let mut tone_sent = false;
    let mut seen_vowel = false;
    let mut in_run = false;
    let mut prev: Option<char> = None;
    let mut late_d = false;
    let mut first = true;
    for c in word.chars() {
        let upper = c.is_uppercase();
        let (base, t) = split_tone(to_lower(c));
        if t != Tone::Level {
            tone = t;
        }
        let vowel = letters::is_vowel(base);
        if !vowel && in_run && style.tone_early && !tone_sent {
            tone_sent = push_tone(&mut out, tone);
        }
        let p = plain(base);
        let typed = if upper { letters::to_upper(p) } else { p };
        let first_vowel = !seen_vowel && vowel;
        seen_vowel |= vowel;
        in_run = vowel;
        if style.lone_w && first_vowel && base == 'ư' {
            out.push(if upper { 'W' } else { 'w' });
            prev = None;
            first = false;
            continue;
        }
        out.push(typed);
        let mark = mark_of(base);
        match mark {
            Some(Mark::Hat) if style.late_marks => late.push(p),
            Some(Mark::Hat) => out.push(p),
            Some(Mark::Breve | Mark::Horn) if style.late_marks => late.push('w'),
            Some(Mark::Breve | Mark::Horn) => out.push('w'),
            Some(Mark::Stroke) if style.late_d && first => late_d = true,
            Some(Mark::Stroke) => out.push('d'),
            None if matches!(p, 'a' | 'e' | 'o') && prev == Some(p) => {
                // Literal double vowel: the tone goes before the undo key.
                if !tone_sent {
                    tone_sent = push_tone(&mut out, tone);
                }
                out.push(typed);
            }
            None => {}
        }
        prev = mark.is_none().then_some(p);
        first = false;
    }
    late.dedup();
    out.extend(late);
    if late_d {
        out.push('d');
    }
    if !tone_sent {
        push_tone(&mut out, tone);
    }
    out
}

fn push_tone(out: &mut String, tone: Tone) -> bool {
    match tone.key() {
        Some(k) => {
            out.push(k);
            true
        }
        None => false,
    }
}

/// Distinct key orders other than the canonical one that type `word`,
/// at most [`MAX_VARIANTS`], in a fixed order.
pub(crate) fn variants(word: &str, typed_as: impl Fn(&str) -> bool) -> Vec<String> {
    let canonical = keys(word, Style::default());
    let mut out: Vec<String> = Vec::new();
    for style in ORDER {
        let k = keys(word, style);
        if k != canonical && !out.contains(&k) && typed_as(&k) {
            out.push(k);
            if out.len() == MAX_VARIANTS {
                break;
            }
        }
    }
    out
}
