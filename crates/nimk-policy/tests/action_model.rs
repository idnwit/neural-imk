//! Untrained neutrality, snapshot round trips and masking.

mod common;

use common::{Rng, dataset, random_obs, split_words, train};
use nimk_policy::{ImeLearner, ModelKind, PolicyError, build_action, decide};
use nimk_types::{ActionMask, ActionScores, EditAction, Lang};

const KINDS: [ModelKind; 2] = [ModelKind::LinearAction, ModelKind::GruAction];

#[test]
fn untrained_models_are_neutral() {
    for kind in KINDS {
        let snap = build_action(kind, 42).snapshot();
        let mut rng = Rng(5);
        for _ in 0..200 {
            let s = snap.score(&random_obs(&mut rng));
            assert!((s.p_vi - 0.5).abs() <= 0.05, "{kind:?} p_vi {}", s.p_vi);
            for p in s.edit {
                assert!((p - 0.25).abs() <= 0.05, "{kind:?} edit {:?}", s.edit);
            }
            assert!((s.edit.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        }
    }
}

#[test]
fn build_is_deterministic_per_seed() {
    let (words, _) = split_words();
    let data = dataset(&words, 3, 9);
    for kind in KINDS {
        let mut a = build_action(kind, 11);
        let mut b = build_action(kind, 11);
        train(a.as_mut(), &data, 1, 16, 1);
        train(b.as_mut(), &data, 1, 16, 1);
        // Compare behaviour, not bytes: burn records carry random param ids.
        let (sa, sb) = (a.snapshot(), b.snapshot());
        for ex in &data {
            assert_eq!(sa.score(&ex.obs), sb.score(&ex.obs), "{kind:?}");
        }
    }
}

#[test]
fn save_load_round_trip_gives_identical_scores() {
    let (words, _) = split_words();
    let data = dataset(&words, 4, 4);
    for kind in KINDS {
        let mut a = build_action(kind, 1);
        train(a.as_mut(), &data, 2, 16, 2);
        let bytes = a.save();
        assert_eq!(&bytes[..4], b"NIMK");
        let mut b = build_action(kind, 99);
        b.load(&bytes).expect("load");
        let (sa, sb) = (a.snapshot(), b.snapshot());
        assert_eq!(sa.id(), sb.id());
        for ex in data.iter().take(100) {
            assert_eq!(sa.score(&ex.obs), sb.score(&ex.obs), "{kind:?}");
        }
        // The loaded learner keeps training.
        let before = sb.score(&data[0].obs);
        train(b.as_mut(), &data, 1, 16, 3);
        assert_ne!(b.snapshot().score(&data[0].obs), before, "{kind:?}");
    }
}

#[test]
fn load_rejects_wrong_kind_and_corruption() {
    let linear = build_action(ModelKind::LinearAction, 0).save();
    let gru = build_action(ModelKind::GruAction, 0).save();
    let mut l = build_action(ModelKind::LinearAction, 0);
    let mut g = build_action(ModelKind::GruAction, 0);
    assert!(matches!(l.load(&gru), Err(PolicyError::Corrupt(_))));
    assert!(matches!(g.load(&linear), Err(PolicyError::Corrupt(_))));
    assert!(g.load(&gru[..gru.len() - 10]).is_err());
    assert!(l.load(&linear[..linear.len() - 1]).is_err());
    let mut bad = gru.clone();
    let last = bad.len() - 1;
    bad[last] ^= 0xff;
    bad.truncate(bad.len() / 2);
    assert!(g.load(&bad).is_err());
    assert!(g.load(&gru).is_ok());
}

#[test]
fn snapshot_is_frozen() {
    let (words, _) = split_words();
    let data = dataset(&words, 4, 6);
    for kind in KINDS {
        let mut l = build_action(kind, 3);
        let snap = l.snapshot();
        let before = snap.score(&data[0].obs);
        train(l.as_mut(), &data, 1, 16, 1);
        assert_eq!(snap.score(&data[0].obs), before, "{kind:?}");
        assert_ne!(l.snapshot().score(&data[0].obs), before, "{kind:?}");
    }
}

#[test]
fn decide_masks_illegal_edits() {
    let s = ActionScores {
        edit: [0.7, 0.2, 0.06, 0.04],
        p_vi: 0.6,
    };
    let all = ActionMask {
        append: true,
        pop: true,
        commit: true,
    };
    assert_eq!(decide(&s, all).edit, EditAction::Append);
    assert_eq!(decide(&s, all).render, Lang::Vi);
    let no_append = ActionMask {
        append: false,
        ..all
    };
    assert_eq!(decide(&s, no_append).edit, EditAction::Pop);
    assert_eq!(decide(&s, ActionMask::default()).edit, EditAction::Pass);
}

#[test]
fn empty_batch_is_a_no_op() {
    for kind in KINDS {
        let mut l: Box<dyn ImeLearner> = build_action(kind, 0);
        let obs = common::random_obs(&mut Rng(1));
        let before = l.snapshot().score(&obs);
        assert_eq!(l.update(&[]), 0.0);
        assert_eq!(l.snapshot().score(&obs), before);
    }
}

#[test]
fn gru_parameter_count() {
    // tok 139x32 + GRU 3x(32x64+64 + 64x64+64) + bundle 256x16 + caret 65x8
    // + hidden 106x64+64 + edit 64x4+4 + render 64+1.
    assert_eq!(nimk_policy::GruAction::new(0).num_params(), 35_053);
}

#[test]
fn render_only_updates_never_change_edit_scores() {
    let (words, _) = split_words();
    let data = dataset(&words, 4, 8);
    for kind in KINDS {
        let mut l = build_action(kind, 5);
        train(l.as_mut(), &data, 1, 16, 4);
        let before = l.snapshot();
        let online: Vec<_> = data
            .iter()
            .take(64)
            .map(|ex| {
                let mut ex = ex.clone();
                ex.edit_weight = 0.0;
                ex.render_weight = 1.0;
                ex.target.render = ex.target.render.flip();
                ex
            })
            .collect();
        for chunk in online.chunks(8) {
            l.update(chunk);
        }
        let after = l.snapshot();
        let mut moved = false;
        for ex in data.iter().take(100) {
            let (a, b) = (before.score(&ex.obs), after.score(&ex.obs));
            assert_eq!(a.edit, b.edit, "{kind:?}: edit head moved");
            moved |= a.p_vi != b.p_vi;
        }
        assert!(moved, "{kind:?}: render head learned");
    }
}
