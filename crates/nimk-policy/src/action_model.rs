//! Neural action model contract (ADR 0007). Replaces the `Policy`/`Learner`
//! p(VI) API once the engine migrates; both coexist during the refactor.

use std::str::FromStr;
use std::sync::Arc;

use nimk_types::{Action, ActionMask, ActionScores, EditAction, Example, Lang, Observation};

use crate::PolicyError;
use crate::gru_action::GruAction;
use crate::linear_action::LinearAction;

/// Read-only action model used on the hot path.
pub trait ImeModel: Send + Sync {
    /// Identifier written to the log, e.g. `gru_action@v1`.
    fn id(&self) -> &str;

    /// Scores both heads for one observation. Must not block; p99 < 1 ms.
    fn score(&self, obs: &Observation) -> ActionScores;
}

/// Mutable model owned by the trainer thread.
pub trait ImeLearner: Send {
    /// Identifier of the current weights.
    fn id(&self) -> &str;

    /// One online update; returns the weighted loss of the batch measured
    /// before the update (edit + render heads, each scaled by its weight).
    fn update(&mut self, batch: &[Example]) -> f32;

    /// Whether online labels that only confirm what was committed (implicit
    /// labels, injected labels equal to the committed rendering) should be
    /// trained on. When `false` the trainer drops such render-only batches
    /// and the model learns online from corrections only. Default `true`.
    fn learns_from_confirmations(&self) -> bool {
        true
    }

    /// Read-only copy for the hot path; must not change after publication.
    fn snapshot(&self) -> Arc<dyn ImeModel>;

    /// Serializes weights (and optimizer state if available) with a header
    /// naming kind and format version.
    fn save(&self) -> Vec<u8>;

    /// Restores bytes produced by [`ImeLearner::save`].
    ///
    /// # Errors
    /// [`PolicyError::Corrupt`] on kind/version/shape mismatch.
    fn load(&mut self, bytes: &[u8]) -> Result<(), PolicyError>;
}

/// Available action models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    /// Two linear heads over hashed features (hand-written, fast tests).
    LinearAction,
    /// Char-level GRU over the recent window + context embeddings (burn).
    GruAction,
}

impl ModelKind {
    /// Stable snake_case name (blob keys, settings, `FromStr`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::LinearAction => "linear_action",
            Self::GruAction => "gru_action",
        }
    }
}

impl FromStr for ModelKind {
    type Err = PolicyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "linear_action" => Ok(Self::LinearAction),
            "gru_action" => Ok(Self::GruAction),
            other => Err(PolicyError::UnknownKind(other.to_owned())),
        }
    }
}

/// Builds a fresh, untrained action model. Untrained models are neutral:
/// `p_vi` within 0.5 +- 0.05 and near-uniform edit probabilities.
/// Deterministic for a given `seed` (`LinearAction` ignores it: it starts
/// at all-zero weights).
#[must_use]
pub fn build_action(kind: ModelKind, seed: u64) -> Box<dyn ImeLearner> {
    match kind {
        ModelKind::LinearAction => Box::new(LinearAction::new()),
        ModelKind::GruAction => Box::new(GruAction::new(seed)),
    }
}

/// Turns scores into an action: the most probable edit among those `mask`
/// allows (`Pass` is always legal; ties go to the earlier edit in
/// [`EditAction::ALL`]) and `Vi` iff `p_vi >= 0.5`.
///
/// This is the mechanical masking step the engine applies; models are
/// trained on unmasked logits.
#[must_use]
pub fn decide(scores: &ActionScores, mask: ActionMask) -> Action {
    let mut best = EditAction::Pass;
    let mut best_p = f32::NEG_INFINITY;
    for edit in EditAction::ALL {
        let p = scores.edit[edit.index()];
        if mask.allows(edit) && p > best_p {
            best = edit;
            best_p = p;
        }
    }
    Action {
        edit: best,
        render: Lang::from_p_vi(scores.p_vi),
    }
}
