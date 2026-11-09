# 0014. Transform and Hook pipeline

> **Status:** accepted · **Date:** 2026-11-09 · **Extends:** [0013](0013-shadow-training-and-model-stats.md), [0002](0002-swift-shim-rust-core.md) · **Changes:** [0003](0003-shift-tap-hotkey.md) (Ctrl+Space default)

## Context

Two kinds of code were mixed inside the engine. Some of it changes what the user sees (Telex rendering, the model's choice of language). The rest only watches (the event logger, per-model statistics) and had grown into the trainer.
Adding a new observer, such as per-day usage counts, meant touching the engine and the trainer. Observers also shared the failure modes of the key path: a bug in a statistic could break typing.

## Decision

Split the runtime into two kinds of component, both defined over data.

- **Transforms** are pure stages that change the buffer or the output. `nimk_types::Transform` has one method, `apply(&Input) -> Output`. There are two: `TelexTransform` in `nimk-telex` (raw buffer to `BufferState { vi, valid_vi, viable_vi }`) and `ModelTransform` in `nimk-policy` (`ModelInput` to `ModelOutput { action, p_vi }`: score, decide, hysteresis, lock, commit-step rule). State (the 4-state machine, `shown`, `locked`, the composer) stays in the engine and is passed in as input. `ModelInput` borrows the `Observation` to avoid a clone per key. The teacher-forced path (`Engine::teach`) bypasses the model and stays in the engine. The refactor kept the decision stream bit-identical; a fixture test (`pipeline_fixture`) pins it.
- **Hooks** are observers of `Record`s, in the new crate `nimk-hook`. A `Hook` has `name`, `init`, `on_record` and `flush`, and gets a `HookCx` that reads and writes blobs under `hooks/<name>/`, publishes a summary and reports diagnostics. A hook never sees the engine, never spawns threads and never reads the wall clock or environment.
  Built in: `event_log` (writes the event log), `model_stats` (per-model statistics, blobs `hooks/model_stats/<kind>`; the legacy `stats/<kind>` is migrated once at init) and `usage` (per-day counts per app, blob `hooks/usage/<YYYY-MM-DD>`).
- **`Record` is the single data model.** A hook sees only what the log contains, so any hook can be replayed offline from a recorded log and gives the same result as live. Wall time is derived from data: `session_start.wall_unix_ms` plus event time; local dates use the new `session_start.utc_offset_min` (`#[serde(default)]`, filled by `nimk-ffi`).
- **`model_score` is an additive event** (`kind`, `word_id`, `category`, `correct`, `display_hits`, `display_total`, `flips`, `amend`, `served`), emitted from the trainer's per-word results so `model_stats` can work from records alone. `model_reset { kind }` zeroes a model's counters. `SCHEMA_VERSION` stays 1; replay ignores both.
- **Isolation.** `nimk-engine/src/hookbus.rs` replaces the old logger. It owns the `Record` envelope and runs the hooks in registration order, on one hook thread (`Background`) or on the caller (`Inline`). Every hook call is wrapped in `catch_unwind`; a panicking hook is disabled and reported through diagnostics, and the key path is never affected.
- **Flush cadence** uses event time only, so it is deterministic: after a `word_commit` when at least 30 s have passed since the last flush, and at shutdown. `event_log` also appends every record immediately.
- Summaries and diagnostics return to the engine over a channel and are drained at word boundaries. `Engine::hook_summary(name)` and the FFI export `nimk_hook_summary(core, name)` read the latest one; `Engine::model_stats` and `nimk_model_stats` keep their signatures.
- **Config default.** `ctrl_space` is now `false`: Ctrl+Space collides with the macOS input-source shortcut and IDE autocomplete. Double Shift is unchanged; users can set `ctrl_space = true`.
- **arch-lint.** New scope `hook` with rule `leaf-hook` (hooks depend only on `types` and `storage`, and may not use `std::fs/net/thread/time/env`), rule `engine-uses-transforms` (the engine may not call `nimk_telex::transform` or `nimk_policy::decide` directly), and `nimk_hook` denied in `types-is-bottom`, `leaf-storage`, `leaf-telex`, `leaf-policy` and `ffi-thin`. The `eng-logger` scope became `eng-hookbus`.

## Consequences

- A new observer is one `Hook` impl and a registration; the engine and trainer do not change.
- A faulty hook costs its own data, not typing. Disabled hooks stay disabled until the next start.
- Hooks cannot see state that is not in a `Record`. Anything a hook needs must be logged first, which is why `model_score` exists.
- The usage day follows the uptime clock anchored on the first record after `session_start`. The uptime clock stops during sleep, so the additive `clock { wall_unix_ms, utc_offset_min }` record (logged by `nimk_note_wall_clock` on wake and IME activation; replay ignores it) re-anchors it; the wall clock is still read only in `nimk-ffi`.
- Users who relied on Ctrl+Space must opt in. Replay and the pipeline fixture run with `hotkeys = false` and are unaffected.
- The C ABI grows to 20 exports (`nimk_hook_summary`).
