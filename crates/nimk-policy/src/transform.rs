//! The model stage: scores the observation and picks the action, with the
//! display hysteresis, the manual-switch lock and the commit-step rule.

use std::sync::Arc;

use nimk_types::{Action, EditAction, Lang, Observation, Transform};

use crate::{ImeModel, decide};

/// Input of [`ModelTransform`].
#[derive(Debug, Clone)]
pub struct ModelInput {
    /// What the model sees.
    pub obs: Observation,
    /// Rendering shown before this key (or the sticky mode for a prior).
    pub base: Lang,
    /// A manual switch locked the rendering for this word.
    pub locked: bool,
    /// Hysteresis margin in force.
    pub margin: f32,
    /// The only legal edit, when the mask leaves no choice.
    pub structural: Option<EditAction>,
    /// Focus prior: only the rendering matters; edit is `Pass`.
    pub prior: bool,
}

/// Output of [`ModelTransform`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelOutput {
    /// Chosen edit and rendering.
    pub action: Action,
    /// Sanitized probability for the log; `None` if not finite.
    pub p_vi: Option<f32>,
}

/// Hysteresis: leave `base` only if `|p - 0.5| > margin`. Returns the
/// sanitized probability for the log (`None` if not finite) and the chosen
/// rendering.
#[must_use]
pub fn hysteresis(base: Lang, p: f32, margin: f32) -> (Option<f32>, Lang) {
    if !p.is_finite() {
        return (None, base);
    }
    let p = p.clamp(0.0, 1.0);
    let chosen = if (p - 0.5).abs() > margin {
        Lang::from_p_vi(p)
    } else {
        base
    };
    (Some(p), chosen)
}

/// The model stage around one read-only snapshot.
#[derive(Clone)]
pub struct ModelTransform {
    model: Arc<dyn ImeModel>,
}

impl ModelTransform {
    /// Wraps a model snapshot.
    #[must_use]
    pub fn new(model: Arc<dyn ImeModel>) -> Self {
        Self { model }
    }
}

impl Transform for ModelTransform {
    type Input = ModelInput;
    type Output = ModelOutput;

    fn apply(&self, input: &ModelInput) -> ModelOutput {
        let scores = self.model.score(&input.obs);
        let (p, held) = hysteresis(input.base, scores.p_vi, input.margin);
        if input.prior {
            return ModelOutput {
                action: Action {
                    edit: EditAction::Pass,
                    render: held,
                },
                p_vi: p,
            };
        }
        let edit = match input.structural {
            Some(edit) => edit,
            None => decide(&scores, input.obs.mask).edit,
        };
        let render = if input.locked {
            // A manual switch locks the rendering for the rest of the word.
            input.base
        } else if edit == EditAction::Commit {
            // Hysteresis only steadies the display mid-word; the committed
            // text is the model's decision at the commit step itself.
            match p {
                Some(p) => Lang::from_p_vi(p),
                None => input.base,
            }
        } else {
            held
        };
        ModelOutput {
            action: Action { edit, render },
            p_vi: p,
        }
    }
}
