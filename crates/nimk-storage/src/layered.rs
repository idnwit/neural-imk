use std::io;
use std::sync::Arc;

use crate::{BlobOrigin, BlobStore, EventSink, EventSource, Storage};

/// User storage over read-only defaults.
///
/// - Events go to the user layer only.
/// - `get` reads the user layer first, then the defaults.
/// - `put` writes the user layer; `delete` removes from the user layer only,
///   so deleting a learned blob "resets" it to the default.
/// - `list` is the sorted, de-duplicated union of both layers.
#[derive(Debug)]
pub struct LayeredStorage<U: Storage, D: BlobStore> {
    user: Arc<U>,
    blobs: LayeredBlobs<U, D>,
}

impl<U: Storage, D: BlobStore> LayeredStorage<U, D> {
    /// Layers `defaults` (read-only fallback) under `user`.
    #[must_use]
    pub fn new(user: U, defaults: D) -> Self {
        let user = Arc::new(user);
        let blobs = LayeredBlobs {
            user: Arc::clone(&user),
            defaults,
        };
        Self { user, blobs }
    }

    /// The user layer's blobs alone (no fallback to defaults).
    #[must_use]
    pub fn user_blobs(&self) -> &dyn BlobStore {
        self.user.blobs()
    }
}

impl<U: Storage, D: BlobStore> Storage for LayeredStorage<U, D> {
    fn event_sink(&self, session: &str) -> io::Result<Box<dyn EventSink>> {
        self.user.event_sink(session)
    }

    fn event_source(&self) -> io::Result<Box<dyn EventSource + '_>> {
        self.user.event_source()
    }

    fn blobs(&self) -> &dyn BlobStore {
        &self.blobs
    }
}

/// Blob view of [`LayeredStorage`].
#[derive(Debug)]
struct LayeredBlobs<U: Storage, D: BlobStore> {
    user: Arc<U>,
    defaults: D,
}

impl<U: Storage, D: BlobStore> BlobStore for LayeredBlobs<U, D> {
    fn put(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.user.blobs().put(key, bytes)
    }

    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        match self.user.blobs().get(key)? {
            Some(bytes) => Ok(Some(bytes)),
            None => self.defaults.get(key),
        }
    }

    fn list(&self, prefix: &str) -> io::Result<Vec<String>> {
        let mut keys = self.user.blobs().list(prefix)?;
        keys.extend(self.defaults.list(prefix)?);
        keys.sort();
        keys.dedup();
        Ok(keys)
    }

    fn delete(&self, key: &str) -> io::Result<bool> {
        self.user.blobs().delete(key)
    }

    fn origin(&self, key: &str) -> io::Result<Option<BlobOrigin>> {
        if self.user.blobs().origin(key)?.is_some() {
            return Ok(Some(BlobOrigin::User));
        }
        Ok(self.defaults.origin(key)?.map(|_| BlobOrigin::Default))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemStorage, PlainFsStorage, ReadOnlyDirBlobs};

    /// Defaults root (a `PlainFsStorage` root); open `blobs/` read-only.
    fn defaults_dir() -> (tempfile::TempDir, ReadOnlyDirBlobs) {
        let tmp = tempfile::tempdir().unwrap();
        let fs = PlainFsStorage::open(tmp.path()).unwrap();
        fs.blobs().put("model/m/latest", b"default").unwrap();
        fs.blobs().put("only_default", b"d").unwrap();
        let ro = ReadOnlyDirBlobs::open(tmp.path().join("blobs")).unwrap();
        (tmp, ro)
    }

    #[test]
    fn user_shadows_defaults_and_delete_resets() {
        let (_dflt, ro) = defaults_dir();
        let user = tempfile::tempdir().unwrap();
        let s = LayeredStorage::new(PlainFsStorage::open(user.path()).unwrap(), ro);
        let b = s.blobs();
        assert_eq!(b.get("model/m/latest").unwrap(), Some(b"default".to_vec()));
        assert_eq!(
            b.origin("model/m/latest").unwrap(),
            Some(BlobOrigin::Default)
        );
        assert_eq!(s.user_blobs().get("model/m/latest").unwrap(), None);

        b.put("model/m/latest", b"learned").unwrap();
        b.put("settings/x", b"s").unwrap();
        assert_eq!(b.get("model/m/latest").unwrap(), Some(b"learned".to_vec()));
        assert_eq!(b.origin("model/m/latest").unwrap(), Some(BlobOrigin::User));
        assert_eq!(
            s.user_blobs().get("model/m/latest").unwrap(),
            Some(b"learned".to_vec())
        );

        assert!(b.delete("model/m/latest").unwrap());
        assert!(!b.delete("model/m/latest").unwrap());
        assert_eq!(b.get("model/m/latest").unwrap(), Some(b"default".to_vec()));
        assert!(!b.delete("only_default").unwrap(), "defaults are untouched");
        assert_eq!(b.origin("missing").unwrap(), None);
    }

    #[test]
    fn list_is_sorted_union() {
        let (_dflt, ro) = defaults_dir();
        let s = LayeredStorage::new(MemStorage::new(), ro);
        s.blobs().put("model/m/latest", b"u").unwrap();
        s.blobs().put("a", b"u").unwrap();
        assert_eq!(
            s.blobs().list("").unwrap(),
            vec!["a", "model/m/latest", "only_default"]
        );
        assert_eq!(s.blobs().list("model/").unwrap(), vec!["model/m/latest"]);
    }

    #[test]
    fn events_go_to_the_user_layer() {
        let (_dflt, ro) = defaults_dir();
        let user = MemStorage::new();
        let s = LayeredStorage::new(user.clone(), ro);
        let mut sink = s.event_sink("sess").unwrap();
        sink.append(b"{}").unwrap();
        let src = user.event_source().unwrap();
        let segs = src.segments().unwrap();
        assert_eq!(segs.len(), 1);
        assert_eq!(s.event_source().unwrap().segments().unwrap(), segs);
    }
}
