# 0008. Double-tap Shift and retroactive correction

> **Status:** accepted for the double-Shift hotkey; the retroactive `replacementRange` correction is superseded by [0011](0011-pending-word-commit.md) · **Date:** 2026-11-08 · **Supersedes:** [0003](0003-shift-tap-hotkey.md)

## Context

ADR 0003 used a single Shift tap. A switch after a commit only changed the mode for the next word and did not rewrite text. Two problems:

- A single Shift tap is easy to press by accident.
- Users usually notice a wrong word only after typing the space. The hotkey could not fix that word, so the label was vague: it changed the next word's mode instead of confirming the last word was wrong.

## Decision

- **Hotkey:** double-tap Shift. Two press-and-release taps of either Shift key, each alone and held <= `shift_tap_secs` (0.3 s), with releases <= `double_shift_secs` (0.4 s) apart. Any other key or modifier in between cancels. A single tap does nothing. Ctrl+Space stays as a fallback.
- **Semantics by timing:**

| When pressed | Action | Label |
| --- | --- | --- |
| Mid-word | Flip the current word's rendering, lock until word end | Correction, weight 1.0 |
| Right after a commit, only printable boundary chars typed since | **Replace the just-committed word** with the other rendering, keep the boundary | Correction for that word, weight 1.0 |
| Later (Return, Tab, other key forgot the boundary) but a commit exists since focus | Change sticky mode only | None |
| No commit since focus | Change the prior mode | Correction for the focus prior |

- **Engine** returned `EngineOutput.replace = Some(Replace { delete_before, text })` (UTF-16 units); **Swift** applied it with `insertText(text, replacementRange:)`.
- **Unsupported apps only flip mode and learn.** Swift re-read the text before the caret and skipped the edit on mismatch; no fake `CGEvent` backspaces, so no Accessibility permission. Support was cached per bundle id.
- **Learning:** a retroactive fix produces a correction for every step of that word. Repeated taps toggle the rendering, each producing a label. Only the just-committed word can be fixed.

## Consequences

- Clear online signal: each retroactive fix is a certain label for exactly one word.
- Every app had to be checked for `replacementRange` support.
- The engine had to remember the last committed word and its boundary chars.
- Note: 0011 removed `replacementRange` editing, `EngineOutput.replace` and the compat cache. The double-Shift hotkey and the "flip, then learn" rule remain. See [09](../09-app-compat.md).
