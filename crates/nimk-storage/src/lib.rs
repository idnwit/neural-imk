//! Durable storage behind traits, so encryption or compression can be added
//! later as a decorator (`EncryptedStorage<S: Storage>`) without touching callers.
//!
//! Storage only sees bytes; serialization belongs to the layer above.
#![forbid(unsafe_code)]

mod fs;
mod fs_sink;
mod layered;
mod mem;
mod readonly;
mod validate;

use std::io;

pub use fs::PlainFsStorage;
pub use layered::LayeredStorage;
pub use mem::MemStorage;
pub use readonly::ReadOnlyDirBlobs;

/// Identifier of an event log segment. The sink chooses it (e.g.
/// `2026-11-08_<session>`); segments sort chronologically by string value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SegmentId(pub String);

/// Append-only event stream for one session. The sink names and rotates
/// its own segments (it may read the wall clock; callers cannot).
pub trait EventSink: Send {
    /// Appends one serialized record (no trailing newline).
    ///
    /// # Errors
    /// Returns an I/O error if the record cannot be written.
    fn append(&mut self, record: &[u8]) -> io::Result<()>;

    /// Flushes buffered records to durable storage.
    ///
    /// # Errors
    /// Returns an I/O error if flushing fails.
    fn flush(&mut self) -> io::Result<()>;
}

/// Iterator over the records of one segment.
pub type RecordIter<'a> = Box<dyn Iterator<Item = io::Result<Vec<u8>>> + 'a>;

/// Read side of the event stream.
pub trait EventSource {
    /// Lists segments in chronological order.
    ///
    /// # Errors
    /// Returns an I/O error if the listing fails.
    fn segments(&self) -> io::Result<Vec<SegmentId>>;

    /// Iterates the records of a segment in append order.
    ///
    /// # Errors
    /// Returns an I/O error if the segment cannot be opened.
    fn read_segment(&self, id: &SegmentId) -> io::Result<RecordIter<'_>>;

    /// Like [`EventSource::read_segment`], from byte `offset`, which must be
    /// the end of a record (`0`, or the sum of `len + 1` over the records
    /// already read): a reader that remembers where it stopped reads only what
    /// was appended since. The default skips the first records; a source over
    /// real files should seek instead.
    ///
    /// # Errors
    /// Returns an I/O error if the segment cannot be opened.
    fn read_segment_from(&self, id: &SegmentId, offset: u64) -> io::Result<RecordIter<'_>> {
        let mut skipped = 0u64;
        let mut records = self.read_segment(id)?;
        while skipped < offset {
            match records.next() {
                Some(Ok(r)) => skipped += r.len() as u64 + 1,
                Some(Err(e)) => return Err(e),
                None => break,
            }
        }
        Ok(Box::new(records))
    }
}

/// Which layer a blob is served from (see [`LayeredStorage`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobOrigin {
    /// Written by the user's own storage (a learned checkpoint, a setting).
    User,
    /// Shipped read-only with the app (a bundled default).
    Default,
}

/// Named blobs: model snapshots, config, replay buffer.
pub trait BlobStore: Send + Sync {
    /// Writes a blob atomically (readers see the old or the new value, never a mix).
    ///
    /// # Errors
    /// Returns an I/O error if the write fails.
    fn put(&self, key: &str, bytes: &[u8]) -> io::Result<()>;

    /// Reads a blob, `Ok(None)` if absent.
    ///
    /// # Errors
    /// Returns an I/O error if the read fails.
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>>;

    /// Lists keys starting with `prefix`, sorted.
    ///
    /// # Errors
    /// Returns an I/O error if the listing fails.
    fn list(&self, prefix: &str) -> io::Result<Vec<String>>;

    /// Removes a blob. Returns `true` if something was removed, `false` if
    /// it was absent.
    ///
    /// # Errors
    /// Returns an I/O error if removal fails, or `Unsupported` for a
    /// read-only store.
    fn delete(&self, key: &str) -> io::Result<bool>;

    /// Layer that [`get`](Self::get) would read `key` from, `Ok(None)` if absent.
    /// Plain stores are all [`BlobOrigin::User`].
    ///
    /// # Errors
    /// Returns an I/O error if the lookup fails.
    fn origin(&self, key: &str) -> io::Result<Option<BlobOrigin>> {
        Ok(self.get(key)?.map(|_| BlobOrigin::User))
    }
}

/// Root storage handle.
pub trait Storage: Send + Sync {
    /// Opens a writer for the event stream of `session` (an opaque id that
    /// the sink may embed in segment names).
    ///
    /// # Errors
    /// Returns an I/O error if the stream cannot be opened.
    fn event_sink(&self, session: &str) -> io::Result<Box<dyn EventSink>>;

    /// Opens a reader for the event stream.
    ///
    /// # Errors
    /// Returns an I/O error if the stream cannot be opened.
    fn event_source(&self) -> io::Result<Box<dyn EventSource + '_>>;

    /// Blob store.
    fn blobs(&self) -> &dyn BlobStore;
}
