# Devlog

> **Working log** · **Updated:** 2026-11-11 · Decisions, findings and open items; newest first. Obsolete entries are dropped.

| Date | Type | Note |
| --- | --- | --- |
| 2026-11-11 | finding | Caret-flip spike: VS Code and Zed allow reading before the caret and range replacement; Chromium reads only in-bounds ranges; terminals expose no text but pass DEL and arrow bytes ([09](09-app-compat.md#text-access-for-caret-flip)); design in [ADR 0015](adr/0015-caret-flip.md), then implemented with a simulator page in the companion app |
| 2026-11-09 | decision | Shadow training and per-model stats: every loaded model learns from the same labels, with `models.<kind>.online_training` and `stats/<kind>` ([ADR 0013](adr/0013-shadow-training-and-model-stats.md)) |
| 2026-11-09 | decision | Unified 4-state machine Idle/Composing/Pending/Raw ([ADR 0012](adr/0012-unified-boundary-commit.md)); edits come from keys, the model only picks the rendering; TOML config ([11](11-config.md)); idle commit timer in Swift |
| 2026-11-09 | done | Generator follows the ADR 0012 machine (`nimk-synth/src/track.rs`); both models retrained on the realistic profile (render: GRU 0.981, Linear 0.977) |
| 2026-11-09 | done | Realistic synthetic profile (runs, phrases, Zipf word frequency, app flavours), calibrated on 1319 real words ([real-log-stats](research/real-log-stats.md)); `MixProfile::legacy()` keeps the old behavior |
| 2026-11-09 | finding | Right context does not help on synthetic data, so no two-word UX ([right-context-experiment](research/right-context-experiment.md)) |
| 2026-11-08 | decision | Default policy is `gru_action`: after Telex v2 it scored render 0.974 / ambiguous 0.968 vs Linear 0.965 / 0.961, and only GRU uses left context (probe `theme`) |
| 2026-11-08 | decision | Telex v2 (free marking, late tone/`dd`, `uo` to `uơ`/`ươ`, word lock after undo, `ToneStyle`); 110-row conformance table in `nimk-telex/tests/conformance.rs`; U/O rows never run on real Unikey/OpenKey |
| 2026-11-08 | decision | Model chooser in the IME menu, one checkpoint per model (`model/<kind>/latest`), choice stored in a settings blob |
| 2026-11-08 | done | SDK-defined storage paths and bundled pretrained models ([ADR 0010](adr/0010-sdk-storage-and-bundled-models.md)) |
| 2026-11-08 | decision | Unsupported apps only flip mode and learn; no fake backspaces, no Accessibility permission ([ADR 0008](adr/0008-double-shift-retro-correction.md), later [0011](adr/0011-pending-word-commit.md)) |
| 2026-11-08 | finding | Label leak: `shown` equaled the target rendering during teacher forcing, so the model copied it; `SHOWN_VI` is now reserved and always 0 ([ADR 0007](adr/0007-neural-action-model.md)) |
| 2026-11-08 | finding | Render-only updates drift the GRU trunk; `GruAction` learns only from corrections; commit uses a fresh model decision, hysteresis only mid-word |
| 2026-11-08 | finding | Standalone `w` must give `ư` (as Unikey/EVKey); fixed in `nimk-telex` and the generator |
| 2026-11-08 | decision | NN backend is `burn` =0.22.1 `flex`, no rayon; dev profile `opt-level = 3` ([ADR 0009](adr/0009-burn-nn-backend.md)) |
| 2026-11-08 | decision | Pivot to the neural action model, hard rules removed ([ADR 0007](adr/0007-neural-action-model.md)) |
| 2026-11-08 | decision | `switch_margin` lowered from 0.2 to 0.02 so mid-word display follows the model |
| 2026-11-08 | finding | arch-lint 0.11 limits: `no-panic-in-lib` needs a second pass (`make arch`); `deny-attr` misses `#[unsafe(no_mangle)]` (the `unsafe_code = "deny"` lint covers it); `[[independent]]` does not resolve `crate::` paths, so `restrict-use` is used |
| 2026-11-08 | finding | Snapshots live at `<root>/blobs/model/<kind>/latest` (`PlainFsStorage` adds `blobs/`) |
| 2026-11-08 | decision | Synthetic pretraining ([ADR 0006](adr/0006-synthetic-pretraining.md)), online supervised learning without exploration ([ADR 0005](adr/0005-online-supervised-not-exploratory-rl.md)), storage behind a trait ([ADR 0004](adr/0004-storage-behind-trait.md)) |
| 2026-11-08 | decision | One IMK input source with an internal VI/EN mode; thin Swift shim over a static Rust core ([ADR 0001](adr/0001-single-imk-input-source.md), [0002](adr/0002-swift-shim-rust-core.md)) |
| 2026-11-08 | open | Not verified on a real Mac: IME in Input Sources, the key-to-commit path, per-app behavior ([09](09-app-compat.md)) |
| 2026-11-08 | open | Lower the QoS of the trainer thread; `Overlay::Auto` may be noisy when sticky mode changes |
