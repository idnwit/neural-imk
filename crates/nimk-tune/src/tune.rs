//! Periodic fine-tuning of the active model on the user's own log.
//!
//! The log is append-only, so "new data" is everything after a watermark:
//! per session, the next `seq` not yet used for tuning. [`pending`] counts
//! the words and corrections after it; [`tune`] teaches the new scripts (plus
//! a slice of older ones, so the model does not drift) to a copy of the
//! current snapshot and moves the watermark.
//!
//! The run never touches the live engine: it reads the saved snapshot and
//! the log, and writes its result to a blob. The engine installs that blob
//! when the host asks (`Engine::install_tuned`). Anything the engine learned
//! online between the snapshot being read and the install is replaced.

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use nimk_engine::EngineConfig;
use nimk_policy::ModelKind;
use nimk_storage::{MemStorage, Storage};
use nimk_types::SCHEMA_VERSION;
use serde::{Deserialize, Serialize};

use crate::teach::{TeachOptions, finetune};
use crate::{Script, scripts_from_log};

/// Blob holding the watermark.
pub const STATE_KEY: &str = "tune/state";

/// Blob a finished run leaves for the engine to install.
#[must_use]
pub fn result_key(kind: ModelKind) -> String {
    nimk_engine::tuned_key(kind)
}

fn snapshot_key(kind: ModelKind) -> String {
    format!("model/{}/latest", kind.name())
}

/// Version written to the state blob.
const STATE_VERSION: u32 = 1;

/// What the last runs consumed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TuneState {
    #[serde(default)]
    version: u32,
    /// Per session, the next `seq` that is still new.
    #[serde(default)]
    sessions: BTreeMap<String, u64>,
    /// Wall clock of the last run, ms since the epoch (0 = never).
    #[serde(default)]
    last_unix_ms: u64,
    /// Runs so far.
    #[serde(default)]
    runs: u32,
}

fn read_state(storage: &dyn Storage) -> io::Result<TuneState> {
    Ok(match storage.blobs().get(STATE_KEY)? {
        // An unreadable state reads as "never tuned": everything is new.
        Some(bytes) => match serde_json::from_slice(&bytes) {
            Ok(state) => state,
            Err(_) => TuneState {
                version: STATE_VERSION,
                ..TuneState::default()
            },
        },
        None => TuneState::default(),
    })
}

/// How much data is waiting for the next run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TunePending {
    /// Words committed since the last run.
    pub new_words: u64,
    /// Hotkey corrections since the last run.
    pub new_corrections: u64,
    /// Words in the whole log.
    pub total_words: u64,
    /// Wall clock of the last run, ms since the epoch; `None` = never run.
    pub last_unix_ms: Option<u64>,
    /// Runs so far.
    pub runs: u32,
}

/// The few fields of a log line that counting needs.
#[derive(Deserialize)]
struct Head {
    v: u32,
    session: String,
    seq: u64,
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    target: Option<TargetHead>,
}

/// `target` of a label line.
#[derive(Deserialize)]
struct TargetHead {
    kind: String,
}

/// Counts the words and corrections after the watermark.
///
/// # Errors
/// An I/O error from reading the log or the state blob.
pub fn pending(storage: &dyn Storage) -> io::Result<TunePending> {
    let state = read_state(storage)?;
    let source = storage.event_source()?;
    let mut out = TunePending {
        last_unix_ms: (state.last_unix_ms > 0).then_some(state.last_unix_ms),
        runs: state.runs,
        ..TunePending::default()
    };
    for seg in source.segments()? {
        for line in source.read_segment(&seg)? {
            let Ok(h) = serde_json::from_slice::<Head>(&line?) else {
                continue;
            };
            if h.v != SCHEMA_VERSION {
                continue;
            }
            let new = h.seq >= state.sessions.get(&h.session).map_or(0, |&n| n);
            // A word counts once it has its final label (written some words
            // after the commit): every log has those, synthetic ones too.
            let word_label = h.ty == "label" && h.target.is_some_and(|t| t.kind == "word");
            if word_label {
                out.total_words += 1;
                if new {
                    out.new_words += 1;
                    out.new_corrections += u64::from(h.source.as_deref() == Some("correction"));
                }
            }
        }
    }
    Ok(out)
}

/// Knobs of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuneOptions {
    /// Refuse to run with fewer new words than this.
    pub min_new_words: u64,
    /// Older words taught again with the new ones.
    pub replay_words: usize,
    /// Passes over the training scripts. The experiments saw no gain past 1.
    pub epochs: usize,
    /// Teacher-forced examples per learner update.
    pub batch: usize,
}

impl Default for TuneOptions {
    fn default() -> Self {
        Self {
            min_new_words: 200,
            replay_words: 3000,
            epochs: 1,
            batch: 128,
        }
    }
}

/// Where a finished run leaves the tuned snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuneOutput {
    /// `tune/<kind>/result`, for the running engine to install.
    Result,
    /// `model/<kind>/latest` directly. Only when no engine is running: it
    /// would overwrite this at its next save.
    Latest,
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuneReport {
    /// New words taught.
    pub new_words: u64,
    /// Older words taught again.
    pub replay_words: u64,
    /// Keys taught (all passes).
    pub keys: u64,
}

/// Fine-tunes `kind` on the log after the watermark and moves the
/// watermark. `now_unix_ms` is recorded as the time of the run.
///
/// # Errors
/// `InvalidInput` if fewer than `opts.min_new_words` words are new;
/// `InvalidData` if there is no snapshot of `kind`; I/O errors from the log
/// or the blobs. On any error the watermark does not move.
pub fn tune(
    storage: &Arc<dyn Storage>,
    kind: ModelKind,
    opts: TuneOptions,
    now_unix_ms: u64,
    output: TuneOutput,
) -> io::Result<TuneReport> {
    let state = read_state(storage.as_ref())?;
    let source = storage.event_source()?;
    let log = scripts_from_log(source.as_ref())?;
    let is_new = |i: usize| {
        let m = &log.meta[i];
        m.last_seq >= state.sessions.get(&m.session).map_or(0, |&n| n)
    };
    let words = |i: usize| log.scripts[i].words.len() as u64;
    let new: Vec<usize> = (0..log.scripts.len()).filter(|&i| is_new(i)).collect();
    let new_words: u64 = new.iter().map(|&i| words(i)).sum();
    if new_words < opts.min_new_words {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{new_words} new words, at least {} needed",
                opts.min_new_words
            ),
        ));
    }
    // Replay: the newest old scripts up to the word budget, oldest first.
    let mut replay: Vec<usize> = Vec::new();
    let mut replay_words = 0u64;
    for i in (0..log.scripts.len()).rev().filter(|&i| !is_new(i)) {
        if replay_words >= opts.replay_words as u64 {
            break;
        }
        replay_words += words(i);
        replay.push(i);
    }
    replay.reverse();
    let train: Vec<Script> = replay
        .iter()
        .chain(&new)
        .map(|&i| log.scripts[i].clone())
        .collect();

    let key = snapshot_key(kind);
    let bytes = storage
        .blobs()
        .get(&key)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("no {key}")))?;
    let scratch = MemStorage::new();
    scratch.blobs().put(&key, &bytes)?;
    let scratch: Arc<dyn Storage> = Arc::new(scratch);
    let cfg = EngineConfig {
        policy: kind,
        ..EngineConfig::default()
    };
    let popts = TeachOptions {
        epochs: opts.epochs.max(1),
        batch: opts.batch.max(1),
    };
    let keys = finetune(&train, cfg, popts, Arc::clone(&scratch))?;
    let tuned = scratch.blobs().get(&key)?.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "fine-tuning left no snapshot")
    })?;

    let target = match output {
        TuneOutput::Result => result_key(kind),
        TuneOutput::Latest => key,
    };
    storage.blobs().put(&target, &tuned)?;
    let next = TuneState {
        version: STATE_VERSION,
        sessions: log.next_seq,
        last_unix_ms: now_unix_ms,
        runs: state.runs + 1,
    };
    let json = serde_json::to_vec(&next).map_err(io::Error::other)?;
    storage.blobs().put(STATE_KEY, &json)?;
    Ok(TuneReport {
        new_words,
        replay_words,
        keys,
    })
}
