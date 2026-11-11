# App compatibility: marked text

> **Status:** draft · **Updated:** 2026-11-11 · **Code:** `macos/NeuralIMK/InputController.swift`, `crates/nimk-engine/src/engine/flow.rs`

A double Shift tap flips the pending word in place, so it needs no `replacementRange` ([ADR 0011](adr/0011-pending-word-commit.md)).
The IME depends on only two app behaviors: `setMarkedText` and plain `insertText`.
Nothing below has been tested on a real app yet.

## What the IME needs from an app

| Behavior | Why |
| --- | --- |
| `setMarkedText` keeps showing the marked text up to one boundary character after the word | A pending word is the word plus one non-letter character (ADR 0012) |
| Return, Tab and arrow keys reach the app after the commit `insertText` | The engine commits the pending word, then passes the key on |
| `commitComposition` runs on a mouse click or focus loss | It settles the pending word; `deactivateServer` also commits |
| Two clients may activate back to back on a click (Electron) | A `Debouncer` merges context sends for 120 ms; a key arriving first sends at once |

## Risk by app type

These are expectations to confirm by hand, not measurements.

| Type | Apps | Risk | Confidence |
| --- | --- | --- | --- |
| Electron editors, Chromium browsers | VS Code, Cursor, Chrome | Each `flagsChanged` arrives twice (the engine drops the copy); marked text in custom editors may flicker or be underlined | Verified for `flagsChanged` (real log); unverified for marked text |
| Terminals | Ghostty, Terminal.app, iTerm2, WezTerm, Kitty, Alacritty, Warp | Preedit may render differently; marked text across spaces needs a check | Unverified |
| `NSTextView` and Cocoa apps | TextEdit, Notes, Pages, Messages, Xcode | Expected to work | Unverified |
| Other native editors | Zed | Implements its own `NSTextInputClient` | Unverified |

## Terminal key bindings bypass the IME

A terminal key binding runs before the input method. If it is bound to a key that should commit the word (Return, Shift+Return, Tab), the IME never sees that key, and the word being typed stays as marked text.

```
Shift+Return --> WezTerm: matches config.keys?
                   yes --> SendString to the pty     (IME never sees it)
                   no  --> IME commits the word --> WezTerm encodes the key
```

Seen on 2026-11-09 with WezTerm. Its `config.keys` mapped `Shift+Enter` to `SendString '\x1b[13;2u'`, a binding often added for Claude Code and Codex. The real log had 0 Shift+Return keys from WezTerm. The fix was to delete the binding and keep `config.enable_kitty_keyboard = true`. WezTerm then sends `\x1b[13;2u` itself to apps that enable the kitty protocol, after the IME has committed the word. With that change, the real log shows Shift+Return in WezTerm committing the word (`boundary: "return"`).

## Text access for caret flip

Measured on 2026-11-09..11 with a throwaway probe (branch `spike/caret-flip`) for the caret flip ([ADR 0015](adr/0015-caret-flip.md)). Editors give the IME the text before the caret and accept a range replacement; terminals give nothing, but pass control bytes through.

The probe ran on a double Shift with no marked text. It read the text around the caret through `IMKTextInput`, then replaced the word there with its upper-case form.

| Client | `selectedRange` | Read text before the caret | Replace with `insertText(_:replacementRange:)` | Note |
| --- | --- | --- | --- | --- |
| VS Code (Electron) | Yes | Yes | Works; read-back matched; caret kept | Cursor shares the stack (not measured) |
| Zed | Yes | Yes | Works; read-back matched | The caret moves to the end of the replaced word |
| Orca, Edge (Chromium) | Yes | Only for ranges that do not run past the end of the text; `length()` reports 0 | Not exercised | Same engine as VS Code |
| WezTerm with Claude Code | `NotFound` | No | Not possible | Inserted DEL erased a character, `ESC [ D` moved left, then `.` was inserted |
| TextEdit, RustRover, iTerm, Terminal.app, Codex.app | Not measured | | | |

Accessibility API and synthesized keys were not measured; the probe ran without the Accessibility permission.

## Manual test plan

Run it on an installed build with pretrained models ([07](07-build-and-install.md)). Record results in the last table.

| # | Case | Expected |
| --- | --- | --- |
| 1 | `has ` then double Shift | Marked text flips `has ` to `há ` (or back); nothing inserted yet |
| 2 | `has ` then a letter | `has ` (or `há `) is inserted; the letter starts a new word |
| 3 | `has. ` then double Shift | `há. `; punctuation unchanged |
| 4 | `has ` then Return | `has ` inserted; Return reaches the app |
| 5 | `has ` then `@` or a digit | `has ` inserted; the character reaches the app |
| 6 | `has ` then Backspace twice | Back to marked `has`, then a letter is deleted |
| 7 | `has ` then click elsewhere | The word is inserted at the old place |
| 8 | Double Shift right after clicking into a field, nothing typed | Mode flips; overlay appears at the caret (or the mouse if the app gives no caret) |
| 9 | VS Code: double Shift | Exactly one flip, even though the event arrives twice |

| App | Bundle id | Result | Date |
| --- | --- | --- | --- |
| (none yet) | | | |
