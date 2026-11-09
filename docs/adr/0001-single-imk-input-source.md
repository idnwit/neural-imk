# 0001. One IMK input source; VI/EN is an internal mode

> **Status:** accepted · **Date:** 2026-11-08

## Context

To pick VI or EN per word, the IME must see every key in both modes and switch without help from macOS.
With two separate input sources, switching needs `TISSelectInputSource`, which is slow and drops the preedit.
In a plain EN source (ABC) the IME gets no keys, so it cannot learn or decide.

## Decision

- NeuralIMK is a single InputMethodKit input source. EN is a pass-through mode inside the IME.
- Telex is the only Vietnamese typing method in the PoC.
- The mode (`Lang::En` / `Lang::Vi`) is `Engine` state, not system state.

## Consequences

- The IME sees every key, even in EN, so the policy can flip mode mid-word and the log is complete.
- The hot path must be cheap in both modes (p99 < 2 ms).
- The system hotkey Cmd+Space no longer switches VI/EN; a dedicated hotkey is needed ([0003](0003-shift-tap-hotkey.md), [0008](0008-double-shift-retro-correction.md)).
- If the IME crashes, EN input is lost too; users should keep ABC as a fallback.
