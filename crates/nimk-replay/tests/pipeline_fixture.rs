//! Characterization: the decision stream, the raw log lines and the model
//! statistics of a fixed synthetic session must not change across the
//! Transform/Hook refactor. `NIMK_BLESS=1` rewrites the fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fmt::Write as _;
use std::sync::Arc;

use nimk_engine::{AppContext, Engine, EngineConfig, SessionInfo, Threading};
use nimk_policy::ModelKind;
use nimk_storage::{MemStorage, Storage};
use nimk_synth::{Corpus, SynthConfig, generate, write_log};
use nimk_types::{Boundary, Event, Record};

/// Record types whose raw bytes are pinned (session_start is excluded: it
/// gains `utc_offset_min` in Task 6; model_score is new in Task 5).
const PINNED: &[&str] = &[
    "\"type\":\"context\"",
    "\"type\":\"key\"",
    "\"type\":\"decision\"",
    "\"type\":\"manual_switch\"",
    "\"type\":\"word_commit\"",
    "\"type\":\"label\"",
    "\"type\":\"model_update\"",
];

fn source_records() -> Vec<Record> {
    let cfg = SynthConfig {
        seed: 7,
        sessions: 1,
        words_per_session: 120,
        ..SynthConfig::default()
    };
    let scripts = generate(&cfg, &Corpus::builtin());
    let src = MemStorage::new();
    write_log(&scripts, &src).unwrap();
    let source = src.event_source().unwrap();
    let mut out = Vec::new();
    for id in source.segments().unwrap() {
        for line in source.read_segment(&id).unwrap() {
            out.push(serde_json::from_slice::<Record>(&line.unwrap()).unwrap());
        }
    }
    out
}

fn run(kind: ModelKind) -> String {
    let storage = MemStorage::new();
    let cfg = EngineConfig {
        policy: kind,
        threading: Threading::Inline,
        hotkeys: false,
        ..EngineConfig::default()
    };
    let info = SessionInfo {
        id: "fixture".into(),
        wall_unix_ms: 1_794_111_700_000,
        app_version: "fixture".into(),
        utc_offset_min: 0,
    };
    let mut engine = Engine::new(cfg, Arc::new(storage.clone()) as Arc<dyn Storage>, info);
    for r in source_records() {
        match &r.event {
            Event::Context {
                bundle_id,
                field_id,
                caret,
            } => {
                engine.on_context(&AppContext {
                    bundle_id: bundle_id.clone(),
                    field_id: *field_id,
                    caret: *caret,
                    timestamp: r.t,
                });
            }
            Event::Key { key, .. } => {
                engine.on_key(key);
            }
            Event::Label {
                target,
                label,
                weight,
                source,
            } => engine.inject_label(*target, *label, *weight, *source),
            Event::WordCommit {
                boundary: Boundary::External,
                ..
            } => {
                engine.commit_pending(r.t);
            }
            _ => {}
        }
    }
    let mut out = String::new();
    for k in ModelKind::ALL {
        writeln!(out, "stats {} {:?}", k.name(), engine.model_stats(k)).unwrap();
    }
    engine.shutdown().unwrap();
    let source = storage.event_source().unwrap();
    for id in source.segments().unwrap() {
        for line in source.read_segment(&id).unwrap() {
            let line = String::from_utf8(line.unwrap()).unwrap();
            if PINNED.iter().any(|p| line.contains(p)) {
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out
}

fn check(kind: ModelKind) {
    let path = format!(
        "{}/tests/fixtures/pipeline_{}.txt",
        env!("CARGO_MANIFEST_DIR"),
        kind.name()
    );
    let got = run(kind);
    if std::env::var_os("NIMK_BLESS").is_some() {
        // arch-lint: allow(no-sync-io) reason="test reads and blesses fixture files"
        std::fs::create_dir_all(format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"))).unwrap();
        // arch-lint: allow(no-sync-io) reason="test reads and blesses fixture files"
        std::fs::write(&path, &got).unwrap();
        return;
    }
    // arch-lint: allow(no-sync-io) reason="test reads and blesses fixture files"
    let want = std::fs::read_to_string(&path).expect("fixture missing; run with NIMK_BLESS=1");
    assert!(
        got == want,
        "pipeline output changed for {}; diff {path} against a blessed run",
        kind.name()
    );
}

#[test]
fn linear_pipeline_is_unchanged() {
    check(ModelKind::LinearAction);
}

#[test]
fn gru_pipeline_is_unchanged() {
    check(ModelKind::GruAction);
}
