//! Hook host: wraps [`Event`]s into [`Record`]s (`v`, `seq`, `t`,
//! `session`) and runs every hook on them, in registration order.
//!
//! `Inline` runs hooks on the caller's thread (deterministic); `Background`
//! hands records to one worker thread over a channel, so the keystroke path
//! never waits for hook work or I/O. Hook `init` always runs on the
//! constructing thread, before the worker exists, so init summaries are
//! available immediately. Each hook call is guarded by `catch_unwind`: a
//! panicking hook is disabled with one diagnostic and the others continue.
//! Hooks flush after a `word_commit` once [`FLUSH_SECS`] of event time
//! passed since the last flush, and at shutdown. Failures never reach the
//! hot path: they come back as [`HookMsg::Diagnostic`]s.

use std::io;
use std::mem;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use nimk_hook::{Hook, HookCx, HookMsg};
use nimk_storage::{EventSink, Storage};
use nimk_types::{Event, Record, SCHEMA_VERSION};

/// Event-time seconds between two cadence flushes.
const FLUSH_SECS: f64 = 30.0;

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

/// One registered hook; `alive` turns false after it panics.
struct Slot {
    hook: Box<dyn Hook>,
    alive: bool,
}

/// Runs the hooks over records; collects their messages in `outbox`.
struct Runner {
    slots: Vec<Slot>,
    storage: Arc<dyn Storage>,
    last_flush_t: Option<f64>,
    last_t: f64,
    outbox: Vec<HookMsg>,
}

impl Runner {
    fn new(storage: Arc<dyn Storage>, hooks: Vec<Box<dyn Hook>>) -> Self {
        Self {
            slots: hooks
                .into_iter()
                .map(|hook| Slot { hook, alive: true })
                .collect(),
            storage,
            last_flush_t: None,
            last_t: 0.0,
            outbox: Vec::new(),
        }
    }

    /// Calls `f` on every live hook under `catch_unwind`; a panic disables
    /// the hook with one diagnostic.
    fn each(&mut self, mut f: impl FnMut(&mut dyn Hook, &mut HookCx<'_>)) {
        let blobs = self.storage.blobs();
        for slot in self.slots.iter_mut().filter(|s| s.alive) {
            let name = slot.hook.name();
            let mut cx = HookCx::new(name, blobs, &mut self.outbox);
            let hook = slot.hook.as_mut();
            if catch_unwind(AssertUnwindSafe(|| f(hook, &mut cx))).is_err() {
                slot.alive = false;
                self.outbox.push(HookMsg::Diagnostic(format!(
                    "hook {name}: panicked; disabled"
                )));
            }
        }
    }

    fn init(&mut self) {
        self.each(|hook, cx| hook.init(cx));
    }

    fn record(&mut self, rec: &Record) {
        self.last_t = rec.t;
        self.each(|hook, cx| hook.on_record(rec, cx));
        let due = matches!(rec.event, Event::WordCommit { .. })
            && self.last_flush_t.is_none_or(|t0| rec.t - t0 >= FLUSH_SECS);
        if due {
            // Cadence flush: failures are reported as diagnostics only.
            self.flush(rec.t);
        }
    }

    /// Flushes every live hook. Each failure becomes a diagnostic; the first
    /// one is also returned for the shutdown result.
    fn flush(&mut self, t: f64) -> Option<io::Error> {
        let mut first = None;
        self.each(|hook, cx| {
            if let Err(e) = hook.flush(cx) {
                cx.diagnostic(format!("flush failed: {e}"));
                if first.is_none() {
                    first = Some(e);
                }
            }
        });
        self.last_flush_t = Some(t);
        first
    }

    /// Final flush at shutdown.
    fn finish(&mut self) -> io::Result<()> {
        match self.flush(self.last_t) {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// Sends the runner's messages; returns whether the engine still listens.
/// The outbox is emptied either way so it never grows unbounded.
fn forward(outbox: &mut Vec<HookMsg>, msgs: &Sender<HookMsg>, open: bool) -> bool {
    let mut open = open;
    for m in outbox.drain(..) {
        open = open && msgs.send(m).is_ok();
    }
    open
}

/// Worker-thread loop: runs records until the channel closes, then flushes.
fn worker(mut runner: Runner, rx: Receiver<Record>, msgs: Sender<HookMsg>) -> io::Result<()> {
    let mut open = true;
    for rec in rx {
        runner.record(&rec);
        open = forward(&mut runner.outbox, &msgs, open);
    }
    let flushed = runner.finish();
    forward(&mut runner.outbox, &msgs, open);
    flushed
}

enum Backend {
    Inline(Runner),
    Background {
        tx: Sender<Record>,
        msgs: Receiver<HookMsg>,
        handle: JoinHandle<io::Result<()>>,
    },
    /// Hooks flushed and the worker joined.
    Finished,
}

/// Hook host for one session.
pub(crate) struct HookBus {
    session: String,
    seq: u64,
    backend: Backend,
    /// Hook messages collected but not yet handed out.
    pending: Vec<HookMsg>,
    /// The bus's own warnings (spawn failures, dropped records).
    diagnostics: Vec<String>,
}

impl HookBus {
    /// Creates the bus and runs every hook's `init` on this thread.
    /// `background` then moves the hooks to a worker thread; if spawning
    /// fails they run inline (reported as a diagnostic).
    #[must_use]
    pub(crate) fn new(
        session: String,
        storage: Arc<dyn Storage>,
        hooks: Vec<Box<dyn Hook>>,
        background: bool,
    ) -> Self {
        let mut runner = Runner::new(storage, hooks);
        runner.init();
        let pending = mem::take(&mut runner.outbox);
        let mut diagnostics = Vec::new();
        let backend = if background {
            let (tx, rx) = mpsc::channel::<Record>();
            let (mtx, mrx) = mpsc::channel::<HookMsg>();
            // The runner travels through a one-shot channel so it is not
            // lost if the thread cannot be spawned.
            let (rtx, rrx) = mpsc::channel::<Runner>();
            let spawned = std::thread::Builder::new()
                .name("nimk-hooks".into())
                .spawn(move || match rrx.recv() {
                    Ok(runner) => worker(runner, rx, mtx),
                    Err(_) => Ok(()),
                });
            match spawned {
                Ok(handle) => match rtx.send(runner) {
                    Ok(()) => Backend::Background {
                        tx,
                        msgs: mrx,
                        handle,
                    },
                    Err(mpsc::SendError(runner)) => {
                        diagnostics.push("hook thread exited early; running hooks inline".into());
                        Backend::Inline(runner)
                    }
                },
                Err(e) => {
                    diagnostics.push(format!(
                        "cannot spawn hook thread ({e}); running hooks inline"
                    ));
                    Backend::Inline(runner)
                }
            }
        } else {
            Backend::Inline(runner)
        };
        Self {
            session,
            seq: 0,
            backend,
            pending,
            diagnostics,
        }
    }

    /// Logs one event at event time `t`: every hook sees its record.
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
            Backend::Inline(runner) => runner.record(&record),
            Backend::Background { tx, .. } => {
                if tx.send(record).is_err() {
                    self.diagnostics
                        .push("hook thread gone; record dropped".into());
                }
            }
            Backend::Finished => {
                self.diagnostics
                    .push("hook bus stopped; record dropped".into());
            }
        }
    }

    /// Moves messages the hooks produced so far into `pending`.
    fn collect(&mut self) {
        match &mut self.backend {
            Backend::Inline(runner) => self.pending.append(&mut runner.outbox),
            Backend::Background { msgs, .. } => self.pending.extend(msgs.try_iter()),
            Backend::Finished => {}
        }
    }

    /// Hook summaries and diagnostics produced so far, in order.
    pub(crate) fn drain(&mut self) -> Vec<HookMsg> {
        self.collect();
        mem::take(&mut self.pending)
    }

    /// Drains warnings: the bus's own plus hook diagnostics not yet
    /// drained. Hook summaries stay queued for [`HookBus::drain`].
    pub(crate) fn take_diagnostics(&mut self) -> Vec<String> {
        self.collect();
        let mut d = mem::take(&mut self.diagnostics);
        let mut summaries = Vec::new();
        for m in mem::take(&mut self.pending) {
            match m {
                HookMsg::Diagnostic(msg) => d.push(msg),
                summary @ HookMsg::Summary { .. } => summaries.push(summary),
            }
        }
        self.pending = summaries;
        d
    }

    /// Flushes every hook and joins the worker; afterwards `pending` holds
    /// every message the hooks produced.
    fn finish(&mut self) -> io::Result<()> {
        match mem::replace(&mut self.backend, Backend::Finished) {
            Backend::Inline(mut runner) => {
                let flushed = runner.finish();
                self.pending.append(&mut runner.outbox);
                flushed
            }
            Backend::Background { tx, msgs, handle } => {
                // Closing the channel ends the worker loop; join before
                // collecting so nothing it sends is missed.
                drop(tx);
                let joined = handle.join();
                self.pending.extend(msgs.try_iter());
                joined.map_err(|_| io::Error::other("hook thread panicked"))?
            }
            Backend::Finished => Ok(()),
        }
    }

    /// Flushes and joins but keeps the bus for inspection.
    #[cfg(test)]
    fn shutdown_for_test(&mut self) {
        if let Err(e) = self.finish() {
            self.diagnostics
                .push(format!("hook bus shutdown failed: {e}"));
        }
    }

    /// Flushes every hook and stops the worker thread.
    ///
    /// # Errors
    /// Returns the first hook flush error, or an error if the worker
    /// panicked.
    pub(crate) fn shutdown(mut self) -> io::Result<()> {
        self.finish()
    }
}
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::io;
    use std::sync::{Arc, Mutex};

    use nimk_hook::{EventLogHook, Hook, HookCx};
    use nimk_storage::{EventSink, MemStorage};
    use nimk_types::{Event, Lang, Record, SwitchScope};

    use super::*;

    struct Count(Arc<Mutex<Vec<u64>>>);
    impl Hook for Count {
        fn name(&self) -> &'static str {
            "count"
        }
        fn on_record(&mut self, r: &Record, cx: &mut HookCx<'_>) {
            self.0.lock().unwrap().push(r.seq);
            cx.summary(format!("{}", r.seq));
        }
    }
    struct Boom;
    impl Hook for Boom {
        fn name(&self) -> &'static str {
            "boom"
        }
        fn on_record(&mut self, _r: &Record, _cx: &mut HookCx<'_>) {
            panic!("boom")
        }
    }
    fn ev() -> Event {
        Event::ManualSwitch {
            scope: SwitchScope::InWord,
            word_id: Some(1),
            to: Lang::Vi,
        }
    }

    #[test]
    fn panicking_hook_is_disabled_others_continue() {
        for background in [false, true] {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let hooks: Vec<Box<dyn Hook>> =
                vec![Box::new(Boom), Box::new(Count(Arc::clone(&seen)))];
            let mut bus = HookBus::new("s".into(), Arc::new(MemStorage::new()), hooks, background);
            bus.log(1.0, ev());
            bus.log(2.0, ev());
            bus.shutdown_for_test();
            assert_eq!(*seen.lock().unwrap(), vec![0, 1]);
            let d = bus.take_diagnostics().join("\n");
            assert_eq!(
                d.matches("hook boom").count(),
                1,
                "one diagnostic, then disabled: {d}"
            );
        }
    }

    #[test]
    fn inline_and_background_deliver_same_summaries() {
        let summaries = |background: bool| {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let hooks: Vec<Box<dyn Hook>> = vec![Box::new(Count(Arc::clone(&seen)))];
            let mut bus = HookBus::new("s".into(), Arc::new(MemStorage::new()), hooks, background);
            bus.log(1.0, ev());
            bus.log(2.0, ev());
            bus.shutdown_for_test();
            bus.drain()
                .into_iter()
                .filter(|m| matches!(m, HookMsg::Summary { .. }))
                .collect::<Vec<_>>()
        };
        assert_eq!(summaries(false), summaries(true));
        assert_eq!(summaries(false).len(), 2);
    }

    struct InitSummary;
    impl Hook for InitSummary {
        fn name(&self) -> &'static str {
            "init"
        }
        fn init(&mut self, cx: &mut HookCx<'_>) {
            cx.summary("ready".into());
        }
        fn on_record(&mut self, _r: &Record, _cx: &mut HookCx<'_>) {}
    }

    #[test]
    fn init_summary_is_available_before_any_record() {
        for background in [false, true] {
            let hooks: Vec<Box<dyn Hook>> = vec![Box::new(InitSummary)];
            let mut bus = HookBus::new("s".into(), Arc::new(MemStorage::new()), hooks, background);
            assert_eq!(
                bus.drain(),
                vec![HookMsg::Summary {
                    hook: "init",
                    json: "ready".into()
                }]
            );
            assert!(bus.shutdown().is_ok());
        }
    }

    struct Flushes(Arc<Mutex<usize>>);
    impl Hook for Flushes {
        fn name(&self) -> &'static str {
            "flushes"
        }
        fn on_record(&mut self, _r: &Record, _cx: &mut HookCx<'_>) {}
        fn flush(&mut self, _cx: &mut HookCx<'_>) -> io::Result<()> {
            *self.0.lock().unwrap() += 1;
            Ok(())
        }
    }
    fn commit() -> Event {
        Event::WordCommit {
            word_id: 1,
            raw: "a".into(),
            vi: "a".into(),
            en: "a".into(),
            mode: Lang::Vi,
            boundary: nimk_types::Boundary::Space,
        }
    }

    #[test]
    fn flushes_after_commit_every_30s_of_event_time_and_at_shutdown() {
        for background in [false, true] {
            let n = Arc::new(Mutex::new(0));
            let hooks: Vec<Box<dyn Hook>> = vec![Box::new(Flushes(Arc::clone(&n)))];
            let mut bus = HookBus::new("s".into(), Arc::new(MemStorage::new()), hooks, background);
            bus.log(1.0, commit()); // first commit: flush
            bus.log(10.0, commit()); // < 30 s since: none
            bus.log(45.0, ev()); // not a commit: none
            bus.log(45.0, commit()); // >= 30 s: flush
            assert!(bus.shutdown().is_ok()); // final flush
            assert_eq!(*n.lock().unwrap(), 3);
        }
    }

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

    fn run(background: bool) -> Vec<Vec<u8>> {
        let shared = Shared::default();
        let hooks: Vec<Box<dyn Hook>> = vec![Box::new(EventLogHook::new(Box::new(shared.clone())))];
        let mut bus = HookBus::new("s1".into(), Arc::new(MemStorage::new()), hooks, background);
        bus.log(1.5, ev());
        bus.log(2.0, ev());
        assert!(bus.take_diagnostics().is_empty());
        assert!(bus.shutdown().is_ok());
        shared.0.lock().unwrap().clone()
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
