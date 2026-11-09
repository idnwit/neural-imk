# NeuralIMK overview

> **Status:** draft · **Updated:** 2026-11-09 · **Code:** `Cargo.toml`, `crates/` · **Architecture:** [01](01-architecture.md)

NeuralIMK is a single macOS input method. A neural model picks Vietnamese (Telex) or English for each word, and learns online from the user's double-Shift corrections. The goal is to push manual corrections toward zero.

## Goals

| # | Goal | Docs |
| --- | --- | --- |
| 1 | One IMK input source; VI/EN is an internal mode; Telex is the only typing method | [02](02-keyboard-io.md), [ADR 0001](adr/0001-single-imk-input-source.md) |
| 2 | Switch hotkey handled inside the IME: double Shift flips the word, Ctrl+Space as fallback | [02](02-keyboard-io.md), [ADR 0008](adr/0008-double-shift-retro-correction.md) |
| 3 | Context is only `bundle_id` and the relative caret position | [03](03-context-io.md) |
| 4 | Swappable action models (`LinearAction`, `GruAction`), pretrained on synthetic data, learning online | [04](04-engine-ml.md), [models](models/linear.md), [ADR 0007](adr/0007-neural-action-model.md) |
| 5 | Log every event for offline replay and model comparison | [05](05-storage-and-log.md) |
| 6 | VI/EN overlay at the caret and a menu-bar status item | [06](06-gui.md) |

## Non-goals

| Skipped | Why |
| --- | --- |
| Log encryption, sensitive-app blocklist | Logs are plaintext; a decorator over `Storage` is the planned path |
| Hiding the marked-text underline | Accepted ([ADR 0002](adr/0002-swift-shim-rust-core.md)) |
| Fake backspaces to fix committed text | The last word stays marked text until committed, so it can flip in place ([ADR 0011](adr/0011-pending-word-commit.md)) |
| VNI, abbreviations, macros | Telex only |
| Codesign, notarize | Dev machine only, ad-hoc signed |
| Multi-user, model sync | One machine, one model |

## Success metrics

| Metric | Target | Measured by |
| --- | --- | --- |
| Manual switches per 1000 words | At least 50% below `nimk-replay --frozen` on real logs | `nimk-replay` |
| `nimk_key` latency p99 | Under 2 ms including inference | Real process |
| Stability | No crash in a day of typing | Dogfooding |

## Milestones

| Milestone | Content | Done when |
| --- | --- | --- |
| M0 | 8-crate workspace, `arch-lint.toml`, `make check` | fmt, clippy, tests, arch-lint pass |
| M1 | Rust core without UI | Golden tests pass |
| M2 | Swift shim and `libnimk_ffi.a` | Typing works in TextEdit, Notes, Chrome, VS Code; double Shift flips |
| M3 | Real context and logs, overlay, status item | Logs carry correct `bundle_id` and caret |
| M4 | Both models, synthetic pretraining, correction window | `nimk-replay` beats `--frozen` on real logs |
| M5 | One week of dogfooding | Success metrics met; conclusion in [devlog](devlog.md) |

## Glossary

| Term | Meaning |
| --- | --- |
| `raw` | Raw keys of the current word, e.g. `tieengs` |
| marked text | Text being composed and underlined, not yet committed |
| pending word | A finished word still marked; double Shift flips its rendering |
| manual switch | Double Shift or Ctrl+Space; the source of reliable labels |
| `Observation` / `Action` | Model input and output per key; `Action` is `edit` plus `render` |
| `p_vi` | Probability that Vietnamese is wanted; the render output |
