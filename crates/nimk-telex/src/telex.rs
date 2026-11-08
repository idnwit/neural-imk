//! Key-by-key Telex state machine behind [`crate::transform`].

use crate::letters::{Mark, Tone, is_vowel, mark_of, plain, to_lower, to_upper, with_tone};
use crate::syllable::tone_position;

/// Final consonants a mark key may "reach over" (`tieng` + `e` -> `tiêng`).
const REACHABLE_FINALS: [&str; 9] = ["", "c", "ch", "m", "n", "ng", "nh", "p", "t"];

/// One rendered letter: lowercase base (marks included, tone excluded) + case.
#[derive(Debug, Clone, Copy)]
struct Letter {
    base: char,
    upper: bool,
}

/// Telex composition state for one word.
///
/// The tone is kept per word and placed at render time, so typing order
/// does not matter (`tiengs` == `tieengs` after `e`). Pressing a transform
/// key that is already in effect undoes it, appends the key literally and
/// disables that transform family for the rest of the word (`ass` -> `as`,
/// `asss` -> `ass`).
#[derive(Debug, Default)]
pub(crate) struct Composer {
    letters: Vec<Letter>,
    tone: Tone,
    tone_off: bool,
    hat_off: bool,
    horn_off: bool,
    stroke_off: bool,
}

impl Composer {
    /// Applies one key.
    pub(crate) fn push(&mut self, key: char) {
        let lower = to_lower(key);
        let handled = match lower {
            's' | 'f' | 'r' | 'x' | 'j' => Tone::from_key(lower).is_some_and(|t| self.tone_key(t)),
            'z' => self.clear_tone(),
            'a' | 'e' | 'o' => self.hat_key(lower),
            'w' => self.horn_key(),
            'd' => self.stroke_key(),
            _ => false,
        };
        if !handled {
            self.letters.push(Letter {
                base: lower,
                upper: key.is_uppercase(),
            });
        }
    }

    /// Renders the current text with the tone placed.
    pub(crate) fn render(&self) -> String {
        let bases: Vec<char> = self.letters.iter().map(|l| l.base).collect();
        let tone_at = if self.tone == Tone::Level {
            None
        } else {
            tone_position(&bases)
        };
        self.letters
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let c = if tone_at == Some(i) {
                    with_tone(l.base, self.tone)
                } else {
                    l.base
                };
                if l.upper { to_upper(c) } else { c }
            })
            .collect()
    }

    fn tone_key(&mut self, tone: Tone) -> bool {
        if self.tone_off || !self.letters.iter().any(|l| is_vowel(l.base)) {
            return false;
        }
        if self.tone == tone {
            self.tone = Tone::Level;
            self.tone_off = true;
            return false;
        }
        self.tone = tone;
        true
    }

    fn clear_tone(&mut self) -> bool {
        if self.tone == Tone::Level {
            return false;
        }
        self.tone = Tone::Level;
        true
    }

    /// Index of the rightmost vowel, if only a reachable final follows it.
    fn last_vowel(&self) -> Option<usize> {
        let i = self.letters.iter().rposition(|l| is_vowel(l.base))?;
        let tail: String = self.letters[i + 1..].iter().map(|l| l.base).collect();
        REACHABLE_FINALS.contains(&tail.as_str()).then_some(i)
    }

    fn hat_key(&mut self, key: char) -> bool {
        if self.hat_off {
            return false;
        }
        let Some(i) = self.last_vowel() else {
            return false;
        };
        let base = self.letters[i].base;
        if plain(base) != key {
            return false;
        }
        if mark_of(base) == Some(Mark::Hat) {
            self.letters[i].base = key;
            self.hat_off = true;
            return false;
        }
        self.letters[i].base = hat(key);
        true
    }

    fn horn_key(&mut self) -> bool {
        if self.horn_off {
            return false;
        }
        let Some(last) = self.last_vowel() else {
            return false;
        };
        // Rightmost a/o/u within the final vowel run.
        let mut target = None;
        let mut i = last;
        loop {
            let base = self.letters[i].base;
            if !is_vowel(base) {
                break;
            }
            if matches!(plain(base), 'a' | 'o' | 'u') {
                target = Some(i);
                break;
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }
        let Some(t) = target else {
            return false;
        };
        let base = self.letters[t].base;
        let partner = self.horn_partner(t);
        if matches!(mark_of(base), Some(Mark::Breve | Mark::Horn)) {
            self.letters[t].base = plain(base);
            if let Some(p) = partner {
                self.letters[p].base = 'u';
            }
            self.horn_off = true;
            return false;
        }
        self.letters[t].base = horn(plain(base));
        if let Some(p) = partner {
            self.letters[p].base = 'ư';
        }
        true
    }

    /// For an `o` at `t` preceded by `u` (not the `u` of `qu`), that `u`'s index:
    /// `uo` + `w` -> `ươ`.
    fn horn_partner(&self, t: usize) -> Option<usize> {
        let p = t.checked_sub(1)?;
        let is_o = plain(self.letters[t].base) == 'o';
        let is_u = plain(self.letters[p].base) == 'u';
        let is_qu = p == 1 && self.letters[0].base == 'q';
        (is_o && is_u && !is_qu).then_some(p)
    }

    /// `dd` -> `đ`, only at the start of the word (where `đ` can occur).
    fn stroke_key(&mut self) -> bool {
        if self.stroke_off || self.letters.len() != 1 {
            return false;
        }
        match self.letters[0].base {
            'd' => {
                self.letters[0].base = 'đ';
                true
            }
            'đ' => {
                self.letters[0].base = 'd';
                self.stroke_off = true;
                false
            }
            _ => false,
        }
    }
}

fn hat(key: char) -> char {
    match key {
        'a' => 'â',
        'e' => 'ê',
        'o' => 'ô',
        other => other,
    }
}

fn horn(base: char) -> char {
    match base {
        'a' => 'ă',
        'o' => 'ơ',
        'u' => 'ư',
        other => other,
    }
}
