use std::sync::Arc;

use nimk_engine::{Engine, EngineConfig, SessionInfo, Threading};
use nimk_policy::ModelKind;
use nimk_storage::{MemStorage, Storage};

use super::*;
use crate::{Corpus, MixProfile, SynthConfig, generate};

fn scripts(seed: u64, sessions: usize, words: usize) -> Vec<Script> {
    let cfg = SynthConfig {
        seed,
        sessions,
        words_per_session: words,
        mix: MixProfile {
            p_typo: 0.02,
            p_shortcut: 0.02,
            ..MixProfile::default()
        },
        ..SynthConfig::default()
    };
    generate(&cfg, &Corpus::builtin())
}

fn run(
    kind: ModelKind,
    seed: u64,
    sessions: usize,
    words: usize,
    opts: PretrainOptions,
) -> (PretrainReport, MemStorage) {
    let cfg = EngineConfig {
        policy: kind,
        ..EngineConfig::default()
    };
    let storage = MemStorage::new();
    let report = pretrain(
        &scripts(seed, sessions, words),
        cfg,
        opts,
        Arc::new(storage.clone()),
    )
    .unwrap();
    (report, storage)
}

fn assert_snapshot_loads(kind: ModelKind, storage: MemStorage) {
    assert!(storage.blobs().get(&snapshot_key(kind)).unwrap().is_some());
    // Pre-training does not write events.
    assert!(
        storage
            .event_source()
            .unwrap()
            .segments()
            .unwrap()
            .is_empty()
    );
    let cfg = EngineConfig {
        policy: kind,
        hotkeys: false,
        threading: Threading::Inline,
        ..EngineConfig::default()
    };
    let session = SessionInfo {
        id: "after-pretrain".into(),
        wall_unix_ms: 0,
        app_version: "test".into(),
        utc_offset_min: 0,
    };
    let mut engine = Engine::new(cfg, Arc::new(storage), session);
    assert_eq!(engine.take_diagnostics(), Vec::<String>::new());
}

#[test]
fn linear_action_clones_behaviour_and_saves_snapshot() {
    let opts = PretrainOptions {
        epochs: 3,
        eval_frac: 0.2,
        ..PretrainOptions::default()
    };
    let (r, storage) = run(ModelKind::LinearAction, 5, 20, 150, opts);
    assert_eq!((r.epochs, r.train_scripts, r.eval.scripts), (3, 16, 4));
    let e = &r.eval;
    assert!(e.keys > 1000 && e.words >= 4 * 150, "{r:?}");
    assert!(e.ambiguous > 20, "{r:?}");
    assert!(e.edit_accuracy() > 0.95, "{r:?}");
    assert!(e.render_accuracy() > 0.6, "{r:?}");
    assert!(e.ambiguous_accuracy() > 0.55, "{r:?}");
    assert!(e.text_match_rate() > 0.5, "{r:?}");
    assert_snapshot_loads(ModelKind::LinearAction, storage);
}

#[test]
fn deterministic_with_fixed_seed() {
    let opts = PretrainOptions {
        epochs: 1,
        eval_frac: 0.25,
        ..PretrainOptions::default()
    };
    let (a, sa) = run(ModelKind::LinearAction, 11, 8, 60, opts);
    let (b, sb) = run(ModelKind::LinearAction, 11, 8, 60, opts);
    assert_eq!(a, b);
    let key = snapshot_key(ModelKind::LinearAction);
    assert_eq!(sa.blobs().get(&key).unwrap(), sb.blobs().get(&key).unwrap());
}

#[test]
fn empty_input_and_no_eval_report_zero() {
    let cfg = EngineConfig {
        policy: ModelKind::LinearAction,
        ..EngineConfig::default()
    };
    let r = pretrain(
        &[],
        cfg.clone(),
        PretrainOptions::default(),
        Arc::new(MemStorage::new()),
    )
    .unwrap();
    assert_eq!(
        (r.train_scripts, r.eval.keys, r.eval.edit_accuracy()),
        (0, 0, 0.0)
    );

    let opts = PretrainOptions {
        epochs: 1,
        eval_frac: 0.0,
        ..PretrainOptions::default()
    };
    let storage = MemStorage::new();
    let r = pretrain(&scripts(1, 2, 20), cfg, opts, Arc::new(storage.clone())).unwrap();
    assert_eq!((r.train_scripts, r.eval.scripts), (2, 0));
    assert_snapshot_loads(ModelKind::LinearAction, storage);
}

#[test]
fn eval_split_keeps_both_sides_non_empty() {
    assert_eq!(eval_count(10, 0.1), 1);
    assert_eq!(eval_count(10, 0.0), 0);
    assert_eq!(eval_count(10, 1.0), 9);
    assert_eq!(eval_count(1, 0.5), 0);
    assert_eq!(eval_count(3, 0.01), 1);
    assert_eq!(eval_count(3, f64::NAN), 0);
}

#[test]
fn gru_action_learns_edits_on_a_small_run() {
    let opts = PretrainOptions {
        epochs: 1,
        eval_frac: 0.25,
        ..PretrainOptions::default()
    };
    let (r, storage) = run(ModelKind::GruAction, 3, 4, 25, opts);
    assert_eq!(r.eval.scripts, 1);
    assert!(r.eval.edit_accuracy() > 0.6, "{r:?}");
    assert_snapshot_loads(ModelKind::GruAction, storage);
}
