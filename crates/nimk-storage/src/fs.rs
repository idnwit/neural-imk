use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::fs_sink::FsEventSink;
use crate::validate::{blob_key_segments, check_segment_id};
use crate::{BlobOrigin, BlobStore, EventSink, EventSource, RecordIter, SegmentId, Storage};

/// Extension of event segment files.
const SEGMENT_EXT: &str = "jsonl";

/// Plain files under a root directory:
/// `events/<segment>.jsonl` (one record per line) and `blobs/<key>`.
///
/// - Segments are named `<YYYY-MM-DD>_<session>` (UTC date, session reduced
///   to `[A-Za-z0-9_-]`); the sink rotates to a new file when the UTC date
///   changes. Within one day, segments sort by session id.
/// - A record is committed once its terminating `\n` is on disk; a trailing
///   line without `\n` (crash or concurrent write) is skipped by readers.
/// - Blob keys are `/`-separated relative names (`model/tiny_mlp/latest`)
///   mapped to subdirectories of `blobs/`.
#[derive(Debug, Clone)]
pub struct PlainFsStorage {
    root: PathBuf,
    blobs: FsBlobStore,
}

impl PlainFsStorage {
    /// Opens (creating if needed) storage rooted at `root`.
    ///
    /// # Errors
    /// Returns an I/O error if the directories cannot be created.
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root: PathBuf = root.into();
        let blobs_dir = root.join("blobs");
        fs::create_dir_all(root.join("events"))?;
        fs::create_dir_all(&blobs_dir)?;
        Ok(Self {
            root,
            blobs: FsBlobStore { dir: blobs_dir },
        })
    }

    fn events_dir(&self) -> PathBuf {
        self.root.join("events")
    }
}

impl Storage for PlainFsStorage {
    fn event_sink(&self, session: &str) -> io::Result<Box<dyn EventSink>> {
        Ok(Box::new(FsEventSink::open(self.events_dir(), session)?))
    }

    fn event_source(&self) -> io::Result<Box<dyn EventSource + '_>> {
        Ok(Box::new(FsEventSource {
            dir: self.events_dir(),
        }))
    }

    fn blobs(&self) -> &dyn BlobStore {
        &self.blobs
    }
}

/// Reads segment files from `events/`.
struct FsEventSource {
    dir: PathBuf,
}

impl EventSource for FsEventSource {
    fn segments(&self) -> io::Result<Vec<SegmentId>> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            let is_file = entry.file_type()?.is_file();
            if !is_file || path.extension().and_then(|e| e.to_str()) != Some(SEGMENT_EXT) {
                continue;
            }
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                && check_segment_id(stem).is_ok()
            {
                ids.push(SegmentId(stem.to_owned()));
            }
        }
        ids.sort();
        Ok(ids)
    }

    fn read_segment(&self, id: &SegmentId) -> io::Result<RecordIter<'_>> {
        self.read_segment_from(id, 0)
    }

    fn read_segment_from(&self, id: &SegmentId, offset: u64) -> io::Result<RecordIter<'_>> {
        check_segment_id(&id.0)?;
        let mut file = File::open(self.dir.join(format!("{}.{SEGMENT_EXT}", id.0)))?;
        if offset > 0 {
            file.seek(SeekFrom::Start(offset))?;
        }
        Ok(Box::new(LineIter {
            reader: BufReader::new(file),
            done: false,
        }))
    }
}

/// Yields `\n`-terminated lines without the newline; skips a final
/// unterminated line (an uncommitted, possibly partial record).
struct LineIter<R> {
    reader: R,
    done: bool,
}

impl<R: BufRead> Iterator for LineIter<R> {
    type Item = io::Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let mut line = Vec::new();
        match self.reader.read_until(b'\n', &mut line) {
            Ok(_) if line.last() == Some(&b'\n') => {
                line.pop();
                Some(Ok(line))
            }
            Ok(_) => {
                self.done = true;
                None
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

/// Counter that makes temp file names unique within the process.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Blob store over `blobs/`; writes are tmp file + fsync + rename.
#[derive(Debug, Clone)]
struct FsBlobStore {
    dir: PathBuf,
}

impl FsBlobStore {
    fn path_of(&self, key: &str) -> io::Result<PathBuf> {
        key_path(&self.dir, key)
    }
}

/// Maps a validated blob key to its file under `dir`.
pub(crate) fn key_path(dir: &Path, key: &str) -> io::Result<PathBuf> {
    let mut path = dir.to_path_buf();
    path.extend(blob_key_segments(key)?);
    Ok(path)
}

/// Reads a blob file; a missing file (or a directory in its place) is `None`.
pub(crate) fn read_blob(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::IsADirectory
                    | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Whether `path` is an existing regular file.
pub(crate) fn blob_exists(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file())
}

/// Lists the sorted keys under `dir` that start with `prefix`. A missing
/// `dir` lists nothing.
pub(crate) fn list_keys(dir: &Path, prefix: &str) -> io::Result<Vec<String>> {
    let mut keys = Vec::new();
    match collect_keys(dir, "", &mut keys) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(keys),
        Err(e) => return Err(e),
    }
    keys.retain(|k| k.starts_with(prefix));
    keys.sort();
    Ok(keys)
}

/// Writes `bytes` to `path` atomically via a hidden temp file in the same dir.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("blob path has no parent"))?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::other("blob path has no file name"))?;
    fs::create_dir_all(parent)?;
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".{name}.{}.{n}.tmp", std::process::id()));
    let result = (|| {
        let mut file = File::options().write(true).create_new(true).open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        // arch-lint: allow(no-silent-result-drop) reason="cleanup after a failed write; the write error is returned"
        let _ = fs::remove_file(&tmp);
        return result;
    }
    // Persist the rename itself; not all platforms allow fsync on a dir.
    if let Ok(dir) = File::open(parent) {
        // arch-lint: allow(no-silent-result-drop) reason="dir fsync is best effort; the data file is already synced"
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Recursively collects keys under `dir`, skipping hidden entries (temp files)
/// and names that are not valid UTF-8.
fn collect_keys(dir: &Path, prefix: &str, out: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let key = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_keys(&entry.path(), &key, out)?;
        } else if kind.is_file() {
            out.push(key);
        }
    }
    Ok(())
}

impl BlobStore for FsBlobStore {
    fn put(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        write_atomic(&self.path_of(key)?, bytes)
    }

    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        read_blob(&self.path_of(key)?)
    }

    fn list(&self, prefix: &str) -> io::Result<Vec<String>> {
        list_keys(&self.dir, prefix)
    }

    fn delete(&self, key: &str) -> io::Result<bool> {
        match fs::remove_file(self.path_of(key)?) {
            Ok(()) => Ok(true),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound
                        | io::ErrorKind::IsADirectory
                        | io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    fn origin(&self, key: &str) -> io::Result<Option<BlobOrigin>> {
        Ok(blob_exists(&self.path_of(key)?).then_some(BlobOrigin::User))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_iter_skips_unterminated_tail() {
        let data: &[u8] = b"a\n\nb\npartial";
        let lines: Vec<Vec<u8>> = LineIter {
            reader: data,
            done: false,
        }
        .collect::<io::Result<_>>()
        .unwrap();
        assert_eq!(lines, vec![b"a".to_vec(), Vec::new(), b"b".to_vec()]);
    }

    #[test]
    fn offset_stops_before_an_unfinished_line_and_resumes_after_it() {
        let dir = tempfile::tempdir().unwrap();
        let events = dir.path().join("events");
        fs::create_dir_all(&events).unwrap();
        let path = events.join("2026-11-12_a.jsonl");
        let source = FsEventSource { dir: events };
        let id = SegmentId("2026-11-12_a".to_owned());
        let read_from = |offset: u64| -> Vec<Vec<u8>> {
            source
                .read_segment_from(&id, offset)
                .unwrap()
                .collect::<io::Result<_>>()
                .unwrap()
        };

        fs::write(&path, "one\ntw").unwrap();
        assert_eq!(read_from(0), vec![b"one".to_vec()]);
        // A reader that stopped after "one" (4 bytes) sees "two" once it is done.
        assert!(read_from(4).is_empty());
        fs::write(&path, "one\ntwo\n").unwrap();
        assert_eq!(read_from(4), vec![b"two".to_vec()]);
    }
}
