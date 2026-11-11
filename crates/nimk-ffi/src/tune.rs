//! Fine-tuning on the user's own log (`nimk_tune_*`).
//!
//! Two of the four functions touch the core and must run on the IMK main
//! thread like every other call: [`nimk_tune_prepare`] (save the models so
//! the run reads current weights) and [`nimk_tune_install`] (load the
//! result). The other two take the data directory and never see the core, so
//! the host can run them on a background queue while typing goes on:
//! [`nimk_tune_pending`] counts the new data, [`nimk_tune_run`] fine-tunes.
//!
//! Host sequence for the menu button: `nimk_tune_pending` (show the count),
//! on click `nimk_tune_prepare`, then `nimk_tune_run` in the background,
//! then `nimk_tune_install`.

use std::ffi::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use nimk_storage::Storage;
use nimk_tune::{TuneOptions, TuneOutput, tune, tune_pending};

use crate::abi::{NimkOut, model_from_code, read_cstr};
use crate::handle::{NimkCore, open_storage};

/// `nimk_tune_run`: the model was fine-tuned and its result is waiting for
/// `nimk_tune_install`.
pub const NIMK_TUNE_OK: u8 = 0;
/// `nimk_tune_run`: fewer new words than `min_new_words`; nothing changed.
pub const NIMK_TUNE_TOO_LITTLE: u8 = 1;
/// `nimk_tune_run`: failed (no snapshot of that kind, unreadable log or
/// blobs, a panic); nothing changed.
pub const NIMK_TUNE_FAILED: u8 = 2;
/// `nimk_tune_run`: bad arguments (null pointer, unknown kind).
pub const NIMK_TUNE_BAD_ARGS: u8 = 3;

/// New data waiting for the next fine-tuning run, from `nimk_tune_pending`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NimkTunePending {
    /// Words with a final label since the last run.
    pub new_words: u64,
    /// Hotkey corrections among them.
    pub new_corrections: u64,
    /// Words in the whole log.
    pub total_words: u64,
    /// Wall clock of the last run, ms since the epoch; 0 = never run.
    pub last_unix_ms: u64,
    /// Runs so far.
    pub runs: u32,
}

/// What `nimk_tune_run` did.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NimkTuneReport {
    /// New words taught.
    pub new_words: u64,
    /// Older words taught again so the model does not drift.
    pub replay_words: u64,
    /// Keys taught.
    pub keys: u64,
}

/// Opens the same layered storage the core uses (user dir over bundled
/// models); `None` if `data_dir` is null.
///
/// # Safety
/// Both pointers null or NUL-terminated strings valid for the call.
unsafe fn storage_at(
    data_dir: *const c_char,
    models_dir: *const c_char,
) -> Option<Arc<dyn Storage>> {
    // SAFETY: null or NUL-terminated, valid for this call (contract).
    let dir = unsafe { read_cstr(data_dir) }?;
    // SAFETY: as above.
    let models = unsafe { read_cstr(models_dir) };
    let mut diags = Vec::new();
    Some(open_storage(Some(&dir), models.as_deref(), &mut diags))
}

/// Counts the data since the last fine-tuning run. Reads the whole log (a
/// second or so for a large one): call it off the main thread. Returns
/// `false` (and leaves `out` untouched) on null arguments or a read error.
///
/// # Safety
/// `data_dir`/`models_dir`: null or NUL-terminated, valid for the call
/// (`models_dir` may be null). `out`: null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_tune_pending(
    data_dir: *const c_char,
    models_dir: *const c_char,
    out: *mut NimkTunePending,
) -> bool {
    // SAFETY: null or writable per contract.
    let Some(out) = (unsafe { out.as_mut() }) else {
        return false;
    };
    // SAFETY: pointer contracts forwarded from this function's.
    let Some(storage) = (unsafe { storage_at(data_dir, models_dir) }) else {
        return false;
    };
    match catch_unwind(AssertUnwindSafe(|| tune_pending(storage.as_ref()))) {
        Ok(Ok(p)) => {
            *out = NimkTunePending {
                new_words: p.new_words,
                new_corrections: p.new_corrections,
                total_words: p.total_words,
                last_unix_ms: p.last_unix_ms.map_or(0, |ms| ms),
                runs: p.runs,
            };
            true
        }
        _ => false,
    }
}

/// Saves what the models learned so far (apply `out` like
/// `nimk_commit_pending`: a word being composed is committed) so that
/// `nimk_tune_run` reads current weights. Returns `false` for a null core.
///
/// # Safety
/// `core`: null or live from `nimk_new`. `out`: null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_tune_prepare(
    core: *mut NimkCore,
    timestamp: f64,
    out: *mut NimkOut,
) -> bool {
    let mut done = false;
    // SAFETY: pointer contracts forwarded from this function's.
    unsafe {
        crate::exports::answer(core, out, "nimk_tune_prepare", |core| {
            let output = core.engine.checkpoint_models(timestamp);
            done = true;
            Some(core.out.fill(output))
        });
    }
    done
}

/// Fine-tunes the model `kind` on the log since the last run and leaves the
/// result for `nimk_tune_install`. Blocks for seconds to minutes: call it
/// off the main thread, after `nimk_tune_prepare`, and never twice at once.
/// Returns a `NIMK_TUNE_*` code; `report` (optional) is filled on
/// `NIMK_TUNE_OK`.
///
/// # Safety
/// `data_dir`/`models_dir`: null or NUL-terminated, valid for the call
/// (`models_dir` may be null). `report`: null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_tune_run(
    data_dir: *const c_char,
    models_dir: *const c_char,
    kind: u8,
    min_new_words: u64,
    now_unix_ms: u64,
    report: *mut NimkTuneReport,
) -> u8 {
    let Some(kind) = model_from_code(kind) else {
        return NIMK_TUNE_BAD_ARGS;
    };
    // SAFETY: pointer contracts forwarded from this function's.
    let Some(storage) = (unsafe { storage_at(data_dir, models_dir) }) else {
        return NIMK_TUNE_BAD_ARGS;
    };
    let opts = TuneOptions {
        min_new_words,
        ..TuneOptions::default()
    };
    let run = catch_unwind(AssertUnwindSafe(|| {
        tune(&storage, kind, opts, now_unix_ms, TuneOutput::Result)
    }));
    match run {
        Ok(Ok(r)) => {
            // SAFETY: null or writable per contract.
            if let Some(report) = unsafe { report.as_mut() } {
                *report = NimkTuneReport {
                    new_words: r.new_words,
                    replay_words: r.replay_words,
                    keys: r.keys,
                };
            }
            NIMK_TUNE_OK
        }
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::InvalidInput => NIMK_TUNE_TOO_LITTLE,
        _ => NIMK_TUNE_FAILED,
    }
}

/// Loads the result of `nimk_tune_run` into the active model (apply `out`
/// like `nimk_commit_pending`). What the model learned online since
/// `nimk_tune_prepare` is replaced. Returns `false` (nothing changes) for a
/// null core or when there is no result to install.
///
/// # Safety
/// `core`: null or live from `nimk_new`. `out`: null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_tune_install(
    core: *mut NimkCore,
    timestamp: f64,
    out: *mut NimkOut,
) -> bool {
    let mut done = false;
    // SAFETY: pointer contracts forwarded from this function's.
    unsafe {
        crate::exports::answer(core, out, "nimk_tune_install", |core| {
            let (installed, output) = core.engine.install_tuned(timestamp);
            done = installed;
            Some(core.out.fill(output))
        });
    }
    done
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::ptr;

    use super::*;
    use crate::abi::NIMK_MODEL_GRU;

    #[test]
    fn bad_arguments_are_reported_not_fatal() {
        // SAFETY: null pointers are allowed everywhere; kind 9 is unknown.
        unsafe {
            assert_eq!(
                nimk_tune_run(c"/x".as_ptr(), ptr::null(), 9, 1, 0, ptr::null_mut()),
                NIMK_TUNE_BAD_ARGS
            );
            assert_eq!(
                nimk_tune_run(
                    ptr::null(),
                    ptr::null(),
                    NIMK_MODEL_GRU,
                    1,
                    0,
                    ptr::null_mut()
                ),
                NIMK_TUNE_BAD_ARGS
            );
            assert!(!nimk_tune_pending(
                ptr::null(),
                ptr::null(),
                ptr::null_mut()
            ));
            assert!(!nimk_tune_prepare(ptr::null_mut(), 0.0, ptr::null_mut()));
            assert!(!nimk_tune_install(ptr::null_mut(), 0.0, ptr::null_mut()));
        }
    }
}
