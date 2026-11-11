# 0015. Caret flip: fix an earlier word in place

> **Status:** proposed · **Date:** 2026-11-11 · **Extends:** [0011](0011-pending-word-commit.md), [0014](0014-transform-hook-pipeline.md) · **Evidence:** [09, text access for caret flip](../09-app-compat.md#text-access-for-caret-flip)

## Context

Only the pending word can be flipped today ([02, hotkey semantics](../02-keyboard-io.md#hotkey-semantics)). A user typing fast often notices a wrong rendering two or three words later, when the word is already in the app. Then the only fix is to delete and retype it.

The user wants to move the caret onto the wrong word, with a click or the arrow keys, and press the flip hotkey. A spike (2026-11-09..11) measured what an input method can reach without extra permissions ([09](../09-app-compat.md#text-access-for-caret-flip)):

- VS Code and Zed let the IME read the text before the caret and replace a range with `insertText(_:replacementRange:)`.
- Chromium fields return text only for ranges that end at or before the end of the text.
- Terminals expose no text, but control bytes the IME inserts as text reach the program: DEL erases a character and `ESC [ D` moves left.

Of 48 real flips in that period, 47 hit the pending word. Caret flip covers the rest: errors noticed late.

## Decision

A flip hotkey that finds no pending word tries a **caret flip**: it switches the word at the caret between its Vietnamese and English rendering in place, and labels it as a correction.

```text
 double Shift, no pending word
          |
          v
 +-------------------+  selectedRange known  +---------------------------+
 | Swift: pick client| --------------------> | text client: read before  |
 | strategy          |                       | caret, replace a range    |
 +-------------------+                       +---------------------------+
          | NotFound + terminal app                     |
          v                                             v
 +-------------------+                       +---------------------------+
 | terminal line:    | ---- word text -----> | engine: retro_flip(word)  |
 | tracked from keys |                       | -> other rendering, label |
 +-------------------+                       +---------------------------+
```

Swift finds the word and performs the edit. The engine decides the replacement and records the label.

### Strategies

| Strategy | When | Find the word | Replace it |
| --- | --- | --- | --- |
| Text client | `selectedRange().location != NSNotFound` | Read `string(from:actualRange:)` for windows that end at the caret (40, 20, 10, 5 UTF-16 units). A non-empty selection is the target. Otherwise the letter run containing the caret, or ending at most 3 non-letters before it | `insertText(newWord + gap, replacementRange: wordStart..<caret)`, where `gap` is the text between the word and the caret. The insertion ends at the caret, so the caret stays put |
| Terminal line | `selectedRange` is `NotFound` and the app category is `terminal` | The line tracker below | One `insertText` of `ESC [ C` x (word end - caret) if the caret is inside the word, `DEL` x word length, the new word, then `ESC [ D` x the same count |
| None | Any other client | - | Today's behaviour: the hotkey only switches the sticky mode |

### Line tracker

A pure Swift type mirrors the terminal's input line from the keys the IME already sees.

| Event | Effect |
| --- | --- |
| Text the IME inserts, printable keys passed through | Insert at the caret |
| Backspace | Delete before the caret |
| Left, Right | Move one character |
| Opt+Left, Opt+Right | Move one whitespace-delimited word |
| Cmd+Left, Cmd+Right, Home, End, Ctrl+A, Ctrl+E | Line start or end |
| Return, Up, Down, Tab, Esc, focus change, mouse click, other Ctrl or Cmd chords, raw mode | Invalidate until the next line boundary |

A flip on an invalid line does nothing new. Vim normal mode, shell history and completion menus all invalidate the line, so they stay safe.

### Engine

- New module `history`: per focus id, a ring of the last 200 committed words `{word_id, raw, vi, en, committed}`. It is filled at `WordCommit` and is not on the per-key path.
- `Engine::retro_flip(observed, timestamp) -> Option<RetroFlip>` first restores the sticky mode the preceding hotkey press changed. It then looks up the most recent word in the focused field whose committed rendering equals `observed`.
- Found: the replacement is the other rendering. The engine logs `manual_switch { scope: retro }` and a `correction` label, weight 1.0. The label goes through `Labeler::refinalize` while the word is cached (statistics amend the earlier record), else as a new label.
- Not found: a word with Vietnamese letters becomes its Telex raw (`nimk_telex::encode`). A plain ASCII word becomes its Telex rendering when that is a valid syllable. No label is written.
- FFI: `nimk_retro_flip(core, observed, timestamp, out_text) -> u8`, returning the target language or `NIMK_MODE_UNCHANGED`.

```rust
pub struct RetroFlip {
    pub replacement: String,
    pub to: Lang,
    pub word_id: Option<u64>,
}
```

### Schema, config and feedback

| Item | Value |
| --- | --- |
| Event | `SwitchScope::Retro` (`retro`), additive; `SCHEMA_VERSION` stays 1; replay treats it like `after_commit` with a word id |
| Config | `retro_flip = true` (text clients), `retro_flip_terminal = true` (terminal line); both overridable per `[[app_category]]` |
| Overlay | `-> viet` style badge with the new rendering, orange (manual) |

## Consequences

- Late errors are fixed with the same hotkey, and the model still learns from them.
- No Accessibility permission and no synthesized `CGEvent`s are needed.
- The terminal strategy depends on programs reading DEL and legacy arrow sequences. It was verified only in WezTerm with Claude Code, with the kitty keyboard protocol on. iTerm and Terminal.app need checking.
- Replacement in Chromium fields was not exercised in the spike. The first implementation step verifies it in Edge.
- One undo step restores the old word; undo does not merge it with the typing before it.
- A word typed outside the IME (pasted, another session) gets the stateless fallback and no label.

### Testing

| Area | Test |
| --- | --- |
| History and `retro_flip` | Per-field lookup, most recent match, label via refinalize without double counting, late label, fallback without label, `retro` record |
| Word finding | Caret inside the word, after trailing spaces, selection, punctuation, UTF-16 with Vietnamese letters |
| Line tracker | Every event row above, invalidation, and the keystroke plan |
| Manual | VS Code, Cursor, Zed, Edge, TextEdit, RustRover; Claude Code and Codex in WezTerm, iTerm and Terminal.app; vim insert vs normal mode |
