# 0013. Shadow training and per-model statistics

> **Status:** accepted · **Date:** 2026-11-09 · **Extends:** [0010](0010-sdk-storage-and-bundled-models.md), [0007](0007-neural-action-model.md)

## Context

The trainer used to own only the active learner, so a model the user switched away from learned nothing, and nobody could say which model makes fewer mistakes on the user's own typing.
Labels and Observations do not depend on which model produced them, so every model can learn from, and be measured on, the same stream.

## Decision

Ground truth and every metric are defined in [12-metrics.md](../12-metrics.md): an uncorrected word is correct, a corrected word's truth is the language switched to, and a late correction amends the earlier record.

- The trainer holds one core per loaded model (both kinds). Every finalized label's Examples go to each core whose `online_training` is on, and each learner applies its own rules (`GruAction` still learns only from corrections). Only the active core publishes snapshots to the hot path.
- `online_training` is per model: `[models.<kind>] online_training` in `default.toml` (default true), overridable in `config.toml` and by the IME menu. The menu choice is stored in the blob `settings/models` and wins over `config.toml` at the next start. Off means frozen: no updates, saves, shadow scoring or counting (the active model is always counted because it serves).
- Each learner saves its own `model/<kind>/latest` on the usual cadence and at shutdown. A shadow is written only if it changed, so an untouched shadow never copies a bundled model into the user layer.
- Statistics per word, for the active model and each enabled shadow: corrected (active: the user corrected the word; shadow: its own fresh decision at the commit step would have been corrected), display accuracy over keystrokes (its display decision with its own hysteresis equals the final label), and flips per word. Kept lifetime and for the last 500 words, plus corrected-by-app-category, in the blob `stats/<kind>` (versioned JSON, version 2). The menu item "Khôi phục model gốc" (restore bundled model) resets them.
- Shadow scoring is off the hot path: the engine pushes each keystroke's Observation (word id, active display decision, hysteresis margin) to the trainer channel, and the trainer scores it with each shadow's latest weights. The active model's hot-path decision is reused for its own stats. `Inline` threading does the same synchronously.

## Consequences

- Two learners live in memory (GRU about 0.2 MB, Linear about 0.7 MB). Background work roughly doubles; the hot path pays one Observation clone per key while a shadow is enabled.
- Switching models rebuilds the trainer (the old one saves every model it trained), so a switch no longer loses learning for the model left behind.
- `Event::ModelStats` is not logged: counters can be derived from `Decision` and `Label` records and persist in `stats/<kind>`.
- A model's counters mix words it served (real corrections) and words it shadowed (would-be corrections); `served_words_total` says how many were real.
