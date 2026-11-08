//! Replay harness: re-runs logged sessions through one or more policies and
//! reports metrics (see docs/05-storage-and-log.md).
//!
//! Each session is rebuilt from its `SessionStart` config with `hotkeys =
//! false` and `Threading::Inline`; logged `Context`/`Key` records are fed to
//! the engine in `seq` order and logged `Label` records are injected through
//! `Engine::inject_label` at the same position, so the evaluation is
//! prequential (the mode at commit is decided before the label is learned).
//! Restarts are modelled with one persistent `MemStorage` per policy and
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

use nimk_policy::PolicyKind;
use nimk_storage::{MemStorage, PlainFsStorage, Storage};

pub use metrics::Counts;
pub use report::{Metrics, PolicyReport, Report, RunMeta, policy_name, render_table};

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
    /// Policies to evaluate, in output order.
    pub policies: Vec<PolicyKind>,
    /// Only segments whose id sorts at or after this prefix.
    pub from: Option<String>,
    /// Overrides `EngineConfig::seed` of every session when set.
    pub seed: Option<u64>,
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

/// Replays every session in `storage` once per policy in `cfg`.
///
/// Malformed log lines are skipped and counted. With no sessions the report
/// has no policy rows.
///
/// # Errors
/// Fails if the log cannot be read.
pub fn replay(storage: &dyn Storage, cfg: &ReplayConfig) -> Result<Report, ReplayError> {
    let source = storage.event_source()?;
    let log = load::load(source.as_ref(), cfg.from.as_deref())?;
    let mut warnings = log.warnings.clone();
    let mut warning_count = warnings.len();
    let mut policies = Vec::with_capacity(cfg.policies.len());
    if !log.sessions.is_empty() {
        for &kind in &cfg.policies {
            let (rep, w) = replay_policy(&log.sessions, kind, cfg.seed);
            warning_count += w.len();
            warnings.extend(w);
            policies.push(rep);
        }
    }
    warnings.truncate(MAX_WARNINGS);
    Ok(Report {
        meta: RunMeta {
            policies: cfg
                .policies
                .iter()
                .map(|&k| policy_name(k).to_owned())
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
        policies,
    })
}

/// Replays all sessions through one policy with a persistent blob store.
fn replay_policy(
    sessions: &[load::Session],
    kind: PolicyKind,
    seed: Option<u64>,
) -> (PolicyReport, Vec<String>) {
    let blobs = MemStorage::new();
    let mut overall = Counts::default();
    let mut by_bundle: BTreeMap<String, Counts> = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut final_policy_id = None;
    for session in sessions {
        let out = drive::replay_session(session, kind, &blobs, seed);
        let name = policy_name(kind);
        warnings.extend(out.warnings.into_iter().map(|w| format!("[{name}] {w}")));
        metrics::accumulate(&session.records, &out.records, &mut overall, &mut by_bundle);
        if out.policy_id.is_some() {
            final_policy_id = out.policy_id;
        }
    }
    let report = PolicyReport {
        policy: policy_name(kind).to_owned(),
        final_policy_id,
        overall: Metrics::from_counts(&overall),
        by_bundle: by_bundle
            .iter()
            .map(|(b, c)| (b.clone(), Metrics::from_counts(c)))
            .collect(),
    };
    (report, warnings)
}
