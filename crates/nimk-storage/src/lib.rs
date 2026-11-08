//! Durable storage behind traits, so encryption or compression can be added
//! later as a decorator (`EncryptedStorage<S: Storage>`) without touching callers.
//!
//! Storage only sees bytes; serialization belongs to the layer above.
#![forbid(unsafe_code)]

mod fs;
mod fs_sink;
mod mem;
mod validate;

use std::io;

pub use fs::PlainFsStorage;
pub use mem::MemStorage;

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
