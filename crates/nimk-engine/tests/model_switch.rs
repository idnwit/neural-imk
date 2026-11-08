//! Runtime model selection: separate checkpoints, persisted choice.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::*;
use nimk_engine::{CheckpointSource, Engine, EngineConfig, Threading};
use nimk_policy::ModelKind;
use nimk_storage::{LayeredStorage, MemStorage, PlainFsStorage, ReadOnlyDirBlobs, Storage};
use nimk_types::Event;

const LIN: &str = "model/linear_action/latest";
const GRU: &str = "model/gru_action/latest";

fn engine(storage: &MemStorage, threading: Threading) -> Engine {
    let cfg = EngineConfig {
        threading,
        ..inline_cfg()
    };
    Engine::new(cfg, Arc::new(storage.clone()), session())
}

/// Types and corrects a few words so the model learns.
fn train(e: &mut Engine, t: &mut f64) {
    e.on_context(&ctx("com.apple.TextEdit", 1, *t));
    type_str(e, "tieengs Vieetj has ", t);
    double_shift(e, t);
    type_str(e, "for test ", t);
    e.commit_pending(*t + 20.0);
    *t += 20.0;
}

fn blob(s: &MemStorage, key: &str) -> Option<Vec<u8>> {
    s.blobs().get(key).unwrap()
}

#[test]
fn checkpoints_are_separate_and_restored() {
    for threading in [Threading::Inline, Threading::Background] {
        let storage = MemStorage::new();
        let mut e = engine(&storage, threading);
        assert_eq!(e.model_kind(), ModelKind::LinearAction);
        assert!(!e.has_checkpoint(ModelKind::LinearAction));
        let mut t = 1.0;
        train(&mut e, &mut t);
        let id_a = e.policy_id();

        e.switch_model(ModelKind::GruAction, t + 1.0);
        assert_eq!(e.model_kind(), ModelKind::GruAction);
        assert!(e.has_checkpoint(ModelKind::LinearAction));
        assert!(!e.has_checkpoint(ModelKind::GruAction));
        assert!(e.policy_id().starts_with("gru_action"));
        let saved_a = blob(&storage, LIN).unwrap();
        assert!(e.take_diagnostics().is_empty());

        e.switch_model(ModelKind::LinearAction, t + 2.0);
        assert!(e.has_checkpoint(ModelKind::GruAction));
        assert_eq!(e.policy_id(), id_a, "{threading:?}");
        assert!(e.take_diagnostics().is_empty());
        e.shutdown().unwrap();
        assert_eq!(blob(&storage, LIN).unwrap(), saved_a, "{threading:?}");
        assert!(blob(&storage, GRU).is_some());
    }
}

#[test]
fn choice_persists_across_engines() {
    let storage = MemStorage::new();
    let mut e = engine(&storage, Threading::Inline);
    e.switch_model(ModelKind::GruAction, 1.0);
    e.shutdown().unwrap();
    assert_eq!(
        blob(&storage, "settings/model").unwrap(),
        br#"{"model":"gru_action"}"#
    );
    let e2 = engine(&storage, Threading::Inline);
    assert_eq!(e2.model_kind(), ModelKind::GruAction);
    assert!(e2.policy_id().starts_with("gru_action"));

    storage.blobs().put("settings/model", b"{nope").unwrap();
    let mut e3 = engine(&storage, Threading::Inline);
    assert_eq!(e3.model_kind(), ModelKind::LinearAction);
    assert!(
        e3.take_diagnostics()
            .iter()
            .any(|d| d.contains("settings/model"))
    );
}

#[test]
fn switch_is_logged_and_commits_pending_word() {
    let storage = MemStorage::new();
    let mut e = engine(&storage, Threading::Inline);
    let mut t = 1.0;
    e.on_context(&ctx("com.apple.TextEdit", 1, t));
    let outs = type_str(&mut e, "abc", &mut t);
    assert!(commits(&outs).is_empty());
    let out = e.switch_model(ModelKind::GruAction, t + 0.1);
    assert!(out.commit.as_deref().is_some_and(|c| !c.is_empty()));
    assert_eq!(out.marked, None);
    // Same kind: only commits, no event.
    let again = e.switch_model(ModelKind::GruAction, t + 0.2);
    assert_eq!(again.commit, None);
    e.shutdown().unwrap();
    let switches: Vec<_> = events(&storage)
        .into_iter()
        .filter_map(|ev| match ev {
            Event::ModelSwitch { from, to } => Some((from, to)),
            _ => None,
        })
        .collect();
    assert_eq!(switches.len(), 1, "{switches:?}");
    assert!(switches[0].0.starts_with("linear_action"));
    assert!(switches[0].1.starts_with("gru_action"));
}

#[test]
fn corrupt_target_checkpoint_falls_back_fresh() {
    let storage = MemStorage::new();
    storage.blobs().put(GRU, b"garbage").unwrap();
    let mut e = engine(&storage, Threading::Inline);
    e.switch_model(ModelKind::GruAction, 1.0);
    assert_eq!(e.model_kind(), ModelKind::GruAction);
    let d = e.take_diagnostics();
    assert!(d.iter().any(|m| m.contains("starting fresh")), "{d:?}");
    // The fresh model keeps working.
    let mut t = 2.0;
    train(&mut e, &mut t);
    e.shutdown().unwrap();
    assert_ne!(blob(&storage, GRU).unwrap(), b"garbage");
}

/// Bundled defaults in a temp dir holding a trained linear checkpoint.
fn bundled_dir() -> (tempfile::TempDir, std::path::PathBuf, Vec<u8>) {
    let scratch = MemStorage::new();
    let mut e = engine(&scratch, Threading::Inline);
    let mut t = 1.0;
    train(&mut e, &mut t);
    e.shutdown().unwrap();
    let bytes = blob(&scratch, LIN).unwrap();
    let dir = tempfile::tempdir().unwrap();
    PlainFsStorage::open(dir.path())
        .unwrap()
        .blobs()
        .put(LIN, &bytes)
        .unwrap();
    let models = dir.path().join("blobs");
    (dir, models, bytes)
}

fn layered(
    user: &MemStorage,
    dir: &std::path::Path,
) -> Arc<LayeredStorage<MemStorage, ReadOnlyDirBlobs>> {
    Arc::new(LayeredStorage::new(
        user.clone(),
        ReadOnlyDirBlobs::open(dir).unwrap(),
    ))
}

#[test]
fn starts_from_bundled_then_user_then_reset_returns_to_bundled() {
    for threading in [Threading::Inline, Threading::Background] {
        let (_dir, models, bundled) = bundled_dir();
        let user = MemStorage::new();
        let cfg = EngineConfig {
            threading,
            ..inline_cfg()
        };
        let mut e = Engine::new(cfg.clone(), layered(&user, &models), session());
        assert_eq!(
            e.checkpoint_source(ModelKind::LinearAction),
            CheckpointSource::Bundled
        );
        assert_eq!(
            e.checkpoint_source(ModelKind::GruAction),
            CheckpointSource::None
        );
        assert!(e.has_checkpoint(ModelKind::LinearAction));
        assert!(e.take_diagnostics().is_empty(), "bundled loads cleanly");
        let bundled_id = e.policy_id();

        let mut t = 1.0;
        train(&mut e, &mut t);
        e.switch_model(ModelKind::GruAction, t + 1.0);
        assert_eq!(
            e.checkpoint_source(ModelKind::LinearAction),
            CheckpointSource::User
        );
        assert_ne!(blob(&user, LIN).unwrap(), bundled, "{threading:?}");
        e.switch_model(ModelKind::LinearAction, t + 2.0);
        e.take_diagnostics();

        // Pending word is committed, user checkpoint removed, bundled reloaded.
        e.on_context(&ctx("com.apple.TextEdit", 2, t + 3.0));
        let mut tt = t + 3.0;
        type_str(&mut e, "abc", &mut tt);
        let out = e.reset_model(tt + 0.1);
        assert!(out.commit.as_deref().is_some_and(|c| !c.is_empty()));
        assert_eq!(
            e.checkpoint_source(ModelKind::LinearAction),
            CheckpointSource::Bundled
        );
        assert_eq!(blob(&user, LIN), None, "{threading:?}");
        assert_eq!(e.policy_id(), bundled_id);
        let d = e.take_diagnostics();
        assert!(
            d.iter().any(|m| m.contains("user checkpoint deleted")),
            "{d:?}"
        );

        // Learning continues and saves to the user layer again.
        train(&mut e, &mut tt);
        e.shutdown().unwrap();
        assert!(blob(&user, LIN).is_some(), "{threading:?}");
    }
}

#[test]
fn reset_without_any_default_starts_fresh() {
    let storage = MemStorage::new();
    let mut e = engine(&storage, Threading::Inline);
    let mut t = 1.0;
    train(&mut e, &mut t);
    e.reset_model(t + 1.0);
    assert_eq!(
        e.checkpoint_source(ModelKind::LinearAction),
        CheckpointSource::None
    );
    assert_eq!(blob(&storage, LIN), None);
    // Idempotent: the outgoing trainer re-saves, then the reset deletes again.
    e.reset_model(t + 2.0);
    assert_eq!(blob(&storage, LIN), None);
    let d = e.take_diagnostics();
    assert!(d.iter().all(|m| m.starts_with("reset ")), "{d:?}");
}
