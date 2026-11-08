//! The exported `extern "C"` functions.
//!
//! Every function tolerates null pointers and wraps its body in
//! `catch_unwind`; on any failure the answer is "pass the key through".

use std::any::Any;
use std::ffi::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use crate::abi::{
    NIMK_MODE_EN, NimkCtx, NimkKey, NimkOut, ctx_from_c, key_from_c, mode_code, read_cstr,
};
use crate::handle::NimkCore;

/// Best-effort text of a panic payload.
fn panic_message(payload: &(dyn Any + Send)) -> &str {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s
    } else {
        "non-string panic payload"
    }
}

/// Records a caught panic in the core's own diagnostics (bounded).
fn record_panic(core: &mut NimkCore, what: &str, payload: &(dyn Any + Send)) {
    // Cap on FFI-side diagnostics kept between two `nimk_take_diagnostics` calls
    // (local so cbindgen does not try to export it).
    const MAX_OWN_DIAGS: usize = 256;
    if core.own_diags.len() < MAX_OWN_DIAGS {
        let msg = format!("nimk-ffi: panic in {what}: {}", panic_message(payload));
        core.own_diags.push(msg);
    }
}

/// Shared body of every call that answers with a `NimkOut`.
///
/// Writes `NimkOut::pass_through()` to `out` first, so every early return and every
/// caught panic leaves the safe answer in place.
///
/// # Safety
/// `core` must be null or a live pointer from `nimk_new`; `out` must be null
/// or valid for writing one `NimkOut`.
unsafe fn answer(
    core: *mut NimkCore,
    out: *mut NimkOut,
    what: &str,
    f: impl FnOnce(&mut NimkCore) -> Option<NimkOut>,
) {
    if !out.is_null() {
        // SAFETY: non-null and valid for writes per the contract; `write`
        // does not read the (possibly uninitialized) old value.
        unsafe { out.write(NimkOut::pass_through()) };
    }
    // SAFETY: null or a live, exclusively used `Box<NimkCore>` per contract.
    let Some(core) = (unsafe { core.as_mut() }) else {
        return;
    };
    core.out.clear();
    match catch_unwind(AssertUnwindSafe(|| f(&mut *core))) {
        Ok(Some(answer)) if !out.is_null() => {
            // SAFETY: as above.
            unsafe { out.write(answer) };
        }
        Ok(_) => {}
        Err(payload) => {
            core.out.clear();
            record_panic(core, what, &*payload);
        }
    }
}

/// Creates the IME core. `data_dir` is the storage root (created if needed);
/// `app_version` is the IME build version. Returns null if the engine could
/// not be built; every other function accepts null and passes keys through.
///
/// # Safety
/// Both arguments must be null or NUL-terminated strings valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_new(
    data_dir: *const c_char,
    app_version: *const c_char,
) -> *mut NimkCore {
    let built = catch_unwind(|| {
        // SAFETY: null or NUL-terminated, valid for this call (contract).
        let dir = unsafe { read_cstr(data_dir) };
        // SAFETY: as above.
        let version = unsafe { read_cstr(app_version) }
            .map_or_else(|| "unknown".to_owned(), |v| v.into_owned());
        NimkCore::new(dir.as_deref(), version)
    });
    built.map_or(ptr::null_mut(), |core| Box::into_raw(Box::new(core)))
}

/// Destroys the core without flushing (prefer `nimk_shutdown`). Null is a no-op.
///
/// # Safety
/// `core` must be null or a pointer from `nimk_new` not yet freed; it must
/// not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_free(core: *mut NimkCore) {
    if core.is_null() {
        return;
    }
    // SAFETY: from `Box::into_raw` in `nimk_new`, freed exactly once (contract).
    let boxed = unsafe { Box::from_raw(core) };
    // A panicking destructor must not unwind into Swift. Nothing is left to
    // report to (the core is gone), so the panic is only contained.
    if let Err(payload) = catch_unwind(AssertUnwindSafe(move || drop(boxed))) {
        // Dropping a panic payload can panic again; leak it instead.
        std::mem::forget(payload);
    }
}

/// Flushes logs, saves the model and destroys the core. Returns `false` if
/// the engine reported an error or panicked (the core is freed regardless).
///
/// # Safety
/// Same as `nimk_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_shutdown(core: *mut NimkCore) -> bool {
    if core.is_null() {
        return false;
    }
    // SAFETY: from `Box::into_raw` in `nimk_new`, freed exactly once (contract).
    let boxed = unsafe { Box::from_raw(core) };
    match catch_unwind(AssertUnwindSafe(move || boxed.engine.shutdown())) {
        Ok(Ok(())) => true,
        // The core is gone, so there is nowhere to put the error text; Swift
        // logs the `false`.
        Ok(Err(_)) | Err(_) => false,
    }
}

/// Reports a new app context (activation, word start). The answer may carry
/// a mode change and an overlay request.
///
/// # Safety
/// `core`: null or live from `nimk_new`. `ctx`: null or a valid `NimkCtx`
/// whose `bundle_id` is null or NUL-terminated. `out`: null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_context(core: *mut NimkCore, ctx: *const NimkCtx, out: *mut NimkOut) {
    // SAFETY: pointer contracts forwarded from this function's.
    unsafe {
        answer(core, out, "nimk_context", |core| {
            // SAFETY: null or a valid `NimkCtx` (contract).
            let ctx = ctx_from_c(ctx.as_ref()?);
            let output = core.engine.on_context(&ctx);
            Some(core.out.fill(output))
        });
    }
}

/// Handles one key. Apply `commit`, then `marked`; return `consumed` from
/// `handle(_:client:)`.
///
/// # Safety
/// `core`: null or live from `nimk_new`. `key`: null or a valid `NimkKey`
/// whose `chars` is null or NUL-terminated. `out`: null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_key(core: *mut NimkCore, key: *const NimkKey, out: *mut NimkOut) {
    // SAFETY: pointer contracts forwarded from this function's.
    unsafe {
        answer(core, out, "nimk_key", |core| {
            // SAFETY: null or a valid `NimkKey` (contract).
            let key = key_from_c(key.as_ref()?)?;
            let output = core.engine.on_key(&key);
            Some(core.out.fill(output))
        });
    }
}

/// Commits the marked text as-is (focus loss, `commitComposition`).
///
/// # Safety
/// `core`: null or live from `nimk_new`. `out`: null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_commit_pending(
    core: *mut NimkCore,
    timestamp: f64,
    out: *mut NimkOut,
) {
    // SAFETY: pointer contracts forwarded from this function's.
    unsafe {
        answer(core, out, "nimk_commit_pending", |core| {
            let output = core.engine.commit_pending(timestamp);
            Some(core.out.fill(output))
        });
    }
}

/// Current mode: `NIMK_MODE_EN` or `NIMK_MODE_VI` (`EN` for a null core).
///
/// # Safety
/// `core` must be null or live from `nimk_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_mode(core: *const NimkCore) -> u8 {
    // SAFETY: null or live per contract; only shared access is needed.
    let Some(core) = (unsafe { core.as_ref() }) else {
        return NIMK_MODE_EN;
    };
    match catch_unwind(AssertUnwindSafe(|| mode_code(core.engine.mode()))) {
        Ok(mode) => mode,
        // Shared access only, so the panic cannot be recorded; EN = pass-through.
        Err(_) => NIMK_MODE_EN,
    }
}

/// Id of the policy serving predictions (e.g. `tiny_mlp@v3`), or null.
/// Valid until the next `nimk_policy_id` call on this core.
///
/// # Safety
/// `core` must be null or live from `nimk_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_policy_id(core: *mut NimkCore) -> *const c_char {
    // SAFETY: null or live and exclusively used per contract.
    let Some(core) = (unsafe { core.as_mut() }) else {
        return ptr::null();
    };
    match catch_unwind(AssertUnwindSafe(|| core.policy_id())) {
        Ok(p) => p,
        Err(payload) => {
            record_panic(core, "nimk_policy_id", &*payload);
            ptr::null()
        }
    }
}

/// Drains warnings as one newline-joined string, or null when there are
/// none. Valid until the next `nimk_take_diagnostics` call on this core.
///
/// # Safety
/// `core` must be null or live from `nimk_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_take_diagnostics(core: *mut NimkCore) -> *const c_char {
    // SAFETY: null or live and exclusively used per contract.
    let Some(core) = (unsafe { core.as_mut() }) else {
        return ptr::null();
    };
    match catch_unwind(AssertUnwindSafe(|| core.take_diagnostics())) {
        Ok(p) => p,
        Err(payload) => {
            record_panic(core, "nimk_take_diagnostics", &*payload);
            ptr::null()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::{CStr, CString};

    use super::*;
    use crate::abi::{NIMK_KIND_KEY_DOWN, NIMK_MODE_UNCHANGED, NIMK_OVERLAY_NONE};

    fn garbage_out() -> NimkOut {
        NimkOut {
            consumed: true,
            commit: c"junk".as_ptr(),
            marked: c"junk".as_ptr(),
            mode: 1,
            overlay: 2,
        }
    }

    fn assert_pass_through(out: &NimkOut) {
        assert!(!out.consumed);
        assert!(out.commit.is_null());
        assert!(out.marked.is_null());
        assert_eq!(out.mode, NIMK_MODE_UNCHANGED);
        assert_eq!(out.overlay, NIMK_OVERLAY_NONE);
    }

    fn key(chars: &CStr, ts: f64) -> NimkKey {
        NimkKey {
            kind: NIMK_KIND_KEY_DOWN,
            keycode: 0,
            modifiers: 0,
            chars: chars.as_ptr(),
            timestamp: ts,
        }
    }

    #[test]
    fn null_core_passes_through() {
        let k = key(c"a", 0.0);
        let mut out = garbage_out();
        // SAFETY: null core is allowed; `k` and `out` are live.
        unsafe { nimk_key(ptr::null_mut(), &k, &mut out) };
        assert_pass_through(&out);

        let mut out = garbage_out();
        // SAFETY: as above.
        unsafe { nimk_commit_pending(ptr::null_mut(), 1.0, &mut out) };
        assert_pass_through(&out);

        let ctx = NimkCtx {
            bundle_id: ptr::null(),
            field_id: 0,
            caret_rel_x: -1.0,
            caret_rel_y: -1.0,
            timestamp: 0.0,
        };
        let mut out = garbage_out();
        // SAFETY: as above.
        unsafe { nimk_context(ptr::null_mut(), &ctx, &mut out) };
        assert_pass_through(&out);
    }

    #[test]
    fn null_everything_is_safe() {
        // SAFETY: every function accepts null pointers.
        unsafe {
            nimk_key(ptr::null_mut(), ptr::null(), ptr::null_mut());
            nimk_context(ptr::null_mut(), ptr::null(), ptr::null_mut());
            nimk_commit_pending(ptr::null_mut(), 0.0, ptr::null_mut());
            assert_eq!(nimk_mode(ptr::null()), NIMK_MODE_EN);
            assert!(nimk_take_diagnostics(ptr::null_mut()).is_null());
            assert!(nimk_policy_id(ptr::null_mut()).is_null());
            assert!(!nimk_shutdown(ptr::null_mut()));
            nimk_free(ptr::null_mut());
        }
    }

    #[test]
    fn panic_message_handles_payload_kinds() {
        let s: Box<dyn Any + Send> = Box::new("boom");
        let owned: Box<dyn Any + Send> = Box::new(String::from("bang"));
        let other: Box<dyn Any + Send> = Box::new(7_u8);
        assert_eq!(panic_message(&*s), "boom");
        assert_eq!(panic_message(&*owned), "bang");
        assert_eq!(panic_message(&*other), "non-string panic payload");
    }

    /// Never unwinds into the caller, whether or not the engine is implemented.
    #[test]
    fn new_never_unwinds() {
        let dir = tempfile::tempdir().unwrap();
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();
        // SAFETY: valid strings; the result is null or a live core freed once.
        unsafe {
            let core = nimk_new(path.as_ptr(), c"test".as_ptr());
            nimk_free(core);
        }
    }

    fn new_core(dir: &tempfile::TempDir) -> *mut NimkCore {
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();
        // SAFETY: valid strings for the call.
        let core = unsafe { nimk_new(path.as_ptr(), c"test".as_ptr()) };
        assert!(!core.is_null(), "engine failed to build");
        core
    }

    #[test]
    fn letter_is_composed_then_committed() {
        let dir = tempfile::tempdir().unwrap();
        let core = new_core(&dir);
        let ctx = NimkCtx {
            bundle_id: c"com.apple.TextEdit".as_ptr(),
            field_id: 1,
            caret_rel_x: 0.5,
            caret_rel_y: 0.5,
            timestamp: 1.0,
        };
        let mut out = NimkOut::pass_through();
        // SAFETY: live core and pointers.
        unsafe {
            nimk_context(core, &ctx, &mut out);
            nimk_key(core, &key(c"a", 1.1), &mut out);
            assert!(out.consumed);
            assert_eq!(CStr::from_ptr(out.marked).to_str().unwrap(), "a");
            nimk_commit_pending(core, 1.2, &mut out);
            assert_eq!(CStr::from_ptr(out.commit).to_str().unwrap(), "a");
            assert!(nimk_mode(core) <= 1);
            let _ = nimk_take_diagnostics(core);
            assert!(!nimk_policy_id(core).is_null());
            assert!(nimk_shutdown(core));
        }
    }

    #[test]
    fn unknown_key_kind_passes_through_with_live_core() {
        let dir = tempfile::tempdir().unwrap();
        let core = new_core(&dir);
        let bad = NimkKey {
            kind: 9,
            keycode: 0,
            modifiers: 0,
            chars: ptr::null(),
            timestamp: 0.0,
        };
        let mut out = garbage_out();
        // SAFETY: live core and pointers.
        unsafe {
            nimk_key(core, &bad, &mut out);
            assert_pass_through(&out);
            nimk_free(core);
        }
    }
}
