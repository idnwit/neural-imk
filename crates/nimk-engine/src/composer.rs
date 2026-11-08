//! Composer: raw keys of the current word, its renderings and word ids.
//!
//! The composer does not know Telex: `engine.rs` computes the Telex
//! rendering of [`Composer::raw`] and hands it back with
//! [`Composer::set_vi`]. The English rendering is the raw keys themselves.
//!
//! Boundary keys commit the word and are then handed back to the app
//! (`consumed = false`), so the app inserts the space, punctuation or runs
//! the shortcut itself.

use nimk_types::{Boundary, KeyEvent, KeyKind, Lang, keycode};

/// Longest word the composer keeps; a further letter splits the word
/// (committed with [`Boundary::Punct`]) so per-key work stays bounded.
pub(crate) const MAX_RAW: usize = 64;

/// What a key means to the composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyClass {
    /// An ASCII letter that extends the word.
    Letter(char),
    /// Backspace.
    Backspace,
    /// Ends the word, then goes to the app.
    Boundary(Boundary),
    /// Not a word key (modifier change); goes to the app untouched.
    Ignore,
}

/// Classifies one key event.
#[must_use]
pub(crate) fn classify(key: &KeyEvent) -> KeyClass {
    if key.kind == KeyKind::FlagsChanged {
        return KeyClass::Ignore;
    }
    if key.mods.is_shortcut() {
        return KeyClass::Boundary(Boundary::Shortcut);
    }
    match key.keycode {
        keycode::BACKSPACE => return KeyClass::Backspace,
        keycode::SPACE => return KeyClass::Boundary(Boundary::Space),
        keycode::RETURN => return KeyClass::Boundary(Boundary::Return),
        keycode::TAB => return KeyClass::Boundary(Boundary::Tab),
        keycode::ESCAPE | keycode::LEFT | keycode::RIGHT | keycode::UP | keycode::DOWN => {
            return KeyClass::Boundary(Boundary::Navigation);
        }
        _ => {}
    }
    let mut chars = key.chars.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => KeyClass::Letter(c),
        (Some('\r' | '\n'), None) => KeyClass::Boundary(Boundary::Return),
        // Function keys (Home, End, F1...) use the private-use area; keys
        // without characters are also navigation-like.
        (Some(c), None) if c.is_control() || ('\u{F700}'..='\u{F8FF}').contains(&c) => {
            KeyClass::Boundary(Boundary::Navigation)
        }
        (None, _) => KeyClass::Boundary(Boundary::Navigation),
        _ => KeyClass::Boundary(Boundary::Punct),
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
        // At most MAX_RAW (64), so the cast never truncates.
        self.raw.len().min(usize::from(u16::MAX)) as u16
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
    fn classify_table() {
        let cases = [
            (key(0, "a", 0), KeyClass::Letter('a')),
            (key(0, "A", Modifiers::SHIFT), KeyClass::Letter('A')),
            (
                key(0, "a", Modifiers::COMMAND),
                KeyClass::Boundary(Boundary::Shortcut),
            ),
            (
                key(0, "å", Modifiers::OPTION),
                KeyClass::Boundary(Boundary::Shortcut),
            ),
            (
                key(keycode::SPACE, " ", 0),
                KeyClass::Boundary(Boundary::Space),
            ),
            (
                key(keycode::RETURN, "\r", 0),
                KeyClass::Boundary(Boundary::Return),
            ),
            (key(76, "\r", 0), KeyClass::Boundary(Boundary::Return)),
            (
                key(keycode::TAB, "\t", 0),
                KeyClass::Boundary(Boundary::Tab),
            ),
            (key(keycode::BACKSPACE, "\u{7f}", 0), KeyClass::Backspace),
            (
                key(keycode::LEFT, "\u{F702}", 0),
                KeyClass::Boundary(Boundary::Navigation),
            ),
            (
                key(keycode::ESCAPE, "\u{1b}", 0),
                KeyClass::Boundary(Boundary::Navigation),
            ),
            (
                key(115, "\u{F729}", 0),
                KeyClass::Boundary(Boundary::Navigation),
            ),
            (key(29, "0", 0), KeyClass::Boundary(Boundary::Punct)),
            (key(43, ",", 0), KeyClass::Boundary(Boundary::Punct)),
            (key(0, "é", 0), KeyClass::Boundary(Boundary::Punct)),
            (key(0, "", 0), KeyClass::Boundary(Boundary::Navigation)),
        ];
        for (k, want) in cases {
            assert_eq!(classify(&k), want, "key {k:?}");
        }
        let flags = KeyEvent {
            kind: KeyKind::FlagsChanged,
            ..key(56, "", Modifiers::SHIFT)
        };
        assert_eq!(classify(&flags), KeyClass::Ignore);
    }

    #[test]
    fn backspace_and_ids() {
        let mut c = Composer::new();
        assert!(!c.pop(), "empty pop passes through");
        assert_eq!(c.take(), None);
        assert_eq!(c.word_id(), 0);
        assert_eq!(c.last_committed(), None);

        c.push('a');
        c.push('s');
        c.set_vi("á");
        assert_eq!(c.rendering(Lang::Vi), "á");
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
