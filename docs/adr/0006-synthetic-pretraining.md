# 0006. Pretrain on synthetic typing data

> **Status:** accepted · **Date:** 2026-11-08

## Context

Learning online from zero means the model guesses near-randomly on ambiguous words in the first days, and users must correct a lot.
Real logs only exist after M3, but M4 needs data to compare policies and benchmark.
Most of the "is this word VI or EN" signal is in the key sequence itself and needs no app context.

## Decision

- Add a `nimk-synth` crate that generates mixed VI/EN typing scripts in the style of Vietnamese users, with no app context (`SYNTHETIC_BUNDLE_ID = "synthetic"`).
- Telex key sequences come from `nimk_telex::encode`, the inverse of `transform`. Ambiguous EN words (`has`, `for`, `user`) are sampled more densely.
- Two outputs: `write_log` writes a normal event log for `nimk-replay`; `pretrain` runs the script through the engine inline (`hotkeys = false`, labels via `Engine::inject_label`) and saves the model as the IME's starting snapshot.
- The corpus is hand-written for the project, with no third-party corpus. The same `SynthConfig` and seed give the same output.

## Consequences

- The IME starts with a model that already separates VI/EN n-grams; online learning only adds per-app and per-user knowledge.
- A deterministic policy benchmark exists before real logs.
- App and position features cannot be learned from synthetic data; the model learns them online.
- The synthetic distribution differs from real typing, so synthetic metrics do not replace real-use metrics.
- Synthetic labels are written as `correction` with weight 1.0, so synthetic logs must be kept apart from real logs when computing metrics.
- Note: the generator was later made realistic and aligned with the ADR 0012 state machine (see [devlog](../devlog.md), [real-log-stats](../research/real-log-stats.md)).
