//! End-to-end tests: write a log with a real engine, then replay it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use nimk_engine::{AppContext, Engine, EngineConfig, KeyEvent, SessionInfo, Threading};
use nimk_policy::ModelKind;
use nimk_replay::{ReplayConfig, ReplayError, Report, render_table, replay_dir};
use nimk_storage::{PlainFsStorage, Storage};
use nimk_types::{KeyKind, Modifiers, keycode};

fn key(chars: &str, keycode: u16, t: f64) -> KeyEvent {
    KeyEvent {
        kind: KeyKind::KeyDown,
        keycode,
        chars: chars.to_owned(),
        mods: Modifiers(0),
        timestamp: t,
    }
}

fn shift(down: bool, t: f64) -> KeyEvent {
    KeyEvent {
        kind: KeyKind::FlagsChanged,
        keycode: keycode::LEFT_SHIFT,
        chars: String::new(),
        mods: Modifiers(if down { Modifiers::SHIFT } else { 0 }),
        timestamp: t,
    }
}

/// Types `words` (space-separated, `^` = Left-Shift tap) into a hotkey-enabled engine.
fn write_session(storage: Arc<dyn Storage>, session: &str, script: &str, t0: f64) {
    let cfg = EngineConfig {
        policy: ModelKind::LinearAction,
        threading: Threading::Inline,
        ..EngineConfig::default()
    };
    let info = SessionInfo {
        id: session.to_owned(),
        wall_unix_ms: 1_794_111_700_000,
        app_version: "test".to_owned(),
    };
    let mut engine = Engine::new(cfg, storage, info);
    let mut t = t0;
    engine.on_context(&AppContext {
        bundle_id: "com.test.app".to_owned(),
        field_id: 1,
        caret: None,
        timestamp: t,
    });
    for word in script.split(' ') {
        if word == "^" {
            t += 0.5;
            engine.on_key(&shift(true, t));
            t += 0.05;
            engine.on_key(&shift(false, t));
            continue;
        }
        for c in word.chars() {
            t += 0.1;
            engine.on_key(&key(&c.to_string(), 0, t));
        }
        t += 0.1;
        engine.on_key(&key(" ", keycode::SPACE, t));
    }
    engine.shutdown().unwrap();
}

const SCRIPT: &str = "vieetj hello ^ xin chaof the world toi ddi hocj";
const WORDS: u64 = 9;

fn linear(root: &Path) -> ReplayConfig {
    ReplayConfig {
        models: vec![ModelKind::LinearAction],
        from: None,
        seed: None,
        init: Some(root.to_path_buf()),
        frozen: false,
    }
}

/// Behaviour-clones a small `linear_action` model into `storage` (an
/// untrained model never commits words).
fn pretrain_linear(storage: &Arc<dyn Storage>) {
    let synth = nimk_synth::SynthConfig {
        seed: 9,
        sessions: 6,
        words_per_session: 80,
        ..nimk_synth::SynthConfig::default()
    };
    let scripts = nimk_synth::generate(&synth, &nimk_synth::Corpus::builtin());
    let cfg = EngineConfig {
        policy: ModelKind::LinearAction,
        ..EngineConfig::default()
    };
    let opts = nimk_synth::PretrainOptions {
        epochs: 1,
        eval_frac: 0.0,
        ..nimk_synth::PretrainOptions::default()
    };
    nimk_synth::pretrain(&scripts, cfg, opts, Arc::clone(storage)).unwrap();
}

fn write_fs_log(root: &Path) {
    let storage: Arc<dyn Storage> = Arc::new(PlainFsStorage::open(root).unwrap());
    pretrain_linear(&storage);
    write_session(Arc::clone(&storage), "s1", SCRIPT, 100.0);
    write_session(storage, "s2", SCRIPT, 200.0);
}

#[test]
fn replays_engine_log_deterministically() {
    let dir = tempfile::tempdir().unwrap();
    write_fs_log(dir.path());

    let report = replay_dir(dir.path(), &linear(dir.path())).unwrap();
    assert_eq!(report.meta.sessions, 2);
    assert_eq!(report.meta.malformed, 0);
    assert_eq!(report.meta.warning_count, 0, "{:?}", report.meta.warnings);
    assert_eq!(report.models.len(), 1);
    let m = &report.models[0].overall;
    let c = &m.counts;
    assert_eq!(c.words, 2 * WORDS);
    assert_eq!(c.labelled_words, 2 * WORDS, "{report:#?}");
    assert!(c.correct_words <= c.labelled_words);
    assert!(c.ambiguous_words <= c.labelled_words);
    assert!(c.auto_switches_agreed <= c.auto_switches_labelled);
    assert_eq!(c.priors_labelled, 2);
    assert!(m.word_accuracy.is_some_and(|a| (0.0..=1.0).contains(&a)));
    let wrong = (c.labelled_words - c.correct_words) as f64;
    assert_eq!(
        m.manual_switches_per_1000,
        Some(wrong * 1000.0 / c.words as f64)
    );
    assert_eq!(
        report.models[0].by_bundle.keys().collect::<Vec<_>>(),
        ["com.test.app"]
    );

    // Replaying the model that wrote the log reproduces its edits.
    assert!(c.edits_compared > 0);
    assert!(
        m.edit_accuracy.is_some_and(|a| a > 0.95),
        "{:?}",
        m.edit_accuracy
    );

    let again = replay_dir(dir.path(), &linear(dir.path())).unwrap();
    assert_eq!(report, again);
    let table = render_table(&report);
    assert!(table.contains("linear_action"), "{table}");
    assert!(table.contains("com.test.app"), "{table}");
}

#[test]
fn all_models_and_json_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let shared: Arc<dyn Storage> = Arc::new(PlainFsStorage::open(dir.path()).unwrap());
    pretrain_linear(&shared);
    write_session(shared, "s1", SCRIPT, 100.0);
    let cfg = ReplayConfig {
        models: vec![ModelKind::LinearAction, ModelKind::GruAction],
        seed: Some(3),
        ..linear(dir.path())
    };
    let report = replay_dir(dir.path(), &cfg).unwrap();
    assert_eq!(report.models.len(), 2);
    assert_eq!(report.models[0].overall.counts.words, WORDS);
    // No gru snapshot: starts untrained, with a warning.
    assert!(
        report
            .meta
            .warnings
            .iter()
            .any(|w| w.contains("gru_action")),
        "{:?}",
        report.meta.warnings
    );
    let json = serde_json::to_string(&report).unwrap();
    let back: Report = serde_json::from_str(&json).unwrap();
    assert_eq!(back, report);
}

#[test]
fn malformed_lines_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    write_fs_log(dir.path());
    let clean = replay_dir(dir.path(), &linear(dir.path())).unwrap();
    let events = dir.path().join("events");
    let first = fs::read_dir(&events)
        .unwrap()
        .map(|e| e.unwrap().path())
        .min()
        .unwrap();
    let mut text = fs::read_to_string(&first).unwrap();
    text.push_str("this is not json\n{\"v\":1}\n");
    fs::write(&first, text).unwrap();

    let report = replay_dir(dir.path(), &linear(dir.path())).unwrap();
    assert_eq!(report.meta.malformed, 2);
    assert_eq!(report.meta.records, clean.meta.records);
    assert_eq!(report.models, clean.models);
}

#[test]
fn empty_dir_reports_no_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let report = replay_dir(dir.path(), &linear(dir.path())).unwrap();
    assert_eq!(report.meta.sessions, 0);
    assert!(report.models.is_empty());
    assert!(render_table(&report).contains("no sessions found"));
}

#[test]
fn missing_dir_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = replay_dir(&dir.path().join("nope"), &linear(dir.path())).unwrap_err();
    assert!(matches!(err, ReplayError::MissingData(_)));
}

#[test]
fn replays_synthetic_log() {
    let cfg = nimk_synth::SynthConfig {
        seed: 2,
        sessions: 2,
        words_per_session: 40,
        ..nimk_synth::SynthConfig::default()
    };
    let scripts = nimk_synth::generate(&cfg, &nimk_synth::Corpus::builtin());
    let dir = tempfile::tempdir().unwrap();
    let fs: Arc<dyn Storage> = Arc::new(PlainFsStorage::open(dir.path()).unwrap());
    pretrain_linear(&fs);
    nimk_synth::write_log(&scripts, fs.as_ref()).unwrap();
    let report = replay_dir(dir.path(), &linear(dir.path())).unwrap();
    assert_eq!(report.meta.sessions, 2);
    let c = &report.models[0].overall.counts;
    let words: u64 = scripts.iter().map(|s| s.words.len() as u64).sum();
    // Raw command text (ADR 0012) has no word record and no script word.
    assert_eq!(c.words, words);
    assert_eq!(c.labelled_words, c.words);
    // Synthetic logs carry no decisions, so there is nothing to compare.
    assert_eq!(report.models[0].overall.edit_accuracy, None);
    // Scripts carry app-flavour bundle ids (terminal, ide, ...).
    assert!(!report.models[0].by_bundle.is_empty());
}

#[test]
fn frozen_replay_scores_without_learning() {
    let dir = tempfile::tempdir().unwrap();
    write_fs_log(dir.path());
    let cfg = ReplayConfig {
        frozen: true,
        ..linear(dir.path())
    };
    let report = replay_dir(dir.path(), &cfg).unwrap();
    assert_eq!(report.meta.warning_count, 0, "{:?}", report.meta.warnings);
    let c = &report.models[0].overall.counts;
    assert_eq!(c.words, 2 * WORDS);
    assert_eq!(c.labelled_words, 2 * WORDS, "labels still score");
    // Both sessions type the same script into the same frozen model.
    assert_eq!(c.correct_words % 2, 0, "{c:?}");
    assert_eq!(c.priors_labelled, 2);
}

#[test]
fn model_switch_records_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let storage: Arc<dyn Storage> = Arc::new(PlainFsStorage::open(dir.path()).unwrap());
    pretrain_linear(&storage);
    let cfg = EngineConfig {
        threading: Threading::Inline,
        ..EngineConfig::default()
    };
    let info = SessionInfo {
        id: "sw".to_owned(),
        wall_unix_ms: 1_794_111_700_000,
        app_version: "test".to_owned(),
    };
    let mut engine = Engine::new(cfg, Arc::clone(&storage), info);
    let mut t = 300.0;
    engine.on_context(&AppContext {
        bundle_id: "com.test.app".to_owned(),
        field_id: 1,
        caret: None,
        timestamp: t,
    });
    for (i, kind) in [ModelKind::GruAction, ModelKind::LinearAction]
        .into_iter()
        .enumerate()
    {
        for c in "hello ".chars() {
            t += 0.1;
            engine.on_key(&key(
                &c.to_string(),
                if c == ' ' { keycode::SPACE } else { 0 },
                t,
            ));
        }
        t += 0.1;
        engine.switch_model(kind, t);
        assert_eq!(engine.model_kind(), kind, "switch {i}");
    }
    engine.shutdown().unwrap();

    let report = replay_dir(dir.path(), &linear(dir.path())).unwrap();
    assert_eq!(report.meta.sessions, 1);
    assert_eq!(report.meta.malformed, 0);
    assert_eq!(report.meta.warning_count, 0, "{:?}", report.meta.warnings);
    assert_eq!(report.models[0].overall.counts.words, 2);
}
