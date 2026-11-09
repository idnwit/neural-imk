//! The event log as a hook: every record, serialized as one JSON line.

use std::io;

use nimk_storage::EventSink;
use nimk_types::Record;

use crate::{Hook, HookCx};

/// Appends every record to the session's event sink.
pub struct EventLogHook {
    sink: Box<dyn EventSink>,
}

impl EventLogHook {
    /// Hook writing every record to `sink`.
    #[must_use]
    pub fn new(sink: Box<dyn EventSink>) -> Self {
        Self { sink }
    }
}

impl Hook for EventLogHook {
    fn name(&self) -> &'static str {
        "event_log"
    }

    fn on_record(&mut self, rec: &Record, cx: &mut HookCx<'_>) {
        let bytes = match serde_json::to_vec(rec) {
            Ok(b) => b,
            Err(e) => return cx.diagnostic(format!("log serialize failed: {e}")),
        };
        if let Err(e) = self.sink.append(&bytes) {
            cx.diagnostic(format!("log append failed (seq {}): {e}", rec.seq));
        }
    }

    fn flush(&mut self, _cx: &mut HookCx<'_>) -> io::Result<()> {
        self.sink.flush()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::sync::{Arc, Mutex};

    use nimk_storage::{MemStorage, Storage};
    use nimk_types::{Event, Lang, SCHEMA_VERSION, SwitchScope};

    use super::*;
    use crate::HookMsg;

    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<Vec<u8>>>>);

    impl EventSink for Shared {
        fn append(&mut self, record: &[u8]) -> io::Result<()> {
            self.0
                .lock()
                .map_err(|_| io::Error::other("poisoned"))?
                .push(record.to_vec());
            Ok(())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct Failing;
    impl EventSink for Failing {
        fn append(&mut self, _record: &[u8]) -> io::Result<()> {
            Err(io::Error::other("disk full"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn record(seq: u64, t: f64) -> Record {
        Record {
            v: SCHEMA_VERSION,
            seq,
            t,
            session: "s1".into(),
            event: Event::ManualSwitch {
                scope: SwitchScope::InWord,
                word_id: Some(1),
                to: Lang::Vi,
            },
        }
    }

    #[test]
    fn writes_each_record_as_its_json_bytes() {
        let shared = Shared::default();
        let mut hook = EventLogHook::new(Box::new(shared.clone()));
        let storage = MemStorage::new();
        let mut out = Vec::new();
        let mut cx = HookCx::new(hook.name(), storage.blobs(), &mut out);
        for (seq, t) in [(0, 1.5), (1, 2.0)] {
            hook.on_record(&record(seq, t), &mut cx);
        }
        assert!(hook.flush(&mut cx).is_ok());
        assert!(out.is_empty());
        let written = shared.0.lock().unwrap().clone();
        assert_eq!(written.len(), 2);
        assert_eq!(written[1], serde_json::to_vec(&record(1, 2.0)).unwrap());
        let rec: Record = serde_json::from_slice(&written[1]).unwrap();
        assert_eq!((rec.seq, rec.t, rec.session.as_str()), (1, 2.0, "s1"));
    }

    #[test]
    fn append_failure_is_a_diagnostic() {
        let mut hook = EventLogHook::new(Box::new(Failing));
        let storage = MemStorage::new();
        let mut out = Vec::new();
        let mut cx = HookCx::new(hook.name(), storage.blobs(), &mut out);
        hook.on_record(&record(7, 1.0), &mut cx);
        assert_eq!(
            out,
            vec![HookMsg::Diagnostic(
                "hook event_log: log append failed (seq 7): disk full".into()
            )]
        );
    }
}
