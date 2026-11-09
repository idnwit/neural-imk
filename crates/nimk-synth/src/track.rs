//! Mirror of the engine's typing state machine (ADR 0012), used by the
//! generator to label keys and by [`crate::log`] to assign word ids, so both
//! agree with what `nimk_engine` does with the same keys.

use nimk_types::{EditAction, KeyEvent, KeyKind, keycode};

/// `command_triggers` of `default.toml`.
const TRIGGERS: &str = "/@!";
/// `command_end_chars` of `default.toml`.
const END_CHARS: &str = " ";

/// Engine state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    Idle,
    Composing,
    Pending,
    Raw,
}

/// What a key is, for the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    Letter,
    Boundary(char),
    Backspace,
    /// Return, Tab, Escape, or any Cmd/Ctrl/Opt chord.
    CommitKey,
}

/// Classifies a scripted key; `None` for non key-down events.
pub(crate) fn classify(key: &KeyEvent) -> Option<Class> {
    if key.kind != KeyKind::KeyDown {
        return None;
    }
    if key.mods.is_shortcut() || matches!(key.keycode, keycode::RETURN | keycode::TAB) {
        return Some(Class::CommitKey);
    }
    if key.keycode == keycode::BACKSPACE {
        return Some(Class::Backspace);
    }
    let mut chars = key.chars.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(Class::Letter),
        (Some(c), _) => Some(Class::Boundary(c)),
        _ => None,
    }
}

/// What the engine does with a key, as an edit action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// A letter joins the word being composed.
    Append,
    /// Backspace removes a letter.
    Pop,
    /// The key ends the current word (the model decides its rendering).
    EndWord,
    /// The key goes straight to the app.
    Pass,
}

impl Step {
    /// The target edit for this step.
    pub(crate) const fn edit(self) -> EditAction {
        match self {
            Self::Append => EditAction::Append,
            Self::Pop => EditAction::Pop,
            Self::EndWord => EditAction::Commit,
            Self::Pass => EditAction::Pass,
        }
    }
}

/// The state machine, advanced one key at a time.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tracker {
    state: State,
}

impl Tracker {
    pub(crate) const fn new() -> Self {
        Self { state: State::Idle }
    }

    /// Whether the next key would be raw (command text).
    pub(crate) fn is_raw(&self) -> bool {
        self.state == State::Raw
    }

    /// Advances over one key.
    pub(crate) fn step(&mut self, class: Class) -> Step {
        use State as S;
        let (next, step) = match (self.state, class) {
            (S::Raw, Class::Letter | Class::Backspace) => (S::Raw, Step::Pass),
            (_, Class::Letter) => (S::Composing, Step::Append),
            (S::Raw, Class::Boundary(c)) => (
                if END_CHARS.contains(c) {
                    S::Idle
                } else {
                    S::Raw
                },
                Step::Pass,
            ),
            (S::Idle, Class::Boundary(c)) => (
                if TRIGGERS.contains(c) {
                    S::Raw
                } else {
                    S::Idle
                },
                Step::Pass,
            ),
            (S::Composing, Class::Boundary(_)) => (S::Pending, Step::EndWord),
            (S::Pending, Class::Boundary(_)) => (S::Idle, Step::Pass),
            (S::Composing | S::Pending, Class::Backspace) => (S::Composing, Step::Pop),
            (S::Idle, Class::Backspace) => (S::Idle, Step::Pass),
            (S::Composing, Class::CommitKey) => (S::Idle, Step::EndWord),
            (_, Class::CommitKey) => (S::Idle, Step::Pass),
        };
        self.state = next;
        step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &str) -> Vec<Step> {
        let mut t = Tracker::new();
        s.chars()
            .map(|c| {
                t.step(if c.is_ascii_alphabetic() {
                    Class::Letter
                } else if c == '\r' {
                    Class::CommitKey
                } else {
                    Class::Boundary(c)
                })
            })
            .collect()
    }

    fn ends(s: &str) -> usize {
        run(s).iter().filter(|&&x| x == Step::EndWord).count()
    }

    #[test]
    fn plain_words_and_pending_boundaries() {
        assert_eq!(ends("ab cd\r"), 2);
        assert_eq!(ends("ab, cd. \r"), 2);
        // The second boundary goes to the app; a trigger there is not raw.
        assert_eq!(ends("cd /usr/bin\r"), 3);
    }

    #[test]
    fn trigger_from_idle_is_raw_until_a_terminator() {
        assert_eq!(ends("/src/main.rs\r"), 0);
        assert_eq!(ends("@src/lib.rs ab\r"), 1);
        assert_eq!(ends("!ls -la\r"), 1);
        assert_eq!(ends("va/hoac\r"), 2);
    }
}
