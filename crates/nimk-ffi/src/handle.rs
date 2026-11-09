//! The opaque handle behind `NimkCore *`: the engine plus the buffers that
//! keep returned C strings alive.

use std::collections::hash_map::RandomState;
use std::ffi::{CStr, CString, c_char};
use std::hash::{BuildHasher, Hasher};
use std::ptr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nimk_engine::{Engine, EngineConfig, SessionInfo};
use nimk_storage::{LayeredStorage, MemStorage, PlainFsStorage, ReadOnlyDirBlobs, Storage};

use crate::abi::{OutBuffers, to_cstring};

/// Opaque IME core. Create with `nimk_new`, destroy with `nimk_shutdown`
/// (or `nimk_free`). Not thread-safe: call from the IMK main thread only.
pub struct NimkCore {
    pub(crate) engine: Engine,
    pub(crate) out: OutBuffers,
    /// Warnings raised by the FFI layer itself (caught panics, fallbacks).
    pub(crate) own_diags: Vec<String>,
    diag_buf: Option<CString>,
    policy_buf: Option<CString>,
}

impl NimkCore {
    /// Builds the core. May panic inside the engine; callers catch unwinds.
    pub(crate) fn new(
        data_dir: Option<&str>,
        models_dir: Option<&str>,
        app_version: String,
    ) -> Self {
        let mut own_diags = Vec::new();
        let storage = open_storage(data_dir, models_dir, &mut own_diags);
        let session = SessionInfo {
            id: session_id(),
            wall_unix_ms: wall_unix_ms(),
            app_version,
            utc_offset_min: 0,
        };
        Self {
            engine: Engine::new(EngineConfig::default(), storage, session),
            out: OutBuffers::default(),
            own_diags,
            diag_buf: None,
            policy_buf: None,
        }
    }

    /// Applies the user's TOML override over the embedded defaults; returns
    /// how many values were rejected (each is reported as a diagnostic).
    pub(crate) fn set_config(&mut self, user: Option<&str>) -> u32 {
        let layers: Vec<&str> = user.into_iter().collect();
        let (cfg, diags) = EngineConfig::from_toml_layers(&layers);
        let n = u32::try_from(diags.len()).map_or(u32::MAX, |n| n);
        self.own_diags.extend(diags.iter().map(ToString::to_string));
        self.engine.reconfigure(cfg);
        n
    }

    /// Joins FFI and engine diagnostics with `\n`; null when there are none.
    /// The pointer stays valid until the next call of this method.
    pub(crate) fn take_diagnostics(&mut self) -> *const c_char {
        let mut lines = std::mem::take(&mut self.own_diags);
        lines.extend(self.engine.take_diagnostics());
        self.diag_buf = (!lines.is_empty()).then(|| to_cstring(lines.join("\n")));
        self.diag_buf.as_deref().map_or(ptr::null(), CStr::as_ptr)
    }

    /// Current policy id; valid until the next call of this method.
    pub(crate) fn policy_id(&mut self) -> *const c_char {
        let id = to_cstring(self.engine.policy_id().to_string());
        self.policy_buf.insert(id).as_ptr()
    }
}

/// Opens plain-file storage at `data_dir`, layered over the read-only
/// bundled defaults in `models_dir` when it is readable. Falls back to
/// in-memory storage (nothing persisted) when `data_dir` is missing or
/// cannot be opened; a missing `models_dir` only costs the pretrained start.
fn open_storage(
    data_dir: Option<&str>,
    models_dir: Option<&str>,
    diags: &mut Vec<String>,
) -> Arc<dyn Storage> {
    let user = match data_dir.filter(|d| !d.is_empty()) {
        Some(dir) => match PlainFsStorage::open(dir) {
            Ok(fs) => fs,
            Err(err) => {
                diags.push(format!(
                    "nimk-ffi: cannot open storage at {dir}: {err}; using memory"
                ));
                return Arc::new(MemStorage::new());
            }
        },
        None => {
            diags.push("nimk-ffi: no data_dir; using in-memory storage".to_owned());
            return Arc::new(MemStorage::new());
        }
    };
    match models_dir.filter(|d| !d.is_empty()) {
        Some(dir) => match ReadOnlyDirBlobs::open(dir) {
            Ok(defaults) => return Arc::new(LayeredStorage::new(user, defaults)),
            Err(err) => diags.push(format!(
                "nimk-ffi: no bundled models at {dir}: {err}; starting untrained"
            )),
        },
        None => diags.push("nimk-ffi: no models_dir; starting untrained".to_owned()),
    }
    Arc::new(user)
}

/// Random-ish session id: wall clock nanos and pid, mixed with the
/// per-process random hasher keys.
fn session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut h = RandomState::new().build_hasher();
    h.write_u128(nanos);
    h.write_u32(std::process::id());
    format!("{:016x}", h.finish())
}

/// Wall clock in Unix milliseconds (0 if the clock is before 1970).
fn wall_unix_ms() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d
            .as_secs()
            .saturating_mul(1000)
            .saturating_add(u64::from(d.subsec_millis())),
        // Clock set before 1970: report 0 rather than fail start-up.
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_layers_over_bundled_models_when_present() {
        let data = tempfile::tempdir().unwrap();
        let models = tempfile::tempdir().unwrap();
        PlainFsStorage::open(models.path())
            .unwrap()
            .blobs()
            .put("model/m/latest", b"b")
            .unwrap();
        let models_dir = models.path().join("blobs");
        let (d, m) = (data.path().to_str(), models_dir.to_str());
        let mut diags = Vec::new();
        let s = open_storage(d, m, &mut diags);
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(
            s.blobs().get("model/m/latest").unwrap(),
            Some(b"b".to_vec())
        );

        for bad in [None, Some(""), Some("/definitely/not/here")] {
            let mut diags = Vec::new();
            let s = open_storage(d, bad, &mut diags);
            assert_eq!(diags.len(), 1, "{bad:?}: {diags:?}");
            assert_eq!(s.blobs().get("model/m/latest").unwrap(), None);
        }
    }

    #[test]
    fn session_ids_are_hex_and_differ() {
        let a = session_id();
        let b = session_id();
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn wall_clock_is_after_2020() {
        assert!(wall_unix_ms() > 1_577_836_800_000);
    }
}
