//! Event logger: wraps [`Event`]s into [`Record`]s (`v`, `seq`, `t`,
//! `session`) and writes them as JSON to an [`EventSink`].
//!
//! `Inline` serializes and appends on the caller's thread (deterministic);
//! `Background` hands records to a writer thread over a channel, so the
//! keystroke path never waits for I/O. Write failures never propagate to
//! the hot path: they are counted and reported as diagnostics.

use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use nimk_storage::EventSink;
use nimk_types::{Event, Record, SCHEMA_VERSION};

/// Sink that discards everything (fallback when storage has no sink).
#[derive(Debug, Default)]
pub(crate) struct NullSink;

impl EventSink for NullSink {
    fn append(&mut self, _record: &[u8]) -> io::Result<()> {
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Serializes and appends one record; returns a description on failure.
fn write_record(sink: &mut dyn EventSink, record: &Record) -> Result<(), String> {
    let bytes = serde_json::to_vec(record).map_err(|e| format!("log serialize failed: {e}"))?;
    sink.append(&bytes)
        .map_err(|e| format!("log append failed (seq {}): {e}", record.seq))
}

/// Writer-thread loop: drains records until the channel closes, then flushes.
fn writer_loop(
    mut sink: Box<dyn EventSink>,
    rx: Receiver<Record>,
    diag: Sender<String>,
) -> io::Result<()> {
    // Once the engine stops listening for diagnostics, keep writing.
    let mut diag_open = true;
    for record in rx {
        if let Err(msg) = write_record(sink.as_mut(), &record) {
            diag_open = diag_open && diag.send(msg).is_ok();
        }
    }
    sink.flush()
}

enum Backend {
    Inline(Box<dyn EventSink>),
    Background {
        tx: Sender<Record>,
        diag: Receiver<String>,
        handle: JoinHandle<io::Result<()>>,
    },
}

/// Record writer for one session.
pub(crate) struct Logger {
    session: String,
    seq: u64,
    backend: Backend,
    diagnostics: Vec<String>,
}

impl Logger {
    /// Creates a logger. `background` spawns a writer thread; if spawning
    /// fails it falls back to inline writing (reported as a diagnostic).
    #[must_use]
    pub(crate) fn new(session: String, sink: Box<dyn EventSink>, background: bool) -> Self {
        let mut diagnostics = Vec::new();
        let backend = if background {
            let (tx, rx) = mpsc::channel::<Record>();
            let (dtx, drx) = mpsc::channel::<String>();
            // The sink travels through a one-shot channel so it is not lost
            // if the thread cannot be spawned.
            let (stx, srx) = mpsc::channel::<Box<dyn EventSink>>();
            let spawned = std::thread::Builder::new()
                .name("nimk-logger".into())
                .spawn(move || match srx.recv() {
                    Ok(sink) => writer_loop(sink, rx, dtx),
                    Err(_) => Ok(()),
                });
            match spawned {
                Ok(handle) => match stx.send(sink) {
                    Ok(()) => Backend::Background {
                        tx,
                        diag: drx,
                        handle,
                    },
                    Err(mpsc::SendError(sink)) => {
                        diagnostics.push("logger thread exited early; writing inline".into());
                        Backend::Inline(sink)
                    }
                },
                Err(e) => {
                    diagnostics.push(format!("cannot spawn logger thread ({e}); writing inline"));
                    Backend::Inline(sink)
                }
            }
        } else {
            Backend::Inline(sink)
        };
        Self {
            session,
            seq: 0,
            backend,
            diagnostics,
        }
    }

    /// Logs one event at event time `t`.
    pub(crate) fn log(&mut self, t: f64, event: Event) {
        let record = Record {
            v: SCHEMA_VERSION,
            seq: self.seq,
            t,
            session: self.session.clone(),
            event,
        };
        self.seq += 1;
        match &mut self.backend {
            Backend::Inline(sink) => {
                if let Err(msg) = write_record(sink.as_mut(), &record) {
                    self.diagnostics.push(msg);
                }
            }
            Backend::Background { tx, .. } => {
                if tx.send(record).is_err() {
                    self.diagnostics
                        .push("logger thread gone; record dropped".into());
                }
            }
        }
    }

    /// Drains warnings collected so far.
    pub(crate) fn take_diagnostics(&mut self) -> Vec<String> {
        if let Backend::Background { diag, .. } = &self.backend {
            self.diagnostics.extend(diag.try_iter());
        }
        std::mem::take(&mut self.diagnostics)
    }

    /// Flushes everything and stops the writer thread.
    ///
    /// # Errors
    /// Returns the sink's flush error, or an error if the writer panicked.
    pub(crate) fn shutdown(self) -> io::Result<()> {
        match self.backend {
            Backend::Inline(mut sink) => sink.flush(),
            Backend::Background { tx, handle, .. } => {
                drop(tx);
                handle
                    .join()
                    .map_err(|_| io::Error::other("logger thread panicked"))?
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

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

    fn event() -> Event {
        Event::ManualSwitch {
            scope: nimk_types::SwitchScope::InWord,
            word_id: Some(1),
            to: nimk_types::Lang::Vi,
        }
    }

    fn run(background: bool) -> Vec<Vec<u8>> {
        let shared = Shared::default();
        let mut logger = Logger::new("s1".into(), Box::new(shared.clone()), background);
        logger.log(1.5, event());
        logger.log(2.0, event());
        assert!(logger.take_diagnostics().is_empty());
        assert!(logger.shutdown().is_ok());
        shared.0.lock().map(|v| v.clone()).unwrap_or_default()
    }

    #[test]
    fn inline_and_background_write_same_bytes() {
        let a = run(false);
        let b = run(true);
        assert_eq!(a, b);
        assert_eq!(a.len(), 2);
        let rec: Record = serde_json::from_slice(&a[1]).unwrap();
        assert_eq!((rec.seq, rec.t, rec.session.as_str()), (1, 2.0, "s1"));
    }
}
