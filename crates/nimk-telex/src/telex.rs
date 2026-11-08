//! Key-by-key Telex state machine behind [`crate::transform`] (spec: docs/10).

use crate::letters::{Mark, Tone, is_vowel, mark_of, plain, to_lower, to_upper, with_tone};
use crate::syllable::{self, ToneStyle, tone_position};

/// Final consonants a mark key may "reach over" (`tieng` + `e` -> `tiêng`).
const REACHABLE_FINALS: [&str; 9] = ["", "c", "ch", "m", "n", "ng", "nh", "p", "t"];
/// Finals that only take sắc or nặng (R3).
const STOP_FINALS: [&str; 4] = ["c", "ch", "p", "t"];
/// Onsets after which `uo` + `w` with nothing following stays `uơ` (`thuở`).
const UO_HORN_ONSETS: [&str; 3] = ["h", "kh", "th"];

/// One rendered letter: lowercase base (marks included, tone excluded) + case.
#[derive(Debug, Clone, Copy)]
struct Letter {
    base: char,
    upper: bool,
}

/// Telex composition state for one word (R1-R10).
///
/// The tone lives on the syllable and is placed at render time, so typing
/// order does not matter (`tiengs` == `tieengs` after `e`). Repeating a key
/// whose effect is on undoes it, appends the key literally and locks the rest
/// of the word as raw (R7).
#[derive(Debug, Default)]
pub(crate) struct Composer {
    letters: Vec<Letter>,
    tone: Tone,
    style: ToneStyle,
    locked: bool,
    /// Index of a letter created by a lone `w` (`w` -> `ư`), so a second `w`
    /// can turn it back into a literal `w`.
    lone_w: Option<usize>,
}

impl Composer {
    pub(crate) fn new(style: ToneStyle) -> Self {
        Self {
            style,
            ..Self::default()
        }
    }

    /// Applies one key.
    pub(crate) fn push(&mut self, key: char) {
        let lower = to_lower(key);
        let handled = !self.locked
            && match lower {
                's' | 'f' | 'r' | 'x' | 'j' => {
                    Tone::from_key(lower).is_some_and(|t| self.tone_key(t))
                }
                'z' => self.clear_tone(),
                'a' | 'e' | 'o' => self.hat_key(lower),
                'w' => self.horn_key(key.is_uppercase()),
                'd' => self.stroke_key(),
                _ => false,
            };
        if !handled {
            self.letters.push(Letter {
                base: lower,
                upper: key.is_uppercase(),
            });
            if !self.locked {
                self.complete_uo();
            }
        }
    }

    /// Renders the current text with the tone placed.
    pub(crate) fn render(&self) -> String {
        let bases = self.bases();
        let tone_at = if self.tone == Tone::Level {
            None
        } else {
            tone_position(&bases, self.style)
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

    fn bases(&self) -> Vec<char> {
        self.letters.iter().map(|l| l.base).collect()
    }

    /// Vowel run of the syllable's nucleus: the last run of vowels after the
    /// onset (`qu`/`gi` count as onset when a vowel follows).
    fn cluster(&self) -> Option<(usize, usize)> {
        let b = self.bases();
        let ini = syllable::initial_len(&b);
        let end = b.iter().rposition(|c| is_vowel(*c))? + 1;
        let mut start = end;
        while start > ini && is_vowel(b[start - 1]) {
            start -= 1;
        }
        (start < end).then_some((start, end))
    }

    /// Consonants after index `end`, as a string.
    fn tail(&self, end: usize) -> String {
        self.letters[end..].iter().map(|l| l.base).collect()
    }

    /// Whether `edits` (index, new base) leave a valid syllable or a viable prefix.
    fn viable_with(&self, edits: &[(usize, char)], extra: Option<char>) -> bool {
        let mut b = self.bases();
        for &(i, c) in edits {
            b[i] = c;
        }
        b.extend(extra);
        syllable::is_viable(&b)
    }

    fn tone_key(&mut self, tone: Tone) -> bool {
        let Some((_, end)) = self.cluster() else {
            return false;
        };
        // A second vowel run is no Vietnamese syllable (`status`): literal.
        let b = self.bases();
        let ini = syllable::initial_len(&b);
        let mut runs = 0;
        let mut prev = false;
        for c in &b[ini..] {
            let v = is_vowel(*c);
            runs += usize::from(v && !prev);
            prev = v;
        }
        if runs > 1 {
            return false;
        }
        let tail = self.tail(end);
        if STOP_FINALS.contains(&tail.as_str()) && !tone.fits_stop_final() {
            return false;
        }
        if self.tone == tone {
            self.tone = Tone::Level;
            self.locked = true;
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

    fn reachable(&self, end: usize) -> bool {
        REACHABLE_FINALS.contains(&self.tail(end).as_str())
    }

    fn hat_key(&mut self, key: char) -> bool {
        let Some((s, e)) = self.cluster() else {
            return false;
        };
        if !self.reachable(e) {
            return false;
        }
        let Some(i) = (s..e).rev().find(|&i| {
            let b = self.letters[i].base;
            plain(b) == key && matches!(mark_of(b), None | Some(Mark::Hat | Mark::Horn))
        }) else {
            return false;
        };
        let base = self.letters[i].base;
        if mark_of(base) == Some(Mark::Hat) {
            self.letters[i].base = key;
            self.locked = true;
            return false;
        }
        let mut edits = vec![(i, if base == 'ơ' { 'ô' } else { hat(key) })];
        if base == 'ơ' && i > s && self.letters[i - 1].base == 'ư' {
            // `ươ` + `o` -> `uô`.
            edits.push((i - 1, 'u'));
        }
        if !self.viable_with(&edits, None) {
            return false;
        }
        for (j, c) in edits {
            self.letters[j].base = c;
        }
        true
    }

    fn horn_key(&mut self, upper: bool) -> bool {
        let Some((s, e)) = self.cluster() else {
            // No vowel yet: lone `w` is `ư`.
            if !self.viable_with(&[], Some('ư')) {
                return false;
            }
            self.lone_w = Some(self.letters.len());
            self.letters.push(Letter { base: 'ư', upper });
            return true;
        };
        if self.bases() == ['g', 'i'] {
            self.letters[1].base = 'ư';
            return true;
        }
        if !self.reachable(e) {
            return false;
        }
        for i in (s..e).rev() {
            let base = self.letters[i].base;
            if !matches!(plain(base), 'a' | 'o' | 'u') || mark_of(base) == Some(Mark::Hat) {
                continue;
            }
            if matches!(mark_of(base), Some(Mark::Breve | Mark::Horn)) {
                return self.undo_horn(i, s);
            }
            let edits = self.horn_edits(i, s, e);
            if self.viable_with(&edits, None) {
                for (j, c) in edits {
                    self.letters[j].base = c;
                }
                return true;
            }
        }
        false
    }

    /// Letters to change when `w` hits unmarked vowel `i` of cluster `s..e`.
    fn horn_edits(&self, i: usize, s: usize, e: usize) -> Vec<(usize, char)> {
        let base = self.letters[i].base;
        let mut edits = vec![(i, horn(base))];
        if base == 'o' && i > s && plain(self.letters[i - 1].base) == 'u' {
            let onset: String = self.letters[..s].iter().map(|l| l.base).collect();
            let keep_u = UO_HORN_ONSETS.contains(&onset.as_str()) && e == self.letters.len();
            if !keep_u && self.letters[i - 1].base == 'u' {
                edits.push((i - 1, 'ư'));
            }
        }
        edits
    }

    /// Second hook key: reverts it and types `w` (R7).
    fn undo_horn(&mut self, i: usize, s: usize) -> bool {
        let base = self.letters[i].base;
        self.locked = true;
        if self.lone_w == Some(i) {
            self.letters[i].base = 'w';
            return i + 1 == self.letters.len();
        }
        self.letters[i].base = plain(base);
        if base == 'ơ' && i > s && self.letters[i - 1].base == 'ư' {
            self.letters[i - 1].base = 'u';
        }
        false
    }

    /// `ưo` / `uơ` followed by anything completes to `ươ`.
    fn complete_uo(&mut self) {
        let Some((s, e)) = self.cluster() else {
            return;
        };
        let n = self.letters.len();
        for i in s..e.saturating_sub(1) {
            let pair = (self.letters[i].base, self.letters[i + 1].base);
            if matches!(pair, ('ư', 'o') | ('u', 'ơ')) && i + 2 < n {
                let edits = [(i, 'ư'), (i + 1, 'ơ')];
                if self.viable_with(&edits, None) {
                    for (j, c) in edits {
                        self.letters[j].base = c;
                    }
                }
                return;
            }
        }
    }

    /// `dd` -> `đ`: the initial `d`, at any later time if the result is a
    /// valid syllable; a word starting `dd` is always allowed (R6).
    fn stroke_key(&mut self) -> bool {
        match self.letters.first().map(|l| l.base) {
            Some('d') => {
                let mut b = self.bases();
                b[0] = 'đ';
                if self.letters.len() == 1 || syllable::is_valid(&b, Tone::Acute) {
                    self.letters[0].base = 'đ';
                    return true;
                }
                false
            }
            Some('đ') => {
                self.letters[0].base = 'd';
                self.locked = true;
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
