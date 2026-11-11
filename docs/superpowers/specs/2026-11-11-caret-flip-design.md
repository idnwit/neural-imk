# Caret flip: fix an earlier word in place — design

> **Status:** proposed · **Date:** 2026-11-11 · **Spike:** branch `spike/caret-flip` (throwaway probe, not merged)

## Goal

Typing fast, the user notices a wrong rendering only a few words later, after the word has left marked text. Today only the pending word can be fixed ([02](../../02-keyboard-io.md#hotkey-semantics): "Text already inserted into the app is never touched"). With caret flip the user moves the caret onto the wrong word (click or arrow keys) and presses the flip hotkey (double Shift, or Ctrl+Space if enabled): the word switches between its Vietnamese and English rendering in place, and the model learns from it as a correction.

Success criteria:

1. In VS Code, Cursor, Zed and Chromium text fields, a flip with the caret on or right after a committed word replaces exactly that word and leaves the caret where it was.
2. In Claude Code / Codex running in WezTerm, the same works for words typed on the current line, navigated with the keyboard.
3. Everywhere else the flip keeps today's behaviour (sticky mode switch); no text is ever corrupted.
4. The flip writes a `correction` label for the word when the engine still knows it, so online learning benefits.

## Non-goals

- Accessibility (AX) API or synthesized `CGEvent` keystrokes: both need the Accessibility permission; the spike shows the two strategies below cover the target apps without it.
- Fixing words in text the IME never saw (pasted text, other sessions) beyond the stateless fallback in 3.3.
- Multi-word or selection-spanning flips.

## Spike findings (2026-11-09..11)

The probe ran on double Shift with no marked text, read text around the caret via `IMKTextInput`, and replaced the word with its upper-case form.

| Client | `selectedRange` | Read text | Replace | Notes |
| --- | --- | --- | --- | --- |
| VS Code (Electron) | yes | yes | `insertText(_:replacementRange:)` ok, read-back matched, caret kept | Cursor shares the stack |
| Zed | yes | yes | ok, read-back matched | caret moved to the end of the replaced word |
| Orca / Edge (Chromium) | yes | only ranges that do not run past the end of the text; `length()` reports 0 | not exercised | same stack as VS Code |
| WezTerm (+ Claude Code) | `NotFound` | no | — | control bytes inserted as text reach the program: DEL erased a character, `ESC [ D` moved left, then `.` was inserted |
| TextEdit, RustRover, iTerm, Terminal.app, Codex.app | not measured | | | to verify during implementation |

Usage note: of 48 real flips in the period, 47 hit the pending word (already handled). Caret flip targets the remaining, later-noticed errors.

## Design

### 1. Trigger

The engine handles the hotkey exactly as today. When the flip had nothing to flip — `SwitchScope::AfterCommit` with no pending word, or `BeforeTyping` after at least one commit — the Swift shim tries a caret flip. If the caret flip succeeds, `Engine::retro_flip` (3.2) reverts the sticky-mode change of that hotkey press, so a retro fix does not also switch the next word's language.

### 2. Two strategies, chosen per client

| Strategy | When | How it finds the word | How it replaces |
| --- | --- | --- | --- |
| Text client | `selectedRange().location != NSNotFound` | Read `string(from:actualRange:)` for windows that end at the caret (40, 20, 10, 5 UTF-16 units; never past the caret, which Chromium rejects). A non-empty selection is the target itself. Otherwise the letter run containing the caret, or the run ending at most 3 non-letters before it | `insertText(newWord + gap, replacementRange: wordStart..<caret)` where `gap` is the text between the word's end and the caret. The insertion ends at the caret, so the caret stays put (Zed otherwise moves it) |
| Terminal line | `selectedRange` is `NotFound` and the focused app's category is `terminal` | The line tracker (2.1) knows the text typed since the last line boundary and the caret offset in it | One `insertText` of: `ESC [ C` × (word end − caret) if the caret is inside the word, `DEL` × word length, the new word, `ESC [ D` × the same right-moves, restoring the caret |
| None | anything else | — | Today's behaviour; overlay "Không sửa được ở đây" |

#### 2.1 Line tracker (Swift, pure, in `macos/Shared`)

Mirrors the terminal's current input line from what the IME already sees:

| Event | Effect |
| --- | --- |
| Text the IME inserts (`NimkOut.commit`, passed-through printable keys) | Insert at the caret |
| Backspace | Delete before the caret |
| ← / → | Move by one character |
| Opt+← / Opt+→ | Move by one word (whitespace-delimited) |
| Cmd+← / Cmd+→, Home / End, Ctrl+A / Ctrl+E | Line start / end |
| Return, ↑ / ↓, Tab, Esc, focus change, mouse click (`commitComposition`), any other Ctrl/Cmd chord, raw mode | Invalidate: no terminal caret flip until the next line boundary rebuilds it |

A flip while the tracker is invalid does nothing beyond today's behaviour. This keeps vim normal mode, shell history recall and completion menus safe: they all invalidate the line.

### 3. Engine

#### 3.1 Word history

New engine module `history` (independent like the others; wired by `engine.rs`): per focus id, a ring of the last 200 committed words `{word_id, raw, vi, en, committed: Lang}`, filled at `WordCommit`. Not on the per-key path.

#### 3.2 API

```rust
pub struct RetroFlip { pub replacement: String, pub to: Lang, pub word_id: Option<u64> }

impl Engine {
    /// The user flipped `observed` (the word text read from the client or
    /// the line tracker) in the focused field. Returns its other rendering.
    pub fn retro_flip(&mut self, observed: &str, timestamp: f64) -> Option<RetroFlip>;
}
```

Lookup: the most recent history entry of the focused field whose committed rendering equals `observed`. It first restores the sticky mode the preceding hotkey press changed. Found: the replacement is the other rendering (`vi` ↔ `raw`); the engine logs `manual_switch { scope: retro, word_id, to }` and a `label` with `source = correction`, weight 1.0, through `Labeler::refinalize` when the word is still in its cache (statistics amend the earlier record), or as a new label otherwise.

FFI: `nimk_retro_flip(core, observed, timestamp, out_text) -> u8` (target language or `NIMK_MODE_UNCHANGED`); the string lives until the next call.

#### 3.3 Stateless fallback

Not in history (older session, pasted text): a word with Vietnamese letters becomes its Telex raw (`nimk_telex::encode`), a plain ASCII word becomes `TelexTransform` of itself when that is a valid syllable. No label (no word id).

### 4. Event schema

`SwitchScope::Retro` (additive, `snake_case` `retro`). `SCHEMA_VERSION` stays 1; replay treats it like `after_commit` with a word id (the label carries the truth).

### 5. Config

| Key | Default | Meaning |
| --- | --- | --- |
| `retro_flip` | `true` | Enable caret flip for text clients |
| `retro_flip_terminal` | `true` | Enable the terminal-line strategy |

Per `[[app_category]]` overrides allowed, like other live params.

### 6. Feedback

Overlay shows `→ việt` (orange, manual). Failure to find a word shows nothing new; the hotkey keeps switching the sticky mode as today.

## Testing

| Area | Test |
| --- | --- |
| History and `retro_flip` | Engine tests: lookup per field, most recent match, label via refinalize (stats amend, no double count) and late label, fallback without label, `retro` record |
| Word finding | Pure Swift (`macos/Shared`): caret inside word, after trailing spaces, selection, punctuation, UTF-16 with Vietnamese letters |
| Line tracker | Pure Swift: every event row of 2.1, invalidation cases, keystroke plan (right-moves, DEL count, left-moves) |
| Replacement range | Pure Swift: `wordStart..<caret` and gap text keep the caret |
| Manual matrix | VS Code, Cursor, Zed, Edge, TextEdit, RustRover; Claude Code and Codex in WezTerm, iTerm, Terminal.app; vim insert vs normal mode (must not corrupt) |

## Risks

| Risk | Mitigation |
| --- | --- |
| A terminal program interprets DEL / arrows differently (vim normal mode, custom TUIs) | Tracker invalidates on anything it does not model; `retro_flip_terminal` off switch; per-category override |
| Kitty keyboard protocol programs expecting CSI-u for arrows | Legacy `ESC [ C/D` worked in the spike (WezTerm with kitty protocol on); verify iTerm and Terminal.app |
| Chromium replace not yet exercised | First implementation task verifies it in Edge before building on it |
| Undo groups the replacement separately from typing | Acceptable; one Cmd+Z restores the old word |
| A word appears several times in the line or field | Text client uses the exact range at the caret; history lookup uses the most recent match only for the replacement text, never for the position |
