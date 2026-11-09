//! The typing state machine of ADR 0012: four states, one transition
//! table. Everything here is pure; `flow.rs` performs the effects.

/// Where typing is. Derived from the engine's buffers, never stored twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum State {
    /// Nothing marked.
    Idle,
    /// A word is being composed.
    Composing,
    /// A word and exactly one boundary character are marked text.
    Pending,
    /// A command is being typed: keys go straight to the app.
    Raw,
}

/// What a key is, for the transition table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Class {
    /// A language character; the buffer is not full.
    Letter,
    /// A language character but the word buffer is full.
    LetterFull,
    /// A printable non-language character that starts raw mode from Idle.
    Trigger,
    /// Any other printable non-language character.
    Boundary,
    /// Plain Backspace.
    Backspace,
    /// A key that commits: commit keycode, Cmd/Ctrl/Opt chord, or text the
    /// buffer cannot hold.
    Commit,
    /// A key that ends raw mode (only produced in [`State::Raw`]).
    Terminator,
    /// Anything else (bare modifiers, unlisted function keys).
    Ignored,
}

/// What to do with the key (state change included).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Move {
    /// The key goes to the app; state unchanged.
    Pass,
    /// The key goes to the app and raw mode begins.
    EnterRaw,
    /// The key goes to the app and raw mode ends.
    ExitRaw,
    /// Model step: append the letter (Idle starts a word).
    Append,
    /// Model step: Backspace pops a letter.
    Pop,
    /// Model step: the word ends with this character, which stays marked.
    EndWordPending,
    /// Model step: the word ends and is committed now; the key goes on.
    CommitWord,
    /// Commit the pending word, then the letter starts a new word.
    CommitThenAppend,
    /// Commit the pending word; the key goes to the app; Idle.
    CommitThenPass,
    /// Backspace drops the boundary character and reopens the word.
    PendingBackspace,
}

/// The transition table of ADR 0012.
pub(super) fn transition(state: State, class: Class) -> Move {
    use Class as C;
    use Move as T;
    use State as S;
    match (state, class) {
        (_, C::Ignored) => T::Pass,
        (S::Idle, C::Letter | C::LetterFull) => T::Append,
        (S::Idle, C::Trigger) => T::EnterRaw,
        (S::Idle, _) => T::Pass,
        (S::Composing, C::Letter) => T::Append,
        (S::Composing, C::Trigger | C::Boundary) => T::EndWordPending,
        (S::Composing, C::Backspace) => T::Pop,
        (S::Composing, C::LetterFull | C::Commit | C::Terminator) => T::CommitWord,
        (S::Pending, C::Letter | C::LetterFull) => T::CommitThenAppend,
        (S::Pending, C::Backspace) => T::PendingBackspace,
        (S::Pending, _) => T::CommitThenPass,
        (S::Raw, C::Terminator) => T::ExitRaw,
        (S::Raw, _) => T::Pass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_matches_adr_0012() {
        use Class as C;
        use Move as T;
        use State as S;
        let rows = [
            (S::Idle, C::Letter, T::Append),
            (S::Idle, C::Trigger, T::EnterRaw),
            (S::Idle, C::Boundary, T::Pass),
            (S::Idle, C::Backspace, T::Pass),
            (S::Idle, C::Commit, T::Pass),
            (S::Composing, C::Letter, T::Append),
            (S::Composing, C::Boundary, T::EndWordPending),
            (S::Composing, C::Trigger, T::EndWordPending),
            (S::Composing, C::Backspace, T::Pop),
            (S::Composing, C::Commit, T::CommitWord),
            (S::Composing, C::LetterFull, T::CommitWord),
            (S::Pending, C::Letter, T::CommitThenAppend),
            (S::Pending, C::Boundary, T::CommitThenPass),
            (S::Pending, C::Trigger, T::CommitThenPass),
            (S::Pending, C::Backspace, T::PendingBackspace),
            (S::Pending, C::Commit, T::CommitThenPass),
            (S::Raw, C::Letter, T::Pass),
            (S::Raw, C::Backspace, T::Pass),
            (S::Raw, C::Terminator, T::ExitRaw),
            (S::Raw, C::Commit, T::Pass),
            (S::Raw, C::Ignored, T::Pass),
        ];
        for (s, c, want) in rows {
            assert_eq!(transition(s, c), want, "{s:?} + {c:?}");
        }
    }
}
