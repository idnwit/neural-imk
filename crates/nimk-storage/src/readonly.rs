use std::io;
use std::path::PathBuf;

use crate::fs::{blob_exists, key_path, list_keys, read_blob};
use crate::{BlobOrigin, BlobStore};

/// Read-only blob store over an existing directory (`<dir>/<key>`), used
/// for defaults shipped inside the app bundle.
///
/// Keys follow the same rules as [`PlainFsStorage`](crate::PlainFsStorage)
/// blobs. `put` and `delete` fail with `Unsupported`.
#[derive(Debug, Clone)]
pub struct ReadOnlyDirBlobs {
    dir: PathBuf,
}

impl ReadOnlyDirBlobs {
    /// Opens `dir` for reading.
    ///
    /// # Errors
    /// Returns `NotFound` or `NotADirectory` if `dir` is not a directory.
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir: PathBuf = dir.into();
        // arch-lint: allow(no-sync-io) reason="opening a directory-backed store is inherently file I/O"
        if !dir.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} is not a directory", dir.display()),
            ));
        }
        Ok(Self { dir })
    }
}

fn read_only() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "read-only blob store")
}

impl BlobStore for ReadOnlyDirBlobs {
    fn put(&self, _key: &str, _bytes: &[u8]) -> io::Result<()> {
        Err(read_only())
    }

    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        read_blob(&key_path(&self.dir, key)?)
    }

    fn list(&self, prefix: &str) -> io::Result<Vec<String>> {
        list_keys(&self.dir, prefix)
    }

    fn delete(&self, _key: &str) -> io::Result<bool> {
        Err(read_only())
    }

    fn origin(&self, key: &str) -> io::Result<Option<BlobOrigin>> {
        Ok(blob_exists(&key_path(&self.dir, key)?).then_some(BlobOrigin::Default))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlainFsStorage, Storage};

    fn fixture() -> (tempfile::TempDir, ReadOnlyDirBlobs) {
        let tmp = tempfile::tempdir().unwrap();
        let fs = PlainFsStorage::open(tmp.path()).unwrap();
        fs.blobs().put("model/a/latest", b"v1").unwrap();
        fs.blobs().put("top", b"t").unwrap();
        let blobs = ReadOnlyDirBlobs::open(tmp.path().join("blobs")).unwrap();
        (tmp, blobs)
    }

    #[test]
    fn reads_and_lists() {
        let (_tmp, b) = fixture();
        assert_eq!(b.get("model/a/latest").unwrap(), Some(b"v1".to_vec()));
        assert_eq!(b.get("model/a/none").unwrap(), None);
        assert_eq!(b.get("model").unwrap(), None, "directory is not a blob");
        assert_eq!(b.list("model/").unwrap(), vec!["model/a/latest"]);
        assert_eq!(b.list("").unwrap(), vec!["model/a/latest", "top"]);
        assert_eq!(b.origin("top").unwrap(), Some(BlobOrigin::Default),);
        assert_eq!(b.origin("nope").unwrap(), None);
    }

    #[test]
    fn rejects_writes_and_bad_keys() {
        let (_tmp, b) = fixture();
        assert_eq!(
            b.put("x", b"1").unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(
            b.delete("top").unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        for bad in ["", "/abs", "../x", "a//b", ".h"] {
            assert_eq!(
                b.get(bad).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn open_requires_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(ReadOnlyDirBlobs::open(tmp.path().join("missing")).is_err());
        // A blob file is not a directory.
        let fs = PlainFsStorage::open(tmp.path()).unwrap();
        fs.blobs().put("f", b"x").unwrap();
        assert!(ReadOnlyDirBlobs::open(tmp.path().join("blobs/f")).is_err());
    }
}
