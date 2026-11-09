//! The model stage: hysteresis, lock and commit-step rule.
#![allow(clippy::unwrap_used)]
use std::sync::Arc;

use nimk_policy::{ImeModel, ModelInput, ModelTransform, hysteresis};
use nimk_types::{ActionMask, ActionScores, EditAction, Lang, Observation, Transform};

/// Fixed-probability model.
struct P(f32);
impl ImeModel for P {
    fn id(&self) -> &str {
        "p@v0"
    }
    fn score(&self, _o: &Observation) -> ActionScores {
        ActionScores {
            p_vi: self.0,
            ..ActionScores::default()
        }
    }
}

fn input(base: Lang, locked: bool, structural: Option<EditAction>, prior: bool) -> ModelInput {
    let obs = Observation {
        mask: ActionMask {
            append: true,
            pop: false,
            commit: false,
        },
        ..Observation::default()
    };
    ModelInput {
        obs,
        base,
        locked,
        margin: 0.1,
        structural,
        prior,
    }
}

#[test]
fn hysteresis_holds_inside_margin() {
    assert_eq!(hysteresis(Lang::En, 0.55, 0.1), (Some(0.55), Lang::En));
    assert_eq!(hysteresis(Lang::En, 0.7, 0.1), (Some(0.7), Lang::Vi));
    assert_eq!(hysteresis(Lang::Vi, f32::NAN, 0.1), (None, Lang::Vi));
}

#[test]
fn lock_keeps_base_and_commit_uses_raw_p() {
    let t = ModelTransform::new(Arc::new(P(0.55)));
    let locked = t.apply(&input(Lang::En, true, Some(EditAction::Append), false));
    assert_eq!(locked.action.render, Lang::En);
    let commit = t.apply(&input(Lang::En, false, Some(EditAction::Commit), false));
    assert_eq!(commit.action.render, Lang::Vi);
    assert_eq!(commit.action.edit, EditAction::Commit);
}

#[test]
fn prior_uses_hysteresis_only() {
    let t = ModelTransform::new(Arc::new(P(0.9)));
    let out = t.apply(&input(Lang::En, false, None, true));
    assert_eq!(out.action.render, Lang::Vi);
    assert_eq!(out.action.edit, EditAction::Pass);
}
