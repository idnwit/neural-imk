//! IME engine: composer, hotkey detection, decisions, labelling, training
//! and event logging. No AppKit; driven entirely by [`KeyEvent`] and
//! [`AppContext`] values so it can be tested and replayed.
//!
//! Internal modules (implementation detail, see docs/04-engine-ml.md):
//! - `composer`: raw keys of the current word and its VI/EN renderings (a
//!   dumb buffer: every edit is decided by the action model, ADR 0007).
//! - `hotkey`: switch hotkey state machine (double Shift tap, Ctrl+Space).
//! - `labeler`: turns commits and manual switches into [`nimk_types::Example`]s.
//! - `trainer`: replay buffer + correction window; background or inline.
//! - `logger`: serializes [`nimk_types::Record`]s to an `EventSink`.
//!
//! Internal modules are independent of each other; only `engine` wires
//! them together (enforced by arch-lint). Threads live only in `trainer`
//! and `logger`. The trainer publishes `Arc<dyn ImeModel>` snapshots over a
//! channel; the engine drains it with `try_recv` at word boundaries, so the
//! hot path never locks.
#![forbid(unsafe_code)]

mod composer;
mod config;
mod engine;
mod hotkey;
mod labeler;
mod logger;
mod trainer;

pub use config::{
    AppCategory, CategoryParams, DEFAULT_TOML, Diagnostic, EngineConfig, ModelOptions,
    ModelsConfig, Threading,
};
pub use engine::{CheckpointSource, Engine, EngineError, EngineOutput, Overlay, SessionInfo};
pub use nimk_policy::ModelKind;
pub use nimk_types::{AppContext, KeyEvent, LabelSource, Lang, Target};
pub use trainer::{ModelStats, StatCategory};
