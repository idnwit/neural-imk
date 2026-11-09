# 0013. Shadow training and per-model statistics

> **Status:** accepted · **Date:** 2026-11-09 · **Extends:** [0010](0010-sdk-storage-and-bundled-models.md), [0007](0007-neural-action-model.md)

## Context

Until now the trainer owned only the active learner, so a model the user switched away from learned nothing, and nobody could say which model makes fewer mistakes on the user's own typing. Labels and Observations do not depend on which model produced them, so every model can learn from, and be measured on, the same stream.

## Decision

- The trainer holds one core per loaded model (both kinds). Every finalized label's Examples go to each core whose `online_training` is on, each learner applying its own rules (`GruAction` still learns only from corrections). Only the active core publishes snapshots to the hot path.
- `online_training` is a per-model option: `[models.<kind>] online_training` in `default.toml` (default true), overridable in `config.toml` and by the IME menu. The menu choice is persisted in the blob `settings/models` and wins over `config.toml` at the next start. Off means frozen: no updates, no saves, no shadow scoring or counting (the active model is always counted because it serves).
- Each learner saves its own `model/<kind>/latest` on the usual cadence and at shutdown. A shadow is written only if it changed, so an untouched shadow never copies a bundled model into the user layer.
- Statistics, per word, for the active model and every enabled shadow: corrected (active: the user corrected the word; shadow: its own fresh decision at the commit step would have been corrected), display accuracy over keystrokes (the model's display decision with its own hysteresis equals the final label) and flips per word. Lifetime and rolling last 500 words, plus corrected-by-app-category. Stored in the blob `stats/<kind>` (versioned JSON), reset with "Khôi phục model gốc".
- Shadow scoring is off the hot path. The engine pushes each keystroke's Observation (word id, active display decision, hysteresis margin) onto the trainer channel; the trainer side scores it with each shadow's latest weights. The active model's hot-path decision is reused for its own stats and never scored twice. `Inline` threading does the same synchronously.

## Consequences

- Two learners live in memory (GRU about 0.2 MB, Linear about 0.7 MB). Background thread work roughly doubles; the hot path only pays one Observation clone per key while a shadow is enabled.
- Switching models rebuilds the trainer (the old one saves every model it trained), so a switch is no longer lossy for the model left behind.
- `Event::ModelStats` is not logged: counters are derivable from `Decision` and `Label` records and persist in `stats/<kind>`.
- Mixed history: a model's counters mix words it served (real corrections) and words it shadowed (would-be corrections); `served_words_total` says how many were real.
