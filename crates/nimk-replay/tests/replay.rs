//! End-to-end tests: write a log with a real engine, then replay it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;
use std::sync::Arc;

use nimk_engine::{AppContext, Engine, EngineConfig, KeyEvent, SessionInfo, Threading};
use nimk_policy::PolicyKind;
use nimk_replay::{ReplayConfig, ReplayError, Report, render_table, replay, replay_dir};
use nimk_storage::{MemStorage, PlainFsStorage, Storage};
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
        policy: PolicyKind::Baseline,
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

fn baseline() -> ReplayConfig {
    ReplayConfig {
        policies: vec![PolicyKind::Baseline],
        from: None,
        seed: None,
    }
}

fn write_fs_log(root: &Path) {
    let storage: Arc<dyn Storage> = Arc::new(PlainFsStorage::open(root).unwrap());
    write_session(Arc::clone(&storage), "s1", SCRIPT, 100.0);
    write_session(storage, "s2", SCRIPT, 200.0);
}

#[test]
fn replays_engine_log_deterministically() {
    let dir = tempfile::tempdir().unwrap();
    write_fs_log(dir.path());

    let report = replay_dir(dir.path(), &baseline()).unwrap();
    assert_eq!(report.meta.sessions, 2);
    assert_eq!(report.meta.malformed, 0);
    assert_eq!(report.meta.warning_count, 0, "{:?}", report.meta.warnings);
    assert_eq!(report.policies.len(), 1);
    let m = &report.policies[0].overall;
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
        report.policies[0].by_bundle.keys().collect::<Vec<_>>(),
        ["com.test.app"]
    );

    let again = replay_dir(dir.path(), &baseline()).unwrap();
    assert_eq!(report, again);
    let table = render_table(&report);
    assert!(table.contains("baseline"), "{table}");
    assert!(table.contains("com.test.app"), "{table}");
}

#[test]
fn all_policies_and_json_roundtrip() {
    let storage = MemStorage::new();
    let shared: Arc<dyn Storage> = Arc::new(storage.clone());
    write_session(shared, "s1", SCRIPT, 100.0);
    let cfg = ReplayConfig {
        policies: vec![
            PolicyKind::Baseline,
            PolicyKind::Linear,
            PolicyKind::TinyMlp,
        ],
        from: None,
        seed: Some(3),
    };
    let report = replay(&storage, &cfg).unwrap();
    assert_eq!(report.policies.len(), 3);
    for p in &report.policies {
        assert_eq!(p.overall.counts.words, WORDS, "{}", p.policy);
    }
    let json = serde_json::to_string(&report).unwrap();
    let back: Report = serde_json::from_str(&json).unwrap();
    assert_eq!(back, report);
}

#[test]
fn malformed_lines_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    write_fs_log(dir.path());
    let clean = replay_dir(dir.path(), &baseline()).unwrap();
    let events = dir.path().join("events");
    let first = fs::read_dir(&events)
        .unwrap()
        .map(|e| e.unwrap().path())
        .min()
        .unwrap();
    let mut text = fs::read_to_string(&first).unwrap();
    text.push_str("this is not json\n{\"v\":1}\n");
    fs::write(&first, text).unwrap();

    let report = replay_dir(dir.path(), &baseline()).unwrap();
    assert_eq!(report.meta.malformed, 2);
    assert_eq!(report.meta.records, clean.meta.records);
    assert_eq!(report.policies, clean.policies);
}

#[test]
fn empty_dir_reports_no_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let report = replay_dir(dir.path(), &baseline()).unwrap();
    assert_eq!(report.meta.sessions, 0);
    assert!(report.policies.is_empty());
    assert!(render_table(&report).contains("no sessions found"));
}

#[test]
fn missing_dir_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = replay_dir(&dir.path().join("nope"), &baseline()).unwrap_err();
    assert!(matches!(err, ReplayError::MissingData(_)));
}

#[test]
fn replays_synthetic_log() {
    let cfg = nimk_synth::SynthConfig {
        seed: 1,
        sessions: 2,
        words_per_session: 40,
        ..nimk_synth::SynthConfig::default()
    };
    let scripts = nimk_synth::generate(&cfg, &nimk_synth::Corpus::builtin());
    let storage = MemStorage::new();
    nimk_synth::write_log(&scripts, &storage).unwrap();
    let report = replay(&storage, &baseline()).unwrap();
    assert_eq!(report.meta.sessions, 2);
    let c = &report.policies[0].overall.counts;
    let words: u64 = scripts.iter().map(|s| s.words.len() as u64).sum();
    assert_eq!(c.words, words);
    assert_eq!(c.labelled_words, words);
    assert!(
        report.policies[0]
            .by_bundle
            .contains_key(nimk_synth::SYNTHETIC_BUNDLE_ID)
    );
}
