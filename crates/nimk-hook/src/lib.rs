//! Hooks: observers of the engine's `Record` stream (docs/adr/0014).
//!
//! A hook never changes the buffer or the output, never sees the engine,
//! never spawns threads and never reads the wall clock. The engine's
//! `hookbus` owns the thread and calls hooks in registration order.

mod cx;

use std::io;

use nimk_types::Record;

pub use cx::{HookCx, HookMsg};

/// One observer of the record stream.
pub trait Hook: Send {
    /// Stable name; also the blob prefix `hooks/<name>/`.
    fn name(&self) -> &'static str;
    /// Called once before the first record, on the constructing thread.
    fn init(&mut self, _cx: &mut HookCx<'_>) {}
    /// Called for every record, in `seq` order.
    fn on_record(&mut self, rec: &Record, cx: &mut HookCx<'_>);
    /// Persists state (host cadence and shutdown).
    ///
    /// # Errors
    /// Storage failures; the host reports them and keeps running.
    fn flush(&mut self, _cx: &mut HookCx<'_>) -> io::Result<()> {
        Ok(())
    }
}
