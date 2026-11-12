#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use nimk_storage::{BlobStore, EventSink, EventSource, MemStorage, RecordIter, SegmentId, Storage};

use super::*;
use crate::tune::{STATE_KEY, STATE_VERSION};

/// A word label line; `seq` is per session.
fn word(session: &str, seq: u64, correction: bool) -> Vec<u8> {
    let source = if correction { "correction" } else { "implicit" };
    format!(
        r#"{{"v":{SCHEMA_VERSION},"seq":{seq},"t":0.0,"session":"{session}","type":"label","target":{{"kind":"word","id":{seq}}},"label":"vi","weight":0.3,"source":"{source}"}}"#
    )
    .into_bytes()
}

/// A line that is not a word label.
fn clock(session: &str, seq: u64) -> Vec<u8> {
    format!(
        r#"{{"v":{SCHEMA_VERSION},"seq":{seq},"t":0.0,"session":"{session}","type":"clock","wall_unix_ms":1,"utc_offset_min":0}}"#
    )
    .into_bytes()
}

/// Storage that counts the records its event source hands out.
struct Counting {
    inner: MemStorage,
    records_read: Arc<AtomicU64>,
}

impl Counting {
    fn new() -> Self {
        Self {
            inner: MemStorage::new(),
            records_read: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Records read since the last call.
    fn take_reads(&self) -> u64 {
        self.records_read.swap(0, Ordering::Relaxed)
    }
}

impl Storage for Counting {
    fn event_sink(&self, session: &str) -> std::io::Result<Box<dyn EventSink>> {
        self.inner.event_sink(session)
    }

    fn event_source(&self) -> std::io::Result<Box<dyn EventSource + '_>> {
        Ok(Box::new(CountingSource {
            inner: self.inner.event_source()?,
            reads: Arc::clone(&self.records_read),
        }))
    }

    fn blobs(&self) -> &dyn BlobStore {
        self.inner.blobs()
    }
}

struct CountingSource<'a> {
    inner: Box<dyn EventSource + 'a>,
    reads: Arc<AtomicU64>,
}

impl CountingSource<'_> {
    fn count<'a>(&self, records: RecordIter<'a>) -> RecordIter<'a> {
        let reads = Arc::clone(&self.reads);
        Box::new(records.inspect(move |_| {
            reads.fetch_add(1, Ordering::Relaxed);
        }))
    }
}

impl EventSource for CountingSource<'_> {
    fn segments(&self) -> std::io::Result<Vec<SegmentId>> {
        self.inner.segments()
    }

    fn read_segment(&self, id: &SegmentId) -> std::io::Result<RecordIter<'_>> {
        Ok(self.count(self.inner.read_segment(id)?))
    }

    fn read_segment_from(&self, id: &SegmentId, offset: u64) -> std::io::Result<RecordIter<'_>> {
        Ok(self.count(self.inner.read_segment_from(id, offset)?))
    }
}

fn append(s: &dyn Storage, session: &str, records: &[Vec<u8>]) {
    let mut sink = s.event_sink(session).unwrap();
    for r in records {
        sink.append(r).unwrap();
    }
    sink.flush().unwrap();
}

fn write_state(s: &dyn Storage, sessions: &[(&str, u64)], runs: u32) {
    let state = TuneState {
        version: STATE_VERSION,
        sessions: sessions.iter().map(|&(k, v)| (k.to_owned(), v)).collect(),
        last_unix_ms: 5,
        runs,
    };
    s.blobs()
        .put(STATE_KEY, &serde_json::to_vec(&state).unwrap())
        .unwrap();
}

#[test]
fn counts_words_and_corrections_after_the_watermark() {
    let s = MemStorage::new();
    append(
        &s,
        "a",
        &[
            word("a", 0, false),
            clock("a", 1),
            word("a", 2, true),
            word("a", 3, false),
            b"not json".to_vec(),
        ],
    );
    let p = pending(&s).unwrap();
    assert_eq!((p.total_words, p.new_words, p.new_corrections), (3, 3, 1));

    // Seqs before the watermark are old.
    write_state(&s, &[("a", 3)], 1);
    let p = pending(&s).unwrap();
    assert_eq!((p.total_words, p.new_words, p.new_corrections), (3, 1, 0));
    assert_eq!((p.runs, p.last_unix_ms), (1, Some(5)));
}

#[test]
fn a_second_count_reads_nothing_and_a_third_only_what_was_appended() {
    let s = Counting::new();
    let first: Vec<Vec<u8>> = (0..50).map(|i| word("a", i, false)).collect();
    append(&s, "a", &first);
    assert_eq!(pending(&s).unwrap().new_words, 50);
    assert_eq!(s.take_reads(), 50);

    assert_eq!(pending(&s).unwrap().new_words, 50);
    assert_eq!(s.take_reads(), 0, "nothing was appended");

    append(&s, "a", &[word("a", 50, true), clock("a", 51)]);
    let p = pending(&s).unwrap();
    assert_eq!((p.new_words, p.new_corrections), (51, 1));
    assert_eq!(s.take_reads(), 2, "only the two appended records");
}

#[test]
fn the_incremental_count_equals_a_fresh_one() {
    let s = Counting::new();
    let mut seq = 0;
    for round in 0..4 {
        let batch: Vec<Vec<u8>> = (0..7)
            .map(|i| {
                seq += 1;
                if i % 3 == 0 {
                    clock("a", seq)
                } else {
                    word("a", seq, i % 2 == 0)
                }
            })
            .collect();
        append(&s, if round % 2 == 0 { "a" } else { "b" }, &batch);
        let incremental = pending(&s).unwrap();
        s.blobs().delete(INDEX_KEY).unwrap();
        assert_eq!(incremental, pending(&s).unwrap(), "round {round}");
    }
}

#[test]
fn a_moved_watermark_rebuilds_the_counts() {
    let s = Counting::new();
    let recs: Vec<Vec<u8>> = (0..10).map(|i| word("a", i, false)).collect();
    append(&s, "a", &recs);
    assert_eq!(pending(&s).unwrap().new_words, 10);
    s.take_reads();

    // A run consumed everything up to seq 10.
    write_state(&s, &[("a", 10)], 1);
    let p = pending(&s).unwrap();
    assert_eq!((p.new_words, p.total_words), (0, 10));
    assert_eq!(s.take_reads(), 10, "the counts depend on the watermark");

    append(&s, "a", &[word("a", 10, false)]);
    assert_eq!(pending(&s).unwrap().new_words, 1);
    assert_eq!(s.take_reads(), 1);
}

#[test]
fn a_bad_index_is_rebuilt() {
    let s = MemStorage::new();
    append(&s, "a", &[word("a", 0, false)]);
    s.blobs().put(INDEX_KEY, b"{ not json").unwrap();
    assert_eq!(pending(&s).unwrap().new_words, 1);
    // And it is replaced by a good one.
    let saved = s.blobs().get(INDEX_KEY).unwrap().unwrap();
    assert!(serde_json::from_slice::<Index>(&saved).is_ok());
}

#[test]
fn a_segment_that_disappears_leaves_the_counts() {
    let s = MemStorage::new();
    append(&s, "a", &[word("a", 0, false)]);
    // An index that still holds a segment the log no longer has.
    let mut index = Index::load(&s, &TuneState::default());
    index.segments.insert(
        "gone".to_owned(),
        SegmentCount {
            offset: 100,
            new_words: 99,
            new_corrections: 9,
            total_words: 99,
        },
    );
    s.blobs()
        .put(INDEX_KEY, &serde_json::to_vec(&index).unwrap())
        .unwrap();
    let p = pending(&s).unwrap();
    assert_eq!((p.total_words, p.new_words, p.new_corrections), (1, 1, 0));
}
