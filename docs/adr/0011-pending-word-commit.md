# 0011. Pending word: commit when the next word starts

> **Status:** accepted; the boundary part is superseded by [0012](0012-unified-boundary-commit.md) · **Date:** 2026-11-08 · **Supersedes:** the `replacementRange` correction of [0008](0008-double-shift-retro-correction.md)

## Context

ADR 0008 fixed committed words with `insertText(_:replacementRange:)`, plus pre-check, post-check and a per-app verdict cache.
Many apps (terminals, Firefox, Electron) ignore or mishandle `replacementRange`, so the feature worked in only some apps and needed a lot of guard code.

## Decision

- When the model ends a word (`Commit` on space or punctuation), the engine inserts nothing. The chosen rendering plus the boundary char stay in marked text ("pending").
- A pending word is really committed when the user starts the next word (a letter), or on navigation keys, Return, Tab, Escape, shortcuts, non-letter characters, `commitComposition`, focus loss, and model change or reset.
- Space and punctuation append to the pending boundary; Backspace removes boundary chars, and when none remain the word becomes the current word again.
- Double Shift on a pending word flips its rendering inside the marked text, records a `Correction` label and sets the sticky mode.
- `word_commit` and the label are written at the real commit; a flipped word gets `Correction` instead of `Implicit`.
- Structural mask: only a letter may `Append` on an empty buffer. Non-letters with nothing composing or pending do not call the model; they only write a `key` record.
- Removed: `EngineOutput.replace`, the last-word memory, the `replace_*` C ABI fields, Swift pre/post-checks and `ReplaceCompat`, `replace-compat.json`, two menu items and the red overlay.

The full behavior table and state machine are in [02](../02-keyboard-io.md).

## Consequences

- No dependence on `replacementRange`; any app that supports marked text can flip a word ([09](../09-app-compat.md)).
- Boundary chars sit in marked text, so the app sees them one beat late. Return still reaches the app after the commit, so sending messages is unchanged.
- Log: `word_commit` arrives later than the boundary key; replay must commit the pending word before loading its label (`Engine::inject_label` does this).
- Training data: non-letters with an empty buffer, and boundaries appended to a pending word, produce no `Pass` samples; the `Commit` step of a word that is backspaced into is dropped from the label.
- A double tap after typing past the word only changes the sticky mode; the inserted word cannot be fixed.
- Note: the "append many boundary chars" behavior was replaced by exactly one in 0012.
