# Keyboard I/O via IMK

> **Status:** draft · **Updated:** 2026-11-09 · **Code:** `crates/nimk-engine/src/{composer,hotkey,engine}.rs`, `crates/nimk-engine/src/engine/{flow,machine,switch}.rs`, `macos/NeuralIMK/*.swift`

The IME runs a 4-state machine: `Idle`, `Composing`, `Pending`, `Raw`. The word being typed is marked text. A finished word ("pending") holds one boundary character until the next key ([ADR 0012](adr/0012-unified-boundary-commit.md)).
Double Shift flips the rendering of the composing or pending word inside the marked text, so no `replacementRange` is needed ([ADR 0011](adr/0011-pending-word-commit.md)).

## One key, end to end

Swift only forwards each `NSEvent` to Rust and applies the returned `EngineOutput`. All decisions live in `nimk-engine`.

```text
+-----------+ handle(_:client:) +----------------------+  nimk_key   +-------------------+
| App focus | ----------------> | InputController.swift| ----------> | Engine::on_key    |
+-----------+                   +----------------------+             | hotkey -> model   |
      ^                                    |                         | -> apply -> log   |
      |     insertText / setMarkedText     |  NimkOut (EngineOutput) +-------------------+
      +------------------------------------+ <-------------------------------+
```

Swift applies `commit` first, then `marked`. `consumed = false` returns the key to the app untouched.

| Field | What Swift does |
| --- | --- |
| `consumed` | `false`: `handle` returns `false` and the app gets the key |
| `commit` | `client.insertText(_:replacementRange:)` with `NSNotFound`, which also clears marked text. It holds the pending word plus boundary, or the composing word |
| `marked` | `client.setMarkedText(_:selectionRange:replacementRange:)`; `Some("")` clears the preedit |
| `mode` | `Some(_)` updates the status item. This is the sticky mode (the mode the next word starts in) |
| `overlay` | Shows the overlay at the caret when the sticky mode changed: `Auto` (model, at word end) or `Manual` (hotkey). A focus prior changes the mode but shows no overlay ([06](06-gui.md)) |

## Composer and edits

The composer is a dumb buffer. The key decides the `EditAction` through the state machine, the model picks the rendering (`p_vi`), and the engine applies both mechanically.
The buffer keeps `raw`, `vi = telex(raw)` and `en = raw`, up to 64 keys (`MAX_RAW`).

| `EditAction` | Effect | Output |
| --- | --- | --- |
| `Append` | Add the key's character to `raw`, recompute `vi` and the `valid_vi` / `viable_vi` flags, swallow the key | `marked` = shown rendering |
| `Pop` | Drop the last key; an empty buffer resets the word | `marked` = new text, or `""` |
| `Commit` | End the word. A non-letter becomes the pending word's boundary. A commit key (Return, Tab, shortcut) commits at once and goes on to the app | `marked` = word + boundary, `consumed = true`; or `commit = Some(text)`, `consumed = false` |
| `Pass` | Return the key to the app; the buffer stays | `marked` changes only if the rendering changed |

`ActionMask` has three permissions, derived from the state table. The model never picks the edit.

| Permission | When |
| --- | --- |
| `append` | A letter (also in `Idle` or `Pending`, where it opens a new word) and the buffer is not full |
| `pop` | Backspace while `Composing` |
| `commit` | A non-letter, commit key, shortcut, or a letter with a full buffer, while `Composing` |
| (none) | `Idle` with a non-letter, and every `Raw` key: the model is skipped and only a `key` record is written |

The boundary is logged in `word_commit.boundary`: `space`, `return`, `tab`, `punct`, `navigation` (arrows, Escape), `shortcut` (any of Cmd/Ctrl/Option), `external` (mouse click, focus loss, `commit_pending`).
`FlagsChanged` never reaches the model or the `recent` window; only the hotkey detector sees it.

Telex is `nimk-telex` ([10](10-telex.md)). It is a feature source, not a hard rule ([04](04-engine-ml.md)).
`valid_vi` means a full syllable; `viable_vi` means a syllable or a prefix of one (`viê` is viable, not yet valid).

## Hotkey

The default hotkey is two quick Shift taps. Ctrl+Space is an optional fallback for apps that swallow `flagsChanged`; it is off by default (`ctrl_space = true` enables it) because it collides with the macOS input-source shortcut and IDE autocomplete.
Cmd+Space is not used because the system takes it before the IME ([ADR 0008](adr/0008-double-shift-retro-correction.md); [ADR 0011](adr/0011-pending-word-commit.md) replaced its retro-correction).

`recognizedEvents(_:)` must return `[.keyDown, .flagsChanged]` so the IME sees modifier events. Detection is in Rust and uses `KeyEvent::timestamp`.
A tap is one Shift (left or right) pressed and released alone within `shift_tap_secs`. Two taps whose releases are within `double_shift_secs` emit `DoubleShift` at the second release.

```text
          Shift down, alone                       Shift up, held <= tap_secs
 +------+ ------------------> +-------+  ---------------------------------> +---------+
 | Idle |                     | Down1 |                                      | Between |
 +------+ <-----------------  +-------+                                      +---------+
   ^  ^     Shift up, too long    |                                     Shift down, |alone
   |  |                           | other key / modifier                            v
   |  |                           v                                            +---------+
   |  |   all Shift up       +-----------+     other key / modifier           | Down2   |
   |  +--------------------- | Cancelled | <----------------------------------+---------+
   |                         +-----------+                                         |
   |   Shift up, held <= tap_secs and (release - first release) <= double_secs     |
   +---------------------------- emit DoubleShift <-------------------------------+
        (too slow: release becomes a new first tap -> Between)
```

Any other key or modifier between the first press and the second release cancels the gesture. A second tap that is too slow starts a new pair.

| Parameter | Key in `config.toml` | Default |
| --- | --- | --- |
| Tap length | `shift_tap_secs` | `0.3` |
| Gap between releases | `double_shift_secs` | `0.4` |
| Double Shift on | `double_shift` | `true` |
| Ctrl+Space on | `ctrl_space` | `false` (independent of `double_shift`) |
| Hotkeys act and implicit labels | `hotkeys` | `true`; replay sets `false` and injects labels instead |

`consumed` rules:

- A lone Shift tap while Cmd, Ctrl or Option is held is never a tap.
- A hotkey hit returns `consumed = true`. Other `flagsChanged` events return `false` so the app keeps its modifier state.
- A `flagsChanged` that repeats the previous modifier state (VS Code and Electron send each twice) is ignored for tap detection, returns `false`, and still logs a `key` record.
- With `ctrl_space = true`, Ctrl+Space (no Cmd or Option) returns `consumed = true`; with the default `false` it reaches the app.
- With `hotkeys = false`, hotkeys are still recognized and swallowed, but switch no mode and make no label, so word ids match the source log.

## State machine

The transition table in `engine/machine.rs` is the single source of truth. Each key does exactly one thing for a given (state, key class) ([ADR 0012](adr/0012-unified-boundary-commit.md)).

```text
   +------+ letter  +-----------+ non-language char +---------+
   | Idle | ------> | Composing | ----------------> | Pending |
   +------+         +-----------+ <---------------- +---------+
      |  ^            |  ^   Backspace (drops the boundary) |  |
      |  |  commit key|  +-- letter / Backspace (self loop) |  |
      |  +------------+                                     |  |
      |  +--------------------------------------------------+  |
      |    commit key, or a 2nd non-language char: commit it,  |
      |    the key goes on to the app, back to Idle            |
      |           letter: commit, then a new Composing --------+
      | trigger (/ @ !)
      v
   +-----+ any other key passes through (self loop)
   | Raw | ---> Idle on a terminator (space, Return, Tab, Esc, arrows, chord)
   +-----+
```

Only `Composing` and `Pending` hold marked text. `Raw` buffers nothing and `Idle` is empty.

| State | Letter | Non-letter | Trigger (`/ @ !`) | Backspace | Commit key / shortcut | Double Shift |
| --- | --- | --- | --- | --- | --- | --- |
| `Idle` | `Composing` (model picks rendering) | Passes through, no model | Enter `Raw`; trigger passes through | Passes through | Passes through | Flip sticky mode (before typing: a prior label) |
| `Composing` | Append (model picks rendering per key) | `Pending` with one boundary char | Same as non-letter (`và/hoặc`) | Drop a key; empty goes to `Idle` | Commit the model's latest choice, key goes to app, `Idle` | Flip the word and lock it (`in_word`) |
| `Pending` | Commit word + boundary; the letter opens a new word | Commit; the second char goes straight to the app (no marked text), `Idle`, even if it is a trigger | Same as non-letter | Drop the boundary, back to `Composing` | Commit, key goes to app, `Idle` | Flip the pending word, `Correction` label, sticky mode |
| `Raw` | Passes through | Passes through (except `command_end_chars`) | Passes through | Passes through, stays `Raw` | `command_end_keycodes` or a chord: passes through, `Idle` | Flip sticky mode only, no label |

Every commit does `insertText` first, then the key goes to the app (`consumed = false`), so Return, Tab and Cmd-C still act after the word is inserted.
Other commit paths: `commitComposition` (mouse click, plus `nimk_note_navigation` to leave `Raw`), `deactivateServer`, focus change, model switch or reset, shutdown, and the idle timer (`idle_commit_secs`, default 1.5 s, per app category; Swift calls `nimk_commit_pending`).

Examples (typed text is shown as keys, not user data):

| Typed | Result |
| --- | --- |
| `has.` | Marked text `has.`; double Shift turns it into `há.` |
| `Hello, world` | `,` leaves `Hello,` pending; space commits `Hello,` and passes through; `world` is a new word |
| `ls -la` then Return | Space leaves `ls ` pending; `-` commits it and passes through; `la` composes; Return commits `la` then reaches the app |
| `user_name` | `_` leaves `user_` pending; `n` commits it; `name` is a new word |
| `cd ~/Docs` | `~` commits `cd ` and passes through; `/` from `Idle` enters `Raw`; `Docs` passes through |
| `http://x.com` | `:` leaves `http:` pending; the first `/` commits it and passes through; the second `/` enters `Raw` |
| `/help`, `@src/main.rs`, `!ls -la` | `Raw` until a terminator: `/help` to Return, `@src/main.rs` to end of line, `!ls` to the space |
| `/` then space | Enters `Raw`, space leaves it at once; no effect |
| `và/hoặc` | `/` mid-word is only a boundary |

`word_commit` and labels are written at the real commit. Backspacing back into a word leaves no trace in the log and keeps the word id.

## Behavior by app category

One rule for all apps. Only `idle_commit_secs` varies, per `[[app_category]]` ([11](11-config.md)).

| Category | Idle commit | Situation | Result |
| --- | --- | --- | --- |
| Terminal (iTerm2, Terminal, WezTerm, Ghostty, kitty, Alacritty, Warp) | 1.0 s | `ls -la` + Return | `ls ` commits at `-`, `la` commits at Return, then Return reaches the shell |
| | | Tab completion | The word commits first, then Tab reaches the shell |
| | | Ctrl-C, arrows | Commit, then the key reaches the app. A leading `/`, `@` or `!` enters `Raw`, so paths and `!!` skip Telex |
| IDE (VS Code, Cursor, Zed, Xcode, JetBrains) | 1.0 s | `user_name`, `foo.bar(` | Words split at each boundary; no long marked text across `_` |
| | | Completion popup: Enter or Tab | The word commits first, then the key reaches the IDE |
| Browser (Chrome, Safari, Edge, Firefox) | 0.8 s | URL bar, search suggestions | Autofill lags at most one character (the pending boundary) or `idle_commit_secs`; Return navigates after the commit |
| Editor (Notes, TextEdit) | 2.0 s | Prose | Full correction window; the longer timer allows a fix after a short pause |

## Hotkey semantics

The hotkey both switches the mode and gives a sure label. What it does depends on the engine state (`SwitchScope`).
Each press writes a `manual_switch` record with `overlay = Some(Overlay::Manual)`; the result is always the sticky mode.

| Situation | `scope` | Visible action | Label |
| --- | --- | --- | --- |
| Mid-word | `in_word` | Flip the word's rendering and lock it until the word ends | Current word = new rendering, `correction` weight 1.0, for every step of the word |
| Pending word | `after_commit` | Flip the pending word in marked text; pressing again toggles back | At the real commit: `correction` weight 1.0 for the final rendering; if not flipped, the usual implicit label |
| No pending word, a word was committed since focus | `after_commit` | Sticky mode only | None |
| `Raw` | `after_commit` (no `word_id`) | Sticky mode only | None |
| No word committed since focus | `before_typing` | Sticky mode (prior) | Focus prior was wrong, `correction` weight 1.0 |

Only the pending word can be fixed. Text already inserted into the app is never touched.

```text
 typed:   h a s <space>     model picks Commit with EN: marked text "has "  (nothing inserted)
 Shift Shift:               marked text -> "<vi> " (ha + acute), sticky mode = Vi
 typed:   x                 insertText("<vi> "), then "x" starts the next word
                            word_commit(mode Vi) + label Correction(Vi, 1.0) written here
```

The hotkey only changes what is still marked text, so the boundary is never lost.

## IMK lifecycle

Each IMK callback calls exactly one engine function through the C ABI. No callback holds decision logic.

| Callback (`InputController.swift`) | Calls | Purpose |
| --- | --- | --- |
| Process start (`Core`) | `nimk_new`, then `nimk_set_config` with the user's `config.toml` ([11](11-config.md)) | Never fails; bad snapshot or sink falls back and reports via diagnostics |
| `activateServer(_:)` | `nimk_note_wall_clock`, then `nimk_context` after a 120 ms debounce | New app or `field_id` gives a new focus id and prior. A new activation or `deactivateServer` cancels the wait; a key arriving first sends it at once; an empty caret is retried after 80 ms |
| `handle(_:client:)` | `nimk_context` (if a debounce is pending, or a letter starts a word with no marked text), then `nimk_key` | Handle one key; `event.characters` is read only for `keyDown` |
| `deactivateServer(_:)` | `nimk_commit_pending` | No lost preedit or pending word on focus loss |
| `commitComposition(_:)` | `nimk_commit_pending`, then `nimk_note_navigation` | Mouse click; leaves `Raw` |
| After each key (`IdleCommit.swift`) | `nimk_idle_commit_secs` | After that many idle seconds with marked text, call `nimk_commit_pending`. Every key resets it; commit and deactivate cancel it |
| `recognizedEvents(_:)` | none | Returns `[.keyDown, .flagsChanged]` |
| `menu()` | `nimk_mode`, `nimk_policy_id`, model calls | IME menu ([06](06-gui.md)) |
| SIGTERM | `nimk_shutdown` | Flush log, save snapshots |

Swift files in `macos/NeuralIMK/`:

| File | Role |
| --- | --- |
| `main.swift` | Starts `IMKServer`, runs the loop, catches SIGTERM |
| `Core.swift` | Owns `NimkCore`, copies `NimkOut` into `CoreOutput`, diagnostics log |
| `Context.swift` | Reads caret and window, 1 s cache ([03](03-context-io.md)) |
| `InputController.swift` | `InputController: IMKInputController`; forwards events, applies output, builds the menu |
| `Debouncer.swift` | Delays and merges context sends on rapid activation (`make swift-test`) |
| `IdleCommit.swift` | Idle commit timer built on `Debouncer` (`make swift-test`) |
| `Overlay.swift` | Overlay and status item |
| `StatsText.swift` | Menu text for per-model counters |
| `NeuralIMK-Bridging.h` | Imports `nimk_ffi.h` |

FFI rules are in [01](01-architecture.md#c-abi). `Engine` is not `Sync`, so it is called only from the IMK main thread.

Not yet verified on a real install: the IME appears in Input Sources, the key to marked to commit path, the `LSBackgroundOnly` status item, overlay on a second display, `field_id` stability, and marked text lingering after a space in some apps ([09](09-app-compat.md)).
