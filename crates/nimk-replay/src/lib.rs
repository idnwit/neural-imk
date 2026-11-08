//! Replay harness: re-runs logged sessions through one or more action models and
//! reports metrics (see docs/05-storage-and-log.md).
//!
//! Each session is rebuilt from its `SessionStart` config with `hotkeys =
//! false` and `Threading::Inline`; logged `Context`/`Key` records are fed to
//! the engine in `seq` order and logged `Label` records are injected through
//! `Engine::inject_label` at the same position, so the evaluation is
//! prequential (the mode at commit is decided before the label is learned).
//! Restarts are modelled with one persistent `MemStorage` per model and
//! `Engine::new` per session, so snapshots carry across sessions as in
//! production.
//!
//! Modules:
//! - [`cli`]: argument parsing for the `nimk-replay` binary.
//! - `load`: reads segments and groups records into sessions.
//! - `drive`: replays one session through an engine.
//! - `metrics`: aligns replayed decisions with logged labels.
//! - [`report`]: report types and the stdout table.
#![forbid(unsafe_code)]

pub mod cli;
mod drive;
mod load;
mod metrics;
pub mod report;

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use nimk_policy::ModelKind;
use nimk_storage::{MemStorage, PlainFsStorage, Storage};

pub use metrics::Counts;
pub use report::{Metrics, ModelReport, Report, RunMeta, model_name, render_table, snapshot_key};

/// Maximum number of warning messages kept in a [`Report`] (all are counted).
const MAX_WARNINGS: usize = 50;

/// Replay errors.
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    /// The data directory does not exist.
    #[error("data directory {0} does not exist")]
    MissingData(PathBuf),
    /// Reading the log failed.
    #[error("storage: {0}")]
    Io(#[from] io::Error),
    /// Serializing the report failed.
    #[error("report: {0}")]
    Json(#[from] serde_json::Error),
}

/// What to replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayConfig {
    /// Models to evaluate, in output order.
    pub models: Vec<ModelKind>,
    /// Only segments whose id sorts at or after this prefix.
    pub from: Option<String>,
    /// Overrides `EngineConfig::seed` of every session when set.
    pub seed: Option<u64>,
    /// Data directory whose `model/<kind>/latest` snapshot each model starts
    /// from (e.g. the output of `nimk-synth pretrain`). Without it, or when
    /// the snapshot is missing, a model starts untrained, which never
    /// commits words (a warning is reported).
    pub init: Option<PathBuf>,
    /// Do not feed logged labels to the engines, so models never learn
    /// online (a baseline for how much online learning helps or hurts).
    /// Metrics still use the logged labels as ground truth.
    pub frozen: bool,
}

/// Opens `data` as a [`PlainFsStorage`] root and replays it.
///
/// # Errors
/// Fails if `data` does not exist or the log cannot be read.
pub fn replay_dir(data: &Path, cfg: &ReplayConfig) -> Result<Report, ReplayError> {
    // arch-lint: allow(no-sync-io) reason="synchronous CLI, no async runtime"
    if !data.is_dir() {
        return Err(ReplayError::MissingData(data.to_path_buf()));
    }
    let storage = PlainFsStorage::open(data)?;
    replay(&storage, cfg)
}

/// Replays every session in `storage` once per model in `cfg`.
///
/// Malformed log lines are skipped and counted. With no sessions the report
/// has no policy rows.
///
/// # Errors
/// Fails if the log cannot be read.
pub fn replay(storage: &dyn Storage, cfg: &ReplayConfig) -> Result<Report, ReplayError> {
    let source = storage.event_source()?;
    let log = load::load(source.as_ref(), cfg.from.as_deref())?;
    let init = cfg.init.as_deref().map(PlainFsStorage::open).transpose()?;
    let mut warnings = log.warnings.clone();
    let mut warning_count = warnings.len();
    let mut models = Vec::with_capacity(cfg.models.len());
    if !log.sessions.is_empty() {
        for &kind in &cfg.models {
            let (rep, w) = replay_model(&log.sessions, kind, cfg, init.as_ref());
            warning_count += w.len();
            warnings.extend(w);
            models.push(rep);
        }
    }
    warnings.truncate(MAX_WARNINGS);
    Ok(Report {
        meta: RunMeta {
            models: cfg
                .models
                .iter()
                .map(|&k| model_name(k).to_owned())
                .collect(),
            from: cfg.from.clone(),
            seed: cfg.seed,
            segments: log.segments,
            sessions: log.sessions.len(),
            records: log.records,
            malformed: log.malformed,
            warning_count,
            warnings,
        },
        models,
    })
}

/// Replays all sessions through one model with a persistent blob store.
fn replay_model(
    sessions: &[load::Session],
    kind: ModelKind,
    cfg: &ReplayConfig,
    init: Option<&PlainFsStorage>,
) -> (ModelReport, Vec<String>) {
    let blobs = MemStorage::new();
    let mut warnings = Vec::new();
    if let Some(init) = init {
        let key = snapshot_key(kind);
        match init.blobs().get(&key) {
            Ok(Some(bytes)) => {
                if let Err(e) = blobs.blobs().put(&key, &bytes) {
                    warnings.push(format!("[{}] cannot seed {key}: {e}", model_name(kind)));
                }
            }
            Ok(None) => warnings.push(format!(
                "[{}] no {key} in the init directory; starting untrained",
                model_name(kind)
            )),
            Err(e) => warnings.push(format!("[{}] cannot read {key}: {e}", model_name(kind))),
        }
    }
    let mut overall = Counts::default();
    let mut by_bundle: BTreeMap<String, Counts> = BTreeMap::new();
    let mut final_model_id = None;
    for session in sessions {
        let out = drive::replay_session(session, kind, &blobs, cfg.seed, cfg.frozen);
        let name = model_name(kind);
        warnings.extend(out.warnings.into_iter().map(|w| format!("[{name}] {w}")));
        metrics::accumulate(&session.records, &out.records, &mut overall, &mut by_bundle);
        if out.policy_id.is_some() {
            final_model_id = out.policy_id;
        }
    }
    let report = ModelReport {
        model: model_name(kind).to_owned(),
        final_model_id,
        overall: Metrics::from_counts(&overall),
        by_bundle: by_bundle
            .iter()
            .map(|(b, c)| (b.clone(), Metrics::from_counts(c)))
            .collect(),
    };
    (report, warnings)
}
