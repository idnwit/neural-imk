# 0015. Caret flip: fix an earlier word in place

> **Status:** accepted · **Date:** 2026-11-11 · **Code:** `crates/nimk-engine/src/engine/retro.rs`, `macos/Shared/CaretFlip.swift`, `macos/NeuralIMK/CaretFlipper.swift` · **Extends:** [0011](0011-pending-word-commit.md), [0014](0014-transform-hook-pipeline.md) · **Evidence:** [09, text access for caret flip](../09-app-compat.md#text-access-for-caret-flip)

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
| Text client | `selectedRange().location != NSNotFound` and `retro_flip` | Read `string(from:actualRange:)` for windows that end at the caret (40, 20, 10, 5 UTF-16 units) and that start at it (20 down to 1). A selection made only of letters is the target. Otherwise the letter run containing the caret, or ending at most 3 non-letters before it | Caret after the word: `insertText(newWord + gap, replacementRange: wordStart..<caret)`, where `gap` is the text between the word and the caret, so the caret stays put. Caret inside the word: replace the word; the caret ends after it |
| Terminal line | `selectedRange` is `NotFound` and `retro_flip_terminal` (on for the `terminal` and `companion` categories) | The line tracker below | `ESC [ C` x (word end - caret) if the caret is inside the word, or `ESC [ D` x gap if after it; `DEL` x word length; the new word; `ESC [ C` x gap. Each unit is its own `insertText`, 12 ms apart, so programs that tell typing from pasting by chunk size see keystrokes |
| None | Any other client | - | The hotkey only switches the sticky mode |

### Line tracker

A pure Swift type mirrors the terminal's input line from the keys the IME already sees.

| Event | Effect |
| --- | --- |
| Text the IME inserts, printable keys passed through (raw mode included) | Insert at the caret |
| Backspace, forward Delete | Delete before or at the caret |
| Left, Right | Move one character |
| Cmd+Left, Cmd+Right, Home, End, Ctrl+A, Ctrl+E | Line start or end, if the whole line is known |
| Return, keypad Enter | Start a new, fully known line |
| Up, Down, Tab, Esc, PageUp, PageDown, mouse click (`commitComposition`), Option/Ctrl/Cmd chords (word moves, kills, paste) | Invalidate until the next Return |
| Focus change (`activateServer`) | Start a window with unknown edges |

The tracker knows either a whole line (since a Return) or a window around the caret with unknown text on both sides. In a window, moving or deleting past its edge invalidates the line, and a word touching an edge is refused because it may continue outside.
A flip on an invalid line does nothing new. Vim normal mode, shell history and completion menus all invalidate the line, so they stay safe.

### Engine

- Module `engine/retro.rs`: a ring of the last 200 committed words `{word_id, bundle_id, raw, vi, lang}`, filled at `WordCommit`, not on the per-key path. Words are matched per app, not per field: Chromium and Electron change the field id on every click.
- A hotkey press that found no word records `UndoSwitch { prev mode, held-back focus label }`. The next event of any kind settles it (writes the focus-prior label); until then a caret flip may undo it.
- `Engine::retro_flip(observed, timestamp) -> Option<RetroFlip>` works only while that undo is open. It looks up the most recent word in the focused app whose rendering in the app equals `observed`, restores the sticky mode, and updates the remembered rendering, so a second flip toggles back.
- Found: the replacement is the other rendering. The engine logs `manual_switch { scope: retro }` and a `correction` label, weight 1.0, through `Labeler::refinalize`. The labeler now keeps the steps of its last 32 finalized words (was 4); an older word gets the label record with nothing to train on. A label equal to the current ground truth is ignored.
- Not found: a word with Vietnamese letters becomes its Telex raw (`nimk_telex::encode`). A plain ASCII word becomes its Telex rendering when that is a valid syllable. No label is written.
- `None`: the press's plain mode switch stands.
- FFI: `nimk_retro_flip(core, observed, timestamp, *to) -> const char*` (null when there is nothing to flip) and `nimk_retro_policy(core) -> u8` (`NIMK_RETRO_TEXT | NIMK_RETRO_TERMINAL`).
- Replay: a `manual_switch { scope: retro }` record calls `Engine::replay_retro` (undo the press); its label is injected as usual, and `inject_label` uses `refinalize` so a correction after an implicit label is kept.

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
| Event | `SwitchScope::Retro` (`retro`), additive; `SCHEMA_VERSION` stays 1 |
| Config | `retro_flip = true` (text clients); `retro_flip_terminal = false`, true in the `terminal` and `companion` categories; both overridable per `[[app_category]]` |
| Overlay | Orange badge `→ EN test` with the new rendering; the status item shows the restored mode |
| Simulator | Companion app section **Thử sửa từ**: an editor and a terminal-like client ([06](../06-gui.md#caret-flip-simulator)) |

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
| History and `retro_flip` (`crates/nimk-engine/tests/retro.rs`) | Flip and flip back with labels, most recent match, only right after a word-less press, fallback without label, other app not matched, nothing to flip keeps the switch, focus label dropped, replay undo, policy per category |
| Word finding, line tracker, keystroke plan (`macos/tests/app`) | Caret inside the word, after a gap, gap too long, Vietnamese letters; tracker edges, invalidation, Return; the plan replayed through `FakeTerminal` |
| Manual | VS Code, Cursor, Zed, Edge, TextEdit, RustRover; Claude Code and Codex in WezTerm, iTerm and Terminal.app; vim insert vs normal mode |
