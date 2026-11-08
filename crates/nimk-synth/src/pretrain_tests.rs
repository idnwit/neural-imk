use std::sync::Arc;

use nimk_engine::{Engine, EngineConfig, SessionInfo};
use nimk_policy::PolicyKind;
use nimk_storage::{MemStorage, Storage};

use super::*;
use crate::{Corpus, SynthConfig, generate};

fn scripts(seed: u64) -> Vec<Script> {
    let cfg = SynthConfig {
        seed,
        sessions: 20,
        words_per_session: 200,
        ..SynthConfig::default()
    };
    generate(&cfg, &Corpus::builtin())
}

fn run(kind: PolicyKind, seed: u64) -> (PretrainReport, MemStorage) {
    let cfg = EngineConfig {
        policy: kind,
        ..EngineConfig::default()
    };
    let model = nimk_policy::build(kind, cfg.seed);
    let storage = MemStorage::new();
    let report = pretrain(&scripts(seed), cfg, model, Arc::new(storage.clone())).unwrap();
    (report, storage)
}

fn assert_learns(kind: PolicyKind) {
    let (r, storage) = run(kind, 5);
    assert_eq!(r.words, 20 * 200);
    assert!(r.ambiguous > 200, "{r:?}");
    assert!(r.correct <= r.ambiguous);
    assert!(
        r.tail_accuracy > r.head_accuracy + 0.05,
        "{kind:?} did not improve: {r:?}"
    );
    assert!(r.tail_accuracy > 0.8, "{kind:?}: {r:?}");

    // The snapshot is where the IME looks, and loads cleanly.
    let key = format!("model/{}/latest", kind_name(kind));
    assert!(storage.blobs().get(&key).unwrap().is_some());
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
    };
    let mut engine = Engine::new(cfg, Arc::new(storage), session);
    assert_eq!(engine.take_diagnostics(), Vec::<String>::new());
}

#[test]
fn tiny_mlp_learns_and_saves_snapshot() {
    assert_learns(PolicyKind::TinyMlp);
}

#[test]
fn linear_learns_and_saves_snapshot() {
    assert_learns(PolicyKind::Linear);
}

#[test]
fn deterministic_with_fixed_seed() {
    let (a, sa) = run(PolicyKind::Linear, 11);
    let (b, sb) = run(PolicyKind::Linear, 11);
    assert_eq!(a, b);
    let key = "model/linear/latest";
    assert_eq!(sa.blobs().get(key).unwrap(), sb.blobs().get(key).unwrap());
}

#[test]
fn rejects_model_of_another_kind() {
    let cfg = EngineConfig {
        policy: PolicyKind::Linear,
        ..EngineConfig::default()
    };
    let model = nimk_policy::build(PolicyKind::TinyMlp, 1);
    let err = pretrain(&[], cfg, model, Arc::new(MemStorage::new())).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn empty_input_reports_zero() {
    let cfg = EngineConfig::default();
    let model = nimk_policy::build(cfg.policy, cfg.seed);
    let r = pretrain(&[], cfg, model, Arc::new(MemStorage::new())).unwrap();
    assert_eq!((r.words, r.ambiguous, r.accuracy()), (0, 0, 0.0));
}
