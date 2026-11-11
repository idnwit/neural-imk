# NeuralIMK docs

> **Status:** stable · **Updated:** 2026-11-09

`docs/` is the source of truth for design. Code is the source of truth for behaviour. When they differ, fix the doc in the same commit as the code. Start with the [project README](../README.md).

## Index

| File | Content | Code it describes |
| --- | --- | --- |
| [00-overview.md](00-overview.md) | Goals, non-goals, success metrics, milestones | - |
| [01-architecture.md](01-architecture.md) | Processes, data flow, crates, layer rules | `Cargo.toml`, `arch-lint.toml` |
| [02-keyboard-io.md](02-keyboard-io.md) | IMK, composer, state machine, double Shift, per-app UX | `crates/nimk-engine/src/{composer,hotkey}.rs`, `crates/nimk-engine/src/engine/`, `macos/` |
| [03-context-io.md](03-context-io.md) | `bundle_id`, caret position | `macos/NeuralIMK/`, `nimk-types` |
| [04-engine-ml.md](04-engine-ml.md) | Action model contract, online learning, pretraining | `crates/nimk-policy`, `crates/nimk-engine/src/{labeler,trainer}.rs` |
| [05-storage-and-log.md](05-storage-and-log.md) | `Storage` trait, event schema, replay, metrics | `crates/nimk-storage`, `crates/nimk-replay`, `crates/nimk-synth` |
| [06-gui.md](06-gui.md) | Overlay, status item, input menu | `macos/NeuralIMK/Overlay.swift` |
| [07-build-and-install.md](07-build-and-install.md) | Build, bundle, install, debug | `Makefile`, `scripts/` |
| [08-coding-standard.md](08-coding-standard.md) | Code rules, arch-lint, tests | `arch-lint.toml`, `Makefile` |
| [09-app-compat.md](09-app-compat.md) | Marked-text compatibility per app type | `macos/NeuralIMK/InputController.swift` |
| [10-telex.md](10-telex.md) | Telex rules R1-R10 and tests | `crates/nimk-telex` |
| [11-config.md](11-config.md) | Every TOML key, range and default | `crates/nimk-engine/config/default.toml` |
| [12-metrics.md](12-metrics.md) | Ground truth and every online/offline accuracy metric | `crates/nimk-types/src/truth.rs`, `crates/nimk-engine/src/trainer/stats.rs` |
| [models/](models/) | One doc per model: [Linear](models/linear.md), [GRU](models/gru.md) (architecture, training, checkpoints, results) | `crates/nimk-policy` |
| [research/](research/) | Historical research notes behind decisions | - |
| [adr/](adr/) | Architecture decision records, latest: [0016 On-demand fine-tuning](adr/0016-on-demand-fine-tuning.md) | - |
| [devlog.md](devlog.md) | Short dated log, newest first | - |

## Writing standard

### Language and format

- English only. Vietnamese appears only as subject matter (Telex examples such as `tieengs` -> `tiếng`) or quoted app labels, with an English gloss.
- GitHub Markdown, one topic per file, file names `NN-kebab-case.md`.
- Identifiers, crate names and APIs go in `backticks`.
- Line 1 is `# Title`, then a metadata line:

  ```markdown
  > **Status:** draft | stable · **Updated:** YYYY-MM-DD · **Code:** `crates/...`
  ```

- Open each file and section with the conclusion. Sentences under 25 words, paragraphs of at most 3 sentences.
- Use tables for multi-attribute data and numbered lists for ordered steps.
- Code blocks have a language (`rust`, `swift`, `toml`, `sh`). Signatures must match the code.
- Never commit raw user-typed text; only aggregate statistics.

### ASCII diagrams

- Diagrams are ASCII inside ```` ```text ```` fences. No images, no Mermaid, no Unicode box drawing.
- Use `+ - | / \ < > v ^` and letters. At most 100 columns.
- Boxes are `+---+` / `|   |`; arrows are `-->`, `<--`, or `|` with `v`.
- Put one sentence under each diagram stating the takeaway.

```text
+-------------+   NSEvent    +--------------+   C ABI    +-------------+
|  App focus  | -----------> | Swift shim   | ---------> | Rust core   |
+-------------+              +--------------+            +-------------+
```

Swift only forwards; every decision lives in the Rust core.

### ADRs

- File `adr/NNNN-kebab-title.md`, numbered in order. Do not rewrite an accepted ADR; add a new one that supersedes it.
- Shape:

  ```markdown
  # NNNN. Title

  > **Status:** proposed | accepted | superseded by NNNN · **Date:** YYYY-MM-DD

  ## Context
  ## Decision
  ## Consequences
  ```

### Model docs

Each model has `models/<name>.md` with: what it is, architecture (with diagram and parameter count), training, checkpoints, results with exact reproduction commands, date and commit, and known limitations. Unmeasured metrics say "not measured".

### Devlog

- Table `Date | Type | Note`, newest first. Types: `decision`, `finding`, `open`, `done`.

### When to update

- Public API, event schema, layer rules or config defaults change: update the matching doc in the same commit.
- New `arch-lint.toml` rule: update `08-coding-standard.md` and `01-architecture.md`.
- A decision with trade-offs: write an ADR.
- Model code or training changes: re-measure and update `models/*.md` and the README table.
