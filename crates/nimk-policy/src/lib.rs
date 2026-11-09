//! Neural action models (ADR 0007).
//!
//! This crate knows nothing about storage, Telex or the engine. [`ImeModel`]
//! is the read-only hot-path view and [`ImeLearner`] owns mutable weights on
//! the trainer thread and publishes snapshots. Both map an
//! [`nimk_types::Observation`] (shared fixed-size [`encode`]ing) to edit and
//! render scores; see [`LinearAction`] and [`GruAction`], built with
//! [`build_action`] and masked with [`decide`].
#![forbid(unsafe_code)]

mod action_model;
mod codec;
pub mod encode;
mod gru_action;
mod gru_net;
mod linear_action;
mod transform;

pub use action_model::{ImeLearner, ImeModel, ModelKind, build_action, decide};
pub use gru_action::{GRU_LR, GRU_ONLINE_LR, GruAction};
pub use linear_action::{ACTION_BUCKETS, LINEAR_LR, LINEAR_ONLINE_SCALE, LinearAction};
pub use transform::{ModelInput, ModelOutput, ModelTransform, hysteresis};

/// Policy errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// Snapshot bytes could not be decoded.
    #[error("corrupt policy snapshot: {0}")]
    Corrupt(String),
    /// Unknown model name.
    #[error("unknown policy kind: {0}")]
    UnknownKind(String),
}
