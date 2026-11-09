# 0003. Switch hotkey = quick tap of left Shift, Ctrl+Space fallback

> **Status:** superseded by [0008](0008-double-shift-retro-correction.md) · **Date:** 2026-11-08

## Context

The switch hotkey is both a user action and a source of reliable labels, so the IME must receive it directly.
Cmd+Space is swallowed by the system before it reaches the IME. A global event tap needs the Accessibility permission, which conflicts with the goal of asking for none.

## Decision

- Default: press and release left Shift alone within `shift_tap_secs` (0.3 s), with no `keyDown` in between.
- The IME declares `recognizedEvents(_:)` = `[.keyDown, .flagsChanged]` to receive modifier events.
- Fallback: Ctrl+Space via `keyDown`, enabled by `EngineConfig::ctrl_space` (default `true`).
- Detection is a Rust state machine (`HotkeyDetector`) driven by `KeyEvent::timestamp`; Swift does not handle it.
- A hotkey right after a commit only changes the mode for the next word and produces a correction label; it does not rewrite committed text.

## Consequences

- No Accessibility permission or event tap needed.
- Holding Shift to type capitals never triggers it, because a `keyDown` always intervenes.
- Apps that swallow `flagsChanged` leave only Ctrl+Space, which clashes with some apps' shortcuts.
- A word committed in the wrong mode stays wrong on screen. This was fixed in 0008.
- Because the hotkey produces labels, its semantics are part of the learning contract ([02](../02-keyboard-io.md)).
