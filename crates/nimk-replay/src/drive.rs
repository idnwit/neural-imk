//! Replays one logged session through a fresh engine.

use std::io;
use std::sync::Arc;

use nimk_engine::{Engine, EngineConfig, SessionInfo, Threading};
use nimk_policy::ModelKind;
use nimk_storage::{BlobStore, EventSink, EventSource, MemStorage, Storage};
use nimk_types::{AppContext, Boundary, Event, Record};

use crate::load::Session;

/// Storage given to a replayed engine: blobs (model snapshots) persist
/// across sessions of one policy, events go to a per-session store that the
/// harness reads back afterwards.
struct ReplayStorage {
    blobs: MemStorage,
    events: MemStorage,
}

impl Storage for ReplayStorage {
    fn event_sink(&self, session: &str) -> io::Result<Box<dyn EventSink>> {
        self.events.event_sink(session)
    }

    fn event_source(&self) -> io::Result<Box<dyn EventSource + '_>> {
        self.events.event_source()
    }

    fn blobs(&self) -> &dyn BlobStore {
        self.blobs.blobs()
    }
}

/// Result of replaying one session.
#[derive(Debug, Default)]
pub(crate) struct SessionOut {
    /// Records the replayed engine logged, in `seq` order.
    pub(crate) records: Vec<Record>,
    /// Policy id of the replayed engine at the end of the session.
    pub(crate) policy_id: Option<String>,
    /// Config fallbacks, engine diagnostics and storage errors.
    pub(crate) warnings: Vec<String>,
}

/// Builds the engine config for a replayed session from its `SessionStart`.
fn session_config(
    session: &Session,
    kind: ModelKind,
    seed: Option<u64>,
    warnings: &mut Vec<String>,
) -> (EngineConfig, SessionInfo) {
    let start = session.records.iter().find_map(|r| match &r.event {
        Event::SessionStart {
            wall_unix_ms,
            app_version,
            config_json,
            ..
        } => Some((*wall_unix_ms, app_version.clone(), config_json.as_str())),
        _ => None,
    });
    let (wall_unix_ms, app_version, mut cfg) = match start {
        Some((wall, version, json)) => match serde_json::from_str::<EngineConfig>(json) {
            Ok(cfg) => (wall, version, cfg),
            Err(e) => {
                warnings.push(format!(
                    "session {}: bad config_json ({e}); using defaults",
                    session.id
                ));
                (wall, version, EngineConfig::default())
            }
        },
        None => {
            warnings.push(format!(
                "session {}: no session_start; using default config",
                session.id
            ));
            (0, String::from("unknown"), EngineConfig::default())
        }
    };
    cfg.policy = kind;
    cfg.hotkeys = false;
    cfg.threading = Threading::Inline;
    if let Some(seed) = seed {
        cfg.seed = seed;
    }
    let info = SessionInfo {
        id: session.id.clone(),
        wall_unix_ms,
        app_version,
    };
    (cfg, info)
}

/// Feeds one logged record to the engine; labels are skipped when `frozen`.
fn feed(engine: &mut Engine, record: &Record, frozen: bool) {
    match &record.event {
        Event::Context {
            bundle_id,
            field_id,
            caret,
        } => {
            engine.on_context(&AppContext {
                bundle_id: bundle_id.clone(),
                field_id: *field_id,
                caret: *caret,
                timestamp: record.t,
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
        } if !frozen => engine.inject_label(*target, *label, *weight, *source),
        // An external commit without a preceding key (IMK
        // `commitComposition`); a no-op when the replayed engine already
        // committed the word (e.g. inside `on_context`).
        Event::WordCommit {
            boundary: Boundary::External,
            ..
        } => {
            engine.commit_pending(record.t);
        }
        _ => {}
    }
}

/// Reads back and parses every record the replayed engine logged.
fn read_back(events: &MemStorage, warnings: &mut Vec<String>) -> io::Result<Vec<Record>> {
    let source = events.event_source()?;
    let mut records = Vec::new();
    for id in source.segments()? {
        for line in source.read_segment(&id)? {
            match serde_json::from_slice::<Record>(&line?) {
                Ok(r) => records.push(r),
                Err(e) => warnings.push(format!("unreadable replayed record: {e}")),
            }
        }
    }
    records.sort_by_key(|r| r.seq);
    Ok(records)
}

/// Replays `session` through model `kind`, with snapshots kept in `blobs`;
/// `frozen` withholds the logged labels (no online learning).
pub(crate) fn replay_session(
    session: &Session,
    kind: ModelKind,
    blobs: &MemStorage,
    seed: Option<u64>,
    frozen: bool,
) -> SessionOut {
    let mut out = SessionOut::default();
    let (cfg, info) = session_config(session, kind, seed, &mut out.warnings);
    let events = MemStorage::new();
    let storage = Arc::new(ReplayStorage {
        blobs: blobs.clone(),
        events: events.clone(),
    });
    let mut engine = Engine::new(cfg, storage, info);
    for record in &session.records {
        feed(&mut engine, record, frozen);
    }
    out.policy_id = Some(engine.policy_id().to_string());
    let id = &session.id;
    out.warnings.extend(
        engine
            .take_diagnostics()
            .into_iter()
            .map(|d| format!("session {id}: {d}")),
    );
    if let Err(e) = engine.shutdown() {
        out.warnings
            .push(format!("session {id}: shutdown failed ({e})"));
    }
    match read_back(&events, &mut out.warnings) {
        Ok(records) => out.records = records,
        Err(e) => out
            .warnings
            .push(format!("session {id}: cannot read replayed events ({e})")),
    }
    out
}
