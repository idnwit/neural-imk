# 0012. Unified commit and boundary: Idle, Composing, Pending, Raw

> **Status:** accepted · **Date:** 2026-11-09 · **Supersedes:** the boundary part of [0011](0011-pending-word-commit.md) ("pending keeps up to 16 boundary chars")

## Context

ADR 0011 let a pending word accumulate boundary chars and let the model decide every key, including `Pass` mid-word.
In terminals, IDEs and browsers, `ls -la`, `user_name`, `http://x.com` and `/help` gave unpredictable results and marked text stayed open longer than expected.
We need one rule, explainable with a single table, and every parameter in config instead of code constants.

## Decision

A 4-state machine is the single source of truth. The transition table is `transition` in `crates/nimk-engine/src/engine/machine.rs` (tested), described in [02](../02-keyboard-io.md).

- States: `Idle`, `Composing(word)`, `Pending(word + exactly one boundary char)`, `Raw`.
- Key classes: letters (language), printable non-letters (boundary; some are `command_triggers`), Backspace, commit keys (Return, Tab, Escape, arrows, Home/End/PageUp/PageDown, forward Delete, function keys, every Cmd/Ctrl/Opt shortcut), and ignored keys.
- Edits are decided by the key (letter = `Append`, Backspace = `Pop`, ending key = `Commit`). The model only picks the rendering and `p_vi`.
- A non-letter while `Composing` becomes `Pending` with one char (still flippable by double Shift). A second non-letter while `Pending` commits the word and sends that char straight to the app, going to `Idle`.
- `Idle` plus a trigger (default `/`, `@`, `!`) enters `Raw`: keys go straight to the app, with no buffer, Telex or model, and only a `key` record is written. `Raw` ends on a char in `command_end_chars` (default space), a keycode in `command_end_keycodes`, or a shortcut; the ending key still reaches the app. A trigger mid-word is only a boundary.
- Double Shift: `Composing` flips and locks the word; `Pending` flips the pending word (label `Correction`, sticky mode); `Idle` changes the sticky mode (before typing, a focus-prior label); `Raw` changes only the sticky mode, with no label.
- Every commit (Return, Tab, Escape, navigation, shortcut, click, focus loss, model change, idle timer) does `insertText` first, then the key reaches the app (`consumed = false`).
- The idle timer lives in Swift (the engine has no clock): after `idle_commit_secs` without a key while `Composing` or `Pending`, Swift calls `nimk_commit_pending`.
- Labels: commit by timer, Return, a second boundary or a letter is `Implicit`; double Shift is `Correction`.
- Config: all parameters are TOML. Defaults live in `crates/nimk-engine/config/default.toml` (embedded via `include_str!`); the user override is `<Application Support>/<bundle id>/config.toml`, read by Swift and passed through `nimk_set_config`. Per-app-category overrides (`[[app_category]]`: terminal, ide, browser, editor) are chosen by `bundle_id` on each focus change. Bad values are dropped with a diagnostic and defaults are kept. See [11](../11-config.md).

## Consequences

- Predictable in every app: at most one boundary char sits in marked text, so the app lags by at most one char or `idle_commit_secs`.
- Training data: `ActionMask` now has one allowed action; non-letters in `Idle` and all `Raw` keys produce no samples; a word with a mid-word non-letter (`user_name`) splits into several words. Pretrained models still work (the render head is unchanged) but should be retrained once the generator matches the new semantics (done later, see [devlog](../devlog.md)).
- Log: `word_commit` still arrives after the boundary key; word ids are still independent of mode.
- ABI additions: `nimk_set_config`, `nimk_idle_commit_secs`, `nimk_note_navigation`.
- Risk: URLs and paths typed after a trigger are `Raw` and never Telex (intended); a Vietnamese sentence starting with `/` also enters `Raw` until the next space.
