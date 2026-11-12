//! Counting the data typed since the last fine-tuning run, incrementally.
//!
//! The log only grows by appending to its newest segment, and a segment that
//! is no longer written never changes. So the counts are kept per segment,
//! with the byte offset they cover, in the blob [`INDEX_KEY`]: a count reads
//! only what was appended since, which is a few records instead of the whole
//! log (parsing it all took seconds at 100 MB, every time the menu opened).
//!
//! The counts depend on the watermark (which records are "new"), so the index
//! remembers the watermark it was built for; after a run moves it, the next
//! count starts over once. The index is only a cache: a missing, unreadable
//! or stale one is rebuilt, and failing to save it does not fail the count.

use std::collections::BTreeMap;
use std::io;

use nimk_storage::Storage;
use nimk_types::SCHEMA_VERSION;
use serde::{Deserialize, Serialize};

use crate::tune::{TuneState, read_state};

/// Blob holding the per-segment counts.
const INDEX_KEY: &str = "tune/pending-index";

/// Version written to the index blob.
const INDEX_VERSION: u32 = 1;

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

/// What counting needs from one segment, and how far it has read.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct SegmentCount {
    /// Bytes read: the end of the last complete record counted.
    offset: u64,
    new_words: u64,
    new_corrections: u64,
    total_words: u64,
}

/// The cache blob.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    version: u32,
    /// The watermark the counts were made for.
    watermark: BTreeMap<String, u64>,
    segments: BTreeMap<String, SegmentCount>,
}

impl Index {
    /// The saved index when it is usable for `state`, else an empty one.
    fn load(storage: &dyn Storage, state: &TuneState) -> Self {
        if let Ok(Some(bytes)) = storage.blobs().get(INDEX_KEY)
            && let Ok(saved) = serde_json::from_slice::<Self>(&bytes)
            && saved.version == INDEX_VERSION
            && saved.watermark == state.sessions
        {
            return saved;
        }
        Self {
            version: INDEX_VERSION,
            watermark: state.sessions.clone(),
            segments: BTreeMap::new(),
        }
    }
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
/// Reads only what the log gained since the last call (see the module docs);
/// the first call, and the first after a run, read all of it.
///
/// # Errors
/// An I/O error from reading the log or the state blob.
pub fn pending(storage: &dyn Storage) -> io::Result<TunePending> {
    let state = read_state(storage)?;
    let source = storage.event_source()?;
    let mut index = Index::load(storage, &state);
    let mut changed = false;

    let ids = source.segments()?;
    // A segment that is gone leaves the index.
    let before = index.segments.len();
    index
        .segments
        .retain(|id, _| ids.iter().any(|s| &s.0 == id));
    changed |= index.segments.len() != before;

    for id in &ids {
        let mut count = match index.segments.get(&id.0) {
            Some(saved) => *saved,
            None => SegmentCount::default(),
        };
        let mut read = 0u64;
        for line in source.read_segment_from(id, count.offset)? {
            let line = line?;
            read += line.len() as u64 + 1;
            let Ok(h) = serde_json::from_slice::<Head>(&line) else {
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
                count.total_words += 1;
                if new {
                    count.new_words += 1;
                    count.new_corrections += u64::from(h.source.as_deref() == Some("correction"));
                }
            }
        }
        if read > 0 || !index.segments.contains_key(&id.0) {
            count.offset += read;
            index.segments.insert(id.0.clone(), count);
            changed = true;
        }
    }

    let mut out = TunePending {
        last_unix_ms: (state.last_unix_ms > 0).then_some(state.last_unix_ms),
        runs: state.runs,
        ..TunePending::default()
    };
    for c in index.segments.values() {
        out.total_words += c.total_words;
        out.new_words += c.new_words;
        out.new_corrections += c.new_corrections;
    }
    if changed && save(storage, &index).is_err() {
        // Only a cache: the next count redoes the work, the numbers are right.
    }
    Ok(out)
}

/// Saves the index.
fn save(storage: &dyn Storage, index: &Index) -> io::Result<()> {
    let json = serde_json::to_vec(index).map_err(io::Error::other)?;
    storage.blobs().put(INDEX_KEY, &json)
}

#[cfg(test)]
mod tests;
