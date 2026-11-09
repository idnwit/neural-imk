//! What a hook may touch: its own blob prefix, read-only legacy keys, and
//! an outbox of summaries and diagnostics for the engine.

use std::io;

use nimk_storage::BlobStore;

/// Message from a hook to the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookMsg {
    /// Latest summary (JSON) of `hook`.
    Summary {
        /// Name of the publishing hook.
        hook: &'static str,
        /// Summary as JSON.
        json: String,
    },
    /// Human-readable warning.
    Diagnostic(String),
}

/// Context handed to every hook call.
pub struct HookCx<'a> {
    name: &'static str,
    blobs: &'a dyn BlobStore,
    outbox: &'a mut Vec<HookMsg>,
}

impl<'a> HookCx<'a> {
    /// Context for the hook called `name`, writing messages to `outbox`.
    pub fn new(name: &'static str, blobs: &'a dyn BlobStore, outbox: &'a mut Vec<HookMsg>) -> Self {
        Self {
            name,
            blobs,
            outbox,
        }
    }

    fn key(&self, key: &str) -> String {
        format!("hooks/{}/{key}", self.name)
    }

    /// Reads `hooks/<name>/<key>`.
    ///
    /// # Errors
    /// Storage failure.
    pub fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        self.blobs.get(&self.key(key))
    }

    /// Writes `hooks/<name>/<key>` atomically.
    ///
    /// # Errors
    /// Storage failure.
    pub fn put(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.blobs.put(&self.key(key), bytes)
    }

    /// Reads any key, for one-time migrations.
    ///
    /// # Errors
    /// Storage failure.
    pub fn get_legacy(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        self.blobs.get(key)
    }

    /// Publishes this hook's latest summary.
    pub fn summary(&mut self, json: String) {
        self.outbox.push(HookMsg::Summary {
            hook: self.name,
            json,
        });
    }

    /// Reports a warning.
    pub fn diagnostic(&mut self, msg: String) {
        self.outbox
            .push(HookMsg::Diagnostic(format!("hook {}: {msg}", self.name)));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use nimk_storage::{MemStorage, Storage};

    #[test]
    fn keys_are_scoped_under_hooks_name() {
        let s = MemStorage::new();
        let mut out = Vec::new();
        let cx = HookCx::new("usage", s.blobs(), &mut out);
        cx.put("2026-11-09", b"{}").unwrap();
        assert_eq!(
            s.blobs().get("hooks/usage/2026-11-09").unwrap().as_deref(),
            Some(&b"{}"[..])
        );
        assert_eq!(cx.get("2026-11-09").unwrap().as_deref(), Some(&b"{}"[..]));
    }

    #[test]
    fn legacy_reads_any_key_and_summaries_queue() {
        let s = MemStorage::new();
        s.blobs().put("stats/gru_action", b"x").unwrap();
        let mut out = Vec::new();
        let mut cx = HookCx::new("model_stats", s.blobs(), &mut out);
        assert_eq!(
            cx.get_legacy("stats/gru_action").unwrap().as_deref(),
            Some(&b"x"[..])
        );
        cx.summary("{}".into());
        cx.diagnostic("d".into());
        assert_eq!(out.len(), 2);
    }
}
