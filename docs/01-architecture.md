# Architecture

> **Status:** stable · **Updated:** 2026-11-09 · **Code:** `Cargo.toml`, `arch-lint.toml`, `crates/nimk-engine/src/{lib,engine}.rs`, `crates/nimk-ffi/src`

NeuralIMK is one process, `NeuralIMK.app`. A thin Swift shim talks to IMK/AppKit; all logic lives in a Rust core linked statically through a C ABI.
The hot path only reads a model snapshot. Training and log writing run in the background, so a keystroke never waits for I/O.

## Process and threads

One process, three Rust threads. Only the `trainer` and `logger` modules spawn threads.

| Thread | Runs | Communicates |
| --- | --- | --- |
| Main (IMK, called from Swift) | `Engine::on_key`, `on_context`, `commit_pending`, `teach`: hotkey, composer, model score, apply | Reads the current `Arc<dyn ImeModel>`; sends `Record` and `Example` over channels |
| Trainer | Replay buffer, correction window, `ImeLearner::update`, snapshots, per-model stats | Sends new `Arc<dyn ImeModel>` over mpsc; the engine `try_recv`s at a word boundary |
| Log writer | Serializes `Record` to `EventSink::append` | Receives records over a channel |

With `Threading::Inline` (tests, replay, synth) all three run on the calling thread, so results are deterministic.

```text
+------------+  C ABI   +-----------------------------+ Record  +--------+ bytes +---------+
| Swift shim | -------> | main thread: Engine         | ------> | logger | ----> | Storage |
|   (IMK)    | <------- | hotkey > composer > model   |         +--------+       +---------+
+------------+ NimkOut  +-----------------------------+                               ^
                           |                    ^                                     |
                  Example  |                    | Arc<dyn ImeModel> (mpsc, try_recv)  |
                  (batch)  v                    |                                     |
                        +-------------------------+   snapshot (ImeLearner::save)     |
                        |         trainer         | ----------------------------------+
                        +-------------------------+
```

The only arrow from the background into the main thread is a model snapshot, taken at a word boundary.

## Per-key loop

Each key follows a fixed loop: the engine builds an `Observation`, the model scores it, and the engine applies the result mechanically.
Details of the states and edits are in [02](02-keyboard-io.md); the model is covered in [04](04-engine-ml.md).

```text
 KeyEvent
    |
    v
+--------------+ hit   +--------------------+
| HotkeyDetect | ----> | manual_switch      | --> EngineOutput { consumed, marked?, mode }
+--------------+       +--------------------+
    | no hit
    v
+-----------------+  Observation   +-------------------+  ActionScores  +----------------+
| flow: mask_for, | -------------> | ImeModel::score   | -------------> | decide(mask)   |
| observation()   |                | (LinearAction or  |                | + hysteresis   |
+-----------------+                |  GruAction)       |                +----------------+
                                   +-------------------+                        |
                                                                           Action v
+-----------------------------------------------------------------------------------+
| apply: Append -> push + Telex + marked | Pop -> pop | Commit -> pending | Pass     |
+-----------------------------------------------------------------------------------+
    |                                   |
    v                                   v
 EngineOutput { consumed, commit,     Decision / WordCommit / Label records,
   marked, mode, overlay }            Example -> trainer
```

The hotkey runs before the model and never enters `Observation`. The model only picks the rendering (VI or EN); the edit is structural and fixed by the state machine.

## Crates and layers

The workspace `crates/*` has 8 crates (edition 2024). Dependencies only point downward.

```text
 +--------------+  +--------------+  +--------------+      +-------------------------+
 |  nimk-ffi    |  | nimk-replay  |  | nimk-synth   |      | macos/NeuralIMK (Swift) |
 |  staticlib   |  |  bin         |  |  lib + bin   |      | links libnimk_ffi.a     |
 +--------------+  +--------------+  +--------------+      +-------------------------+
        |                 |                 |                    |  C ABI (nimk_ffi.h)
        |                 |                 |    <---------------+  calls nimk-ffi only
        +-----------------+-----------------+
                          |
                          v
                 +-----------------+
                 |   nimk-engine   |
                 +-----------------+
                   |       |      \
                   v       v       v
        +------------+ +-------------+ +----------------+
        | nimk-telex | | nimk-policy | |  nimk-storage  |
        +------------+ |  (burn)     | |  (true leaf,   |
                   |   +-------------+ |  no nimk deps) |
                   v       v           +----------------+
                 +-----------------+
                 |   nimk-types    |
                 +-----------------+
```

A layer may skip the one below it (`nimk-synth` uses `nimk-policy` directly) but never depends upward.

| Crate | Kind | Role |
| --- | --- | --- |
| `nimk-types` | lib | `Lang`, `KeyEvent`, `AppContext`, action contract (`Observation`, `Action`, `ActionMask`, `Example`), event schema |
| `nimk-storage` | lib | `Storage`, `EventSink`, `EventSource`, `BlobStore` and their implementations; bytes only |
| `nimk-telex` | lib | `transform`, `encode`, `is_valid_syllable` ([10](10-telex.md)) |
| `nimk-policy` | lib | `ImeModel`, `ImeLearner`, `ModelKind`, `LinearAction`, `GruAction` (backend `burn`); see [linear](models/linear.md), [gru](models/gru.md) |
| `nimk-engine` | lib | `Engine`: composer, hotkey, labeler, trainer, logger |
| `nimk-ffi` | staticlib | C ABI for Swift; the only crate with `extern "C"` and `unsafe` |
| `nimk-replay` | bin + lib | Replays a log through a model and reports metrics ([05](05-storage-and-log.md)) |
| `nimk-synth` | lib + bin | Synthetic typing data and pretraining ([ADR 0006](adr/0006-synthetic-pretraining.md)) |
| `macos/NeuralIMK` | Swift | IMK shim, overlay, status item ([06](06-gui.md), [09](09-app-compat.md)) |

`arch-lint.toml` enforces the layering in `make check`; see [08](08-coding-standard.md#contract-arch-lint).

## Modules in `nimk-engine`

The five inner modules do not `use` each other. `engine.rs` wires them together; `engine/*.rs` holds the rest of `Engine`.

| Module | Role |
| --- | --- |
| `composer` | Buffer for the current word: `raw`, `vi`, word id ([02](02-keyboard-io.md)) |
| `hotkey` | Double-Shift / Ctrl+Space detector ([02](02-keyboard-io.md)) |
| `labeler` | Commits, focus prior and corrections become labels and `Example`s ([04](04-engine-ml.md)) |
| `trainer` | Replay buffer, correction window, snapshots, stats; background or inline |
| `logger` | Serializes `Record` into the `EventSink` |
| `engine/flow.rs`, `machine.rs` | Per-key loop and the 4-state machine |
| `engine/switch.rs` | `manual_switch`: hotkey handling, flipping the pending word |
| `engine/model.rs` | Model switch, reset, checkpoints, per-model online training and stats |

Threads are spawned only in `trainer` and `logger` (lint `hot-path-no-io`).

## Public API of `nimk-engine`

`Engine` is the only entry point for FFI, replay and synth. It is not `Sync`; call it from one thread.

```rust
impl Engine {
    pub fn new(cfg: EngineConfig, storage: Arc<dyn Storage>, session: SessionInfo) -> Self;
    pub fn with_model(cfg, storage, session, learner: Box<dyn ImeLearner>) -> Self;
    pub fn with_models(cfg, storage, session, learners: Vec<(ModelKind, Box<dyn ImeLearner>)>) -> Self;
    pub fn on_context(&mut self, ctx: &AppContext) -> EngineOutput;
    pub fn on_key(&mut self, key: &KeyEvent) -> EngineOutput;
    pub fn teach(&mut self, key: &KeyEvent, target: Action, edit_weight: f32, render_weight: f32)
        -> EngineOutput;
    pub fn commit_pending(&mut self, timestamp: f64) -> EngineOutput;
    pub fn idle_commit_secs(&self) -> f64;
    pub fn note_navigation(&mut self);
    pub fn reconfigure(&mut self, cfg: EngineConfig);
    pub fn switch_model(&mut self, kind: ModelKind, timestamp: f64) -> EngineOutput;
    pub fn reset_model(&mut self, timestamp: f64) -> EngineOutput;
    pub fn model_kind(&self) -> ModelKind;
    pub fn checkpoint_source(&self, kind: ModelKind) -> CheckpointSource; // User | Bundled | None
    pub fn model_stats(&self, kind: ModelKind) -> ModelStats;
    pub fn online_training(&self, kind: ModelKind) -> bool;
    pub fn set_online_training(&mut self, kind: ModelKind, on: bool);
    pub fn inject_label(&mut self, target: Target, label: Lang, weight: f32, source: LabelSource);
    pub fn mode(&self) -> Lang;
    pub fn policy_id(&self) -> Arc<str>;
    pub fn take_diagnostics(&mut self) -> Vec<String>;
    pub fn shutdown(self) -> Result<(), EngineError>;
}

pub struct EngineOutput {
    pub consumed: bool,
    pub commit: Option<String>,
    pub marked: Option<String>,
    pub mode: Option<Lang>,
    pub overlay: Option<Overlay>,
}
```

| Behavior | Detail |
| --- | --- |
| `new` never fails | Reads blob `settings/model` (overrides `cfg.policy`) and `settings/models` (per-model `online_training`), then loads each `model/<kind>/latest`. Missing or corrupt: fresh model from `cfg.policy` (default `gru_action`). Failed sink: null sink. Both go to `take_diagnostics` |
| `new` logs `session_start` | The first record holds `EngineConfig` as JSON so replay can rebuild it |
| `with_model` | Loads no snapshot and has no shadow model; for benchmarks, pretrain and tests |
| `with_models` | The `cfg.policy` learner serves; the others are shadows while their online training is on ([ADR 0013](adr/0013-shadow-training-and-model-stats.md)) |
| `on_key` | Never panics; an internal error gives `consumed = false` |
| `teach` | Like `on_key`, but applies `target` instead of the model's choice and queues an `Example`. The model is not called. Used for pretraining |
| `on_context` | A new app or `field_id` commits the current word (`Boundary::External`), sets a new focus id and a prior |
| `switch_model` | Commits like `commit_pending`, settles pending labels, saves the old model, loads the new one, writes `settings/model`, logs `model_switch`. Same kind only commits |
| `shutdown` | Settles pending labels, trains the `teach` remainder, flushes the log, saves snapshots. Only error: `EngineError::Storage` |

`mode()` is the sticky mode: the mode the next word starts in, not the rendering of the word being typed.

## Clock and determinism

The engine never reads the wall clock. Its only clock is `KeyEvent::timestamp` / `AppContext::timestamp` (seconds since boot).
The host supplies wall time (`SessionInfo::wall_unix_ms`), and the storage sink reads it to name segments.

| Who | May read wall clock? | Enforced by |
| --- | --- | --- |
| `nimk-types`, `nimk-telex`, `nimk-policy` | No (also no I/O, threads, locks) | `pure-no-side-effects` |
| `nimk-engine` | No (`std::time`, `std::env` banned) | `engine-event-clock` |
| `nimk-storage` (sink) | Yes, to name segments | none |
| `nimk-ffi` | Yes, to build `SessionInfo` | none |

The same log and config therefore replay to the same decisions. `GruAction` keeps its own seeded RNG because `burn`'s device RNG is process-global.

## C ABI

`nimk-ffi` exports 19 functions. The header is generated by `make header` (cbindgen) into `target/nimk_ffi.h`.

| Group | Functions |
| --- | --- |
| Lifecycle | `nimk_new(data_dir, models_dir, app_version)`, `nimk_free`, `nimk_shutdown` |
| Input | `nimk_context`, `nimk_key`, `nimk_commit_pending`, `nimk_note_navigation` |
| Model | `nimk_set_model`, `nimk_model`, `nimk_reset_model`, `nimk_checkpoint_source`, `nimk_model_stats`, `nimk_set_online_training`, `nimk_online_training` |
| State and config | `nimk_mode`, `nimk_policy_id`, `nimk_take_diagnostics`, `nimk_set_config`, `nimk_idle_commit_secs` |

- `nimk_set_config` takes the user's `config.toml` text (or null), merges it onto the embedded defaults, and returns the number of rejected values ([11](11-config.md)).
- `nimk_idle_commit_secs` feeds the Swift idle timer (0 = off; varies by app category).
- `nimk_note_navigation` reports a mouse caret move and leaves `Raw`.
- `kind` codes: `NIMK_MODEL_LINEAR` = 0, `NIMK_MODEL_GRU` = 1. `nimk_set_model` returns `false` for an unknown kind or null core.
- `nimk_checkpoint_source` returns `NIMK_CHECKPOINT_NONE` (0), `BUNDLED` (1) or `USER` (2). `nimk_reset_model` deletes the user checkpoint and reloads the bundled model.
- `nimk_model_stats` fills `NimkModelStats` with per-model counters (aggregate only, [05](05-storage-and-log.md)).

`NimkOut` mirrors `EngineOutput`. Swift applies `commit` (`insertText`) first, then `marked` (`setMarkedText`) ([ADR 0011](adr/0011-pending-word-commit.md), [ADR 0012](adr/0012-unified-boundary-commit.md)).

| `NimkOut` field | Meaning |
| --- | --- |
| `consumed` | `false`: give the key back to the app untouched |
| `commit`, `marked` | Null = unchanged; `marked = ""` clears the preedit |
| `mode` | `NIMK_MODE_EN` (0), `NIMK_MODE_VI` (1), `NIMK_MODE_UNCHANGED` (255) |
| `overlay` | `NIMK_OVERLAY_NONE` (0), `NIMK_OVERLAY_AUTO` (1), `NIMK_OVERLAY_MANUAL` (2) |

FFI rules:

- Every function wraps `catch_unwind`; on error it returns `NimkOut::pass_through()` (`consumed = false`). Panics go to diagnostics (at most 256 lines).
- Strings in `NimkOut` live until the next call; Swift copies them at once.
- `nimk_new` returns null if the engine cannot be built. Every other function accepts null and passes keys through.
- If the data dir cannot be opened, the core uses `MemStorage` and reports it in diagnostics.
- Swift takes `data_dir` from the SDK. `models_dir` is the bundled model folder; if null or missing, no bundled model is used ([ADR 0010](adr/0010-sdk-storage-and-bundled-models.md)).

### Storage layers

`nimk_new` builds `LayeredStorage(PlainFsStorage(data_dir), ReadOnlyDirBlobs(models_dir))`. The engine still holds only `Arc<dyn Storage>`.

```text
 Swift: FileManager                     Swift: Bundle.main.resourceURL
 Application Support/<bundle id>        Contents/Resources/models
        | data_dir                             | models_dir
        v                                      v
+-----------------------------------------------------------+
| LayeredStorage                                            |
|  events    --> PlainFsStorage (user)                      |
|  put/delete--> PlainFsStorage (user)                      |
|  get       --> user, if absent --> ReadOnlyDirBlobs       |
|  list      --> union of both, sorted                      |
+-----------------------------------------------------------+
```

Learned checkpoints always go to the user layer and shadow the bundled one; deleting it restores the bundled model.
