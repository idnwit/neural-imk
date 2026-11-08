use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::validate::{blob_key_segments, check_record, check_segment_id, sanitize_session};
use crate::{BlobStore, EventSink, EventSource, RecordIter, SegmentId, Storage};

/// Locks a mutex, recovering from poisoning: the maps hold plain data that a
/// panicking writer cannot leave half-updated in a harmful way.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Event segments plus the counter used to name them.
#[derive(Debug, Default)]
struct Events {
    next: u64,
    segments: BTreeMap<String, Vec<Vec<u8>>>,
}

/// In-memory storage for tests and replay. Clones share the same data.
///
/// - Segments are named `<counter>_<session>` with a zero-padded 10-digit
///   per-storage counter starting at 1, so they sort in creation order.
///   The session is reduced to `[A-Za-z0-9_-]` like on disk.
/// - Appends are visible to readers immediately; `flush` is a no-op.
/// - Records and blob keys are validated exactly as in
///   [`PlainFsStorage`](crate::PlainFsStorage).
#[derive(Debug, Clone, Default)]
pub struct MemStorage {
    events: Arc<Mutex<Events>>,
    blobs: MemBlobStore,
}

impl MemStorage {
    /// Creates empty storage.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Storage for MemStorage {
    fn event_sink(&self, session: &str) -> io::Result<Box<dyn EventSink>> {
        let session = sanitize_session(session)?;
        let mut events = lock(&self.events);
        events.next += 1;
        let id = format!("{:010}_{session}", events.next);
        events.segments.insert(id.clone(), Vec::new());
        Ok(Box::new(MemEventSink {
            events: Arc::clone(&self.events),
            id,
        }))
    }

    fn event_source(&self) -> io::Result<Box<dyn EventSource + '_>> {
        Ok(Box::new(MemEventSource {
            events: &self.events,
        }))
    }

    fn blobs(&self) -> &dyn BlobStore {
        &self.blobs
    }
}

/// Appends to one in-memory segment.
struct MemEventSink {
    events: Arc<Mutex<Events>>,
    id: String,
}

impl EventSink for MemEventSink {
    fn append(&mut self, record: &[u8]) -> io::Result<()> {
        check_record(record)?;
        lock(&self.events)
            .segments
            .entry(self.id.clone())
            .or_default()
            .push(record.to_vec());
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Reads in-memory segments.
struct MemEventSource<'a> {
    events: &'a Mutex<Events>,
}

impl EventSource for MemEventSource<'_> {
    fn segments(&self) -> io::Result<Vec<SegmentId>> {
        Ok(lock(self.events)
            .segments
            .keys()
            .map(|k| SegmentId(k.clone()))
            .collect())
    }

    /// Returns a snapshot of the segment taken at call time.
    fn read_segment(&self, id: &SegmentId) -> io::Result<RecordIter<'_>> {
        check_segment_id(&id.0)?;
        let records = lock(self.events)
            .segments
            .get(&id.0)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "segment not found"))?;
        Ok(Box::new(records.into_iter().map(Ok)))
    }
}

/// Shared in-memory blob map.
#[derive(Debug, Clone, Default)]
struct MemBlobStore {
    map: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

impl BlobStore for MemBlobStore {
    fn put(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        blob_key_segments(key)?;
        lock(&self.map).insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        blob_key_segments(key)?;
        Ok(lock(&self.map).get(key).cloned())
    }

    fn list(&self, prefix: &str) -> io::Result<Vec<String>> {
        Ok(lock(&self.map)
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
}
