//! Composer: raw keys of the current word, its renderings and word ids.
//!
//! The composer is a dumb buffer: it does not know Telex or when a word
//! ends. `engine.rs` computes the Telex rendering of [`Composer::raw`] and
//! hands it back with [`Composer::set_vi`]; every edit is decided by the
//! action model (ADR 0007). The English rendering is the raw keys.

use nimk_types::{Boundary, KeyEvent, KeyKind, Lang, keycode};

/// Longest word the composer keeps; at this length `Append` is masked out
/// so per-key work stays bounded.
pub(crate) const MAX_RAW: usize = 64;

/// The printable character a key produces, if any: a key-down without
/// Cmd/Ctrl/Option whose `chars` is exactly one character that is not a
/// control or private-use (function key) code point.
#[must_use]
pub(crate) fn printable_char(key: &KeyEvent) -> Option<char> {
    if key.kind != KeyKind::KeyDown || key.mods.is_shortcut() {
        return None;
    }
    let mut chars = key.chars.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_control() && !('\u{E000}'..='\u{F8FF}').contains(&c) => Some(c),
        _ => None,
    }
}

/// What ended a word, for the `WordCommit` log record.
#[must_use]
pub(crate) fn boundary_of(key: &KeyEvent) -> Boundary {
    if key.mods.is_shortcut() {
        return Boundary::Shortcut;
    }
    match key.keycode {
        keycode::SPACE => return Boundary::Space,
        keycode::RETURN => return Boundary::Return,
        keycode::TAB => return Boundary::Tab,
        keycode::ESCAPE | keycode::LEFT | keycode::RIGHT | keycode::UP | keycode::DOWN => {
            return Boundary::Navigation;
        }
        _ => {}
    }
    if printable_char(key).is_some() {
        Boundary::Punct
    } else if matches!(key.chars.as_str(), "\r" | "\n") {
        Boundary::Return
    } else {
        Boundary::Navigation
    }
}

/// A word taken out of the composer by [`Composer::take`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Committed {
    /// Word id.
    pub(crate) id: u64,
    /// Raw keys.
    pub(crate) raw: String,
    /// Telex rendering.
    pub(crate) vi: String,
}

/// Current word state.
#[derive(Debug, Clone, Default)]
pub(crate) struct Composer {
    raw: String,
    vi: String,
    next_id: u64,
}

impl Composer {
    /// Creates an empty composer; the first word gets id 0.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            raw: String::with_capacity(MAX_RAW),
            vi: String::with_capacity(MAX_RAW * 2),
            next_id: 0,
        }
    }

    /// Raw keys of the current word.
    #[must_use]
    pub(crate) fn raw(&self) -> &str {
        &self.raw
    }

    /// Whether no word is being composed.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// Whether the word reached [`MAX_RAW`] keys.
    #[must_use]
    pub(crate) fn is_full(&self) -> bool {
        self.raw.len() >= MAX_RAW
    }

    /// Number of keys in the current word.
    #[must_use]
    pub(crate) fn key_count(&self) -> u16 {
        // Saturating: the cast cannot truncate after `min`.
        self.raw.chars().count().min(usize::from(u16::MAX)) as u16
    }

    /// Id the current (or next) word has / will get.
    #[must_use]
    pub(crate) fn word_id(&self) -> u64 {
        self.next_id
    }

    /// Id of the most recently committed word, if any.
    #[must_use]
    pub(crate) fn last_committed(&self) -> Option<u64> {
        self.next_id.checked_sub(1)
    }

    /// Appends a letter. Callers check [`Composer::is_full`] first. The
    /// Telex rendering is stale until [`Composer::set_vi`].
    pub(crate) fn push(&mut self, c: char) {
        self.raw.push(c);
    }

    /// Removes the last key; returns `false` if the word was empty.
    pub(crate) fn pop(&mut self) -> bool {
        self.raw.pop().is_some()
    }

    /// Stores the Telex rendering of the current raw keys.
    pub(crate) fn set_vi(&mut self, vi: &str) {
        self.vi.clear();
        self.vi.push_str(vi);
    }

    /// Rendering of the current word in `mode`.
    #[must_use]
    pub(crate) fn rendering(&self, mode: Lang) -> &str {
        match mode {
            Lang::Vi => &self.vi,
            Lang::En => &self.raw,
        }
    }

    /// Takes the current word for commit. Returns `None` (and assigns no id)
    /// if the word is empty; otherwise the word id counter advances.
    pub(crate) fn take(&mut self) -> Option<Committed> {
        if self.raw.is_empty() {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        let raw = self.raw.clone();
        let vi = self.vi.clone();
        self.raw.clear();
        self.vi.clear();
        Some(Committed { id, raw, vi })
    }
}

#[cfg(test)]
mod tests {
    use nimk_types::Modifiers;

    use super::*;

    fn key(code: u16, chars: &str, mods: u32) -> KeyEvent {
        KeyEvent {
            kind: KeyKind::KeyDown,
            keycode: code,
            chars: chars.to_owned(),
            mods: Modifiers(mods),
            timestamp: 0.0,
        }
    }

    #[test]
    fn printable_table() {
        let flags = KeyEvent {
            kind: KeyKind::FlagsChanged,
            ..key(56, "", Modifiers::SHIFT)
        };
        let cases = [
            (key(0, "a", 0), Some('a')),
            (key(0, "A", Modifiers::SHIFT), Some('A')),
            (key(keycode::SPACE, " ", 0), Some(' ')),
            (key(43, ",", 0), Some(',')),
            (key(0, "\u{e9}", 0), Some('\u{e9}')),
            (key(0, "a", Modifiers::COMMAND), None),
            (key(0, "a", Modifiers::CONTROL), None),
            (key(0, "\u{e5}", Modifiers::OPTION), None),
            (key(keycode::BACKSPACE, "\u{7f}", 0), None),
            (key(keycode::RETURN, "\r", 0), None),
            (key(keycode::TAB, "\t", 0), None),
            (key(keycode::LEFT, "\u{F702}", 0), None),
            (key(0, "", 0), None),
            (key(0, "ab", 0), None),
            (flags, None),
        ];
        for (k, want) in cases {
            assert_eq!(printable_char(&k), want, "key {k:?}");
        }
    }

    #[test]
    fn boundary_table() {
        let cases = [
            (key(keycode::SPACE, " ", 0), Boundary::Space),
            (key(keycode::RETURN, "\r", 0), Boundary::Return),
            (key(76, "\r", 0), Boundary::Return),
            (key(keycode::TAB, "\t", 0), Boundary::Tab),
            (key(0, "a", Modifiers::COMMAND), Boundary::Shortcut),
            (key(keycode::LEFT, "\u{F702}", 0), Boundary::Navigation),
            (key(keycode::ESCAPE, "\u{1b}", 0), Boundary::Navigation),
            (key(keycode::BACKSPACE, "\u{7f}", 0), Boundary::Navigation),
            (key(43, ",", 0), Boundary::Punct),
            (key(0, "a", 0), Boundary::Punct),
            (key(0, "", 0), Boundary::Navigation),
        ];
        for (k, want) in cases {
            assert_eq!(boundary_of(&k), want, "key {k:?}");
        }
    }

    #[test]
    fn backspace_and_ids() {
        let mut c = Composer::new();
        assert!(!c.pop(), "empty pop reports false");
        assert_eq!(c.take(), None);
        assert_eq!(c.word_id(), 0);
        assert_eq!(c.last_committed(), None);

        c.push('a');
        c.push('s');
        c.set_vi("\u{e1}");
        assert_eq!(c.rendering(Lang::Vi), "\u{e1}");
        assert_eq!(c.rendering(Lang::En), "as");
        assert_eq!(c.key_count(), 2);
        assert!(c.pop());
        assert!(c.pop());
        assert!(c.is_empty());
        assert_eq!(c.take(), None, "erased word takes no id");

        c.push('x');
        c.set_vi("x");
        let w = c.take();
        assert_eq!(
            w,
            Some(Committed {
                id: 0,
                raw: "x".into(),
                vi: "x".into()
            })
        );
        assert_eq!(c.word_id(), 1);
        assert_eq!(c.last_committed(), Some(0));
        assert!(c.is_empty());
        assert_eq!(c.rendering(Lang::Vi), "");
    }

    #[test]
    fn full_word() {
        let mut c = Composer::new();
        for _ in 0..MAX_RAW {
            assert!(!c.is_full());
            c.push('a');
        }
        assert!(c.is_full());
        assert_eq!(c.key_count(), 64);
    }
}
