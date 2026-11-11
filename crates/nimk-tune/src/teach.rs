//! Teacher forcing: scripts are fed to an inline engine through
//! [`Engine::teach`], with the target action of every key.

use std::io;
use std::sync::Arc;

use nimk_engine::{Engine, EngineConfig, EngineError, SessionInfo, Threading};
use nimk_policy::ModelKind;
use nimk_storage::{BlobStore, EventSink, EventSource, Storage};
use nimk_types::AppContext;

use crate::Script;

/// Engine session id while teaching (its events are discarded).
const TEACH_SESSION: &str = "nimk-tune-teach";

/// Knobs of a teaching run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeachOptions {
    /// Passes over the scripts.
    pub epochs: usize,
    /// Teacher-forced examples per learner update
    /// (`EngineConfig::teach_batch`).
    pub batch: usize,
}

impl Default for TeachOptions {
    fn default() -> Self {
        Self {
            epochs: 1,
            batch: 128,
        }
    }
}

/// Storage view that keeps the model snapshot (blobs) but discards events,
/// so teaching never mixes scripted records into a real event log.
pub struct ModelOnly(pub Arc<dyn Storage>);

struct DiscardSink;

impl EventSink for DiscardSink {
    fn append(&mut self, _record: &[u8]) -> io::Result<()> {
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Storage for ModelOnly {
    fn event_sink(&self, _session: &str) -> io::Result<Box<dyn EventSink>> {
        Ok(Box::new(DiscardSink))
    }

    fn event_source(&self) -> io::Result<Box<dyn EventSource + '_>> {
        self.0.event_source()
    }

    fn blobs(&self) -> &dyn BlobStore {
        self.0.blobs()
    }
}

/// Snapshot blob key the engine uses for `kind`.
#[must_use]
pub fn snapshot_key(kind: ModelKind) -> String {
    format!("model/{}/latest", kind.name())
}

fn invalid_data(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn engine_err(e: EngineError) -> io::Error {
    match e {
        EngineError::Storage(io) => io,
    }
}

fn session() -> SessionInfo {
    SessionInfo {
        id: TEACH_SESSION.to_owned(),
        wall_unix_ms: 0,
        app_version: "nimk-tune".to_owned(),
        utc_offset_min: 0,
    }
}

/// Engine context for script number `field_id` in app `bundle_id`.
#[must_use]
pub fn synthetic_context(bundle_id: &str, field_id: u64, timestamp: f64) -> AppContext {
    AppContext {
        bundle_id: bundle_id.to_owned(),
        field_id,
        caret: None,
        timestamp,
    }
}

/// Engine settings for teaching and for evaluating what was taught: no
/// hotkeys, inline trainer, no periodic snapshots (one is saved at the
/// end), `batch` examples per update, no replay.
#[must_use]
pub fn teach_config(cfg: EngineConfig, batch: usize) -> EngineConfig {
    EngineConfig {
        hotkeys: false,
        threading: Threading::Inline,
        snapshot_every_secs: f64::MAX,
        teach_batch: batch.max(1),
        replay_every: 0,
        ..cfg
    }
}

/// Fails with `InvalidData` if the engine reported diagnostics (dropped
/// labels, bad snapshot).
///
/// # Errors
/// `InvalidData` carrying the diagnostics.
pub fn check_diagnostics(engine: &mut Engine, what: &str) -> io::Result<()> {
    let diagnostics = engine.take_diagnostics();
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(invalid_data(format!(
            "{what}: engine diagnostics: {}",
            diagnostics.join("; ")
        )))
    }
}

/// Teacher-forces one script into `engine` as field `field_id`; returns the
/// number of keys.
///
/// # Errors
/// `InvalidData` if keys and targets differ in number or the engine reports
/// diagnostics.
pub fn teach_script(engine: &mut Engine, field_id: u64, script: &Script) -> io::Result<u64> {
    if script.keys.len() != script.targets.len() {
        return Err(invalid_data(format!(
            "{}: {} keys but {} targets",
            script.session,
            script.keys.len(),
            script.targets.len()
        )));
    }
    let t0 = script.keys.first().map_or(0.0, |k| k.timestamp);
    engine.on_context(&synthetic_context(&script.bundle_id, field_id, t0));
    for (key, target) in script.keys.iter().zip(&script.targets) {
        engine.teach(key, *target, 1.0, 1.0);
    }
    check_diagnostics(engine, &script.session)?;
    Ok(script.keys.len() as u64)
}

/// Continues training the snapshot already in `storage` (`model/<kind>/latest`)
/// on `scripts`, for `opts.epochs` passes, and saves the result back there.
/// The learner starts from the snapshot instead of fresh weights, so it can
/// be run on a user's own history ([`crate::scripts_from_log`]) after
/// pre-training. Mix in synthetic scripts to keep the model from forgetting
/// them. Returns the number of keys taught.
///
/// # Errors
/// `InvalidData` if there is no snapshot, the engine reports diagnostics or
/// a script's keys and targets differ in number; I/O errors from saving.
pub fn finetune(
    scripts: &[Script],
    cfg: EngineConfig,
    opts: TeachOptions,
    storage: Arc<dyn Storage>,
) -> io::Result<u64> {
    let cfg = teach_config(cfg, opts.batch);
    let key = snapshot_key(cfg.policy);
    if storage.blobs().get(&key)?.is_none() {
        return Err(invalid_data(format!("snapshot {key} missing")));
    }
    let model_only: Arc<dyn Storage> = Arc::new(ModelOnly(Arc::clone(&storage)));
    let mut engine = Engine::new(cfg, model_only, session());
    check_diagnostics(&mut engine, "finetune start")?;
    let mut keys = 0u64;
    let mut field_id = 0u64;
    for _ in 0..opts.epochs {
        for script in scripts {
            keys += teach_script(&mut engine, field_id, script)?;
            field_id += 1;
        }
    }
    engine.shutdown().map_err(engine_err)?;
    Ok(keys)
}
