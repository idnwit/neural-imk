//! Language policies: feature extraction, prediction and online learning.
//!
//! This crate knows nothing about storage, Telex or the engine. It maps
//! [`DecisionCtx`] to [`Features`] and [`Features`] to `p(VI)`.
//!
//! [`Policy`] is the read-only hot-path view; [`Learner`] owns mutable
//! weights on the trainer thread and publishes snapshots.
#![forbid(unsafe_code)]

mod baseline;
mod codec;
mod extract;
mod linear;
mod mlp;
mod mlp_net;

use std::str::FromStr;
use std::sync::Arc;

pub use baseline::Baseline;
pub use extract::{DENSE_LEN, HashedExtractor, SPARSE_BUCKETS};
pub use linear::{FtrlParams, Linear};
pub use mlp::{AdamParams, TinyMlp};
use nimk_types::{DecisionCtx, Features, Sample};

/// Turns a decision context into model input.
pub trait FeatureExtractor: Send + Sync {
    /// Fills `out` (cleared first by the implementation) from `ctx`.
    /// Must not allocate once `out` has warmed up.
    fn extract(&self, ctx: &DecisionCtx<'_>, out: &mut Features);
}

/// Read-only model used on the hot path.
pub trait Policy: Send + Sync {
    /// Identifier written to the log, e.g. `tiny_mlp@v3`.
    fn id(&self) -> &str;

    /// Probability that the user wants Vietnamese. Must not allocate or block.
    fn predict(&self, features: &Features) -> f32;
}

/// Mutable model owned by the trainer.
pub trait Learner: Send {
    /// Identifier of the current weights, e.g. `tiny_mlp@v3`.
    fn id(&self) -> &str;

    /// One online update over `batch`; returns the mean weighted log-loss
    /// of the batch measured before the update.
    fn update(&mut self, batch: &[Sample]) -> f32;

    /// Read-only copy of the current weights for the hot path.
    fn snapshot(&self) -> Arc<dyn Policy>;

    /// Serializes the weights. The bytes start with a header naming the
    /// policy kind and format version so a mismatched snapshot is detected.
    fn save(&self) -> Vec<u8>;

    /// Restores weights produced by [`Learner::save`].
    ///
    /// # Errors
    /// Returns [`PolicyError::Corrupt`] if the bytes are not a valid snapshot
    /// of this kind and version.
    fn load(&mut self, bytes: &[u8]) -> Result<(), PolicyError>;
}

/// Policy errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// Snapshot bytes could not be decoded.
    #[error("corrupt policy snapshot: {0}")]
    Corrupt(String),
    /// Unknown policy name.
    #[error("unknown policy kind: {0}")]
    UnknownKind(String),
}

/// Available policy implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyKind {
    /// Remembers the last label per app; no learning beyond that.
    Baseline,
    /// FTRL-Proximal logistic regression on hashed features.
    Linear,
    /// Embedding bag + one hidden layer MLP.
    TinyMlp,
}

impl FromStr for PolicyKind {
    type Err = PolicyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "baseline" => Ok(Self::Baseline),
            "linear" => Ok(Self::Linear),
            "tiny_mlp" => Ok(Self::TinyMlp),
            other => Err(PolicyError::UnknownKind(other.to_owned())),
        }
    }
}

/// A learner paired with the extractor whose feature layout it expects.
pub struct Model {
    /// Feature extractor matching the learner's input layout.
    pub extractor: Arc<dyn FeatureExtractor>,
    /// The learner.
    pub learner: Box<dyn Learner>,
}

/// Builds a fresh model. `seed` makes initialization deterministic.
#[must_use]
pub fn build(kind: PolicyKind, seed: u64) -> Model {
    let extractor: Arc<dyn FeatureExtractor> = Arc::new(HashedExtractor);
    let learner: Box<dyn Learner> = match kind {
        PolicyKind::Baseline => Box::new(Baseline::new()),
        PolicyKind::Linear => Box::new(Linear::new(extract::SPARSE_BUCKETS, extract::DENSE_LEN)),
        PolicyKind::TinyMlp => Box::new(TinyMlp::new(
            extract::SPARSE_BUCKETS,
            extract::DENSE_LEN,
            seed,
        )),
    };
    Model { extractor, learner }
}
