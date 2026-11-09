//! The exported `extern "C"` functions.
//!
//! Every function tolerates null pointers and wraps its body in
//! `catch_unwind`; on any failure the answer is "pass the key through".

use std::any::Any;
use std::ffi::{CStr, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use crate::abi::{
    NIMK_CHECKPOINT_BUNDLED, NIMK_CHECKPOINT_NONE, NIMK_CHECKPOINT_USER, NIMK_MODE_EN,
    NIMK_MODEL_LINEAR, NimkCtx, NimkKey, NimkModelStats, NimkOut, ctx_from_c, key_from_c,
    mode_code, model_code, model_from_code, read_cstr,
};
use nimk_engine::CheckpointSource;

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

/// Creates the IME core. `data_dir` is the user storage root (created if
/// needed; the host resolves it from the OS, e.g. Application Support).
/// `models_dir` is the read-only directory of bundled pretrained models
/// (`<models_dir>/model/<kind>/latest`), or null: the core then starts
/// untrained and says so in its diagnostics. `app_version` is the IME build
/// version. Returns null if the engine could not be built; every other
/// function accepts null and passes keys through.
///
/// # Safety
/// All arguments must be null or NUL-terminated strings valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_new(
    data_dir: *const c_char,
    models_dir: *const c_char,
    app_version: *const c_char,
) -> *mut NimkCore {
    let built = catch_unwind(|| {
        // SAFETY: null or NUL-terminated, valid for this call (contract).
        let dir = unsafe { read_cstr(data_dir) };
        // SAFETY: as above.
        let models = unsafe { read_cstr(models_dir) };
        // SAFETY: as above.
        let version = unsafe { read_cstr(app_version) }
            .map_or_else(|| "unknown".to_owned(), |v| v.into_owned());
        NimkCore::new(dir.as_deref(), models.as_deref(), version)
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

/// Commits the marked text as-is: the word being composed or the pending word
/// with its boundary characters (focus loss, `commitComposition`).
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

/// Switches the action model (`NIMK_MODEL_*`). Commits a word being composed
/// (apply `out` like `nimk_commit_pending`), saves the old model's checkpoint,
/// loads the new one and remembers the choice. An unknown `kind` passes through.
///
/// # Safety
/// `core`: null or live from `nimk_new`. `out`: null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_set_model(
    core: *mut NimkCore,
    kind: u8,
    timestamp: f64,
    out: *mut NimkOut,
) -> bool {
    let mut switched = false;
    // SAFETY: pointer contracts forwarded from this function's.
    unsafe {
        answer(core, out, "nimk_set_model", |core| {
            let kind = model_from_code(kind)?;
            let output = core.engine.switch_model(kind, timestamp);
            switched = true;
            Some(core.out.fill(output))
        });
    }
    switched
}

/// Active model: `NIMK_MODEL_LINEAR` or `NIMK_MODEL_GRU` (`LINEAR` for a null core).
///
/// # Safety
/// `core` must be null or live from `nimk_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_model(core: *const NimkCore) -> u8 {
    // SAFETY: null or live per contract; only shared access is needed.
    let Some(core) = (unsafe { core.as_ref() }) else {
        return NIMK_MODEL_LINEAR;
    };
    match catch_unwind(AssertUnwindSafe(|| model_code(core.engine.model_kind()))) {
        Ok(code) => code,
        // Shared access only, so the panic cannot be recorded.
        Err(_) => NIMK_MODEL_LINEAR,
    }
}

/// Resets the active model: commits a word being composed (apply `out` like
/// `nimk_commit_pending`), deletes the user checkpoint and reloads the
/// bundled pretrained model (untrained if the app ships none). Returns
/// `false` for a null core.
///
/// # Safety
/// `core`: null or live from `nimk_new`. `out`: null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_reset_model(
    core: *mut NimkCore,
    timestamp: f64,
    out: *mut NimkOut,
) -> bool {
    let mut done = false;
    // SAFETY: pointer contracts forwarded from this function's.
    unsafe {
        answer(core, out, "nimk_reset_model", |core| {
            let output = core.engine.reset_model(timestamp);
            done = true;
            Some(core.out.fill(output))
        });
    }
    done
}

/// Where model `kind` would be loaded from: `NIMK_CHECKPOINT_NONE`,
/// `NIMK_CHECKPOINT_BUNDLED` or `NIMK_CHECKPOINT_USER` (`NONE` for a null
/// core or an unknown kind).
///
/// # Safety
/// `core` must be null or live from `nimk_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_checkpoint_source(core: *const NimkCore, kind: u8) -> u8 {
    // SAFETY: null or live per contract; only shared access is needed.
    let Some(core) = (unsafe { core.as_ref() }) else {
        return NIMK_CHECKPOINT_NONE;
    };
    // Shared access only, so a panic cannot be recorded; it reads as "none".
    match catch_unwind(AssertUnwindSafe(|| {
        match model_from_code(kind).map(|k| core.engine.checkpoint_source(k)) {
            Some(CheckpointSource::User) => NIMK_CHECKPOINT_USER,
            Some(CheckpointSource::Bundled) => NIMK_CHECKPOINT_BUNDLED,
            Some(CheckpointSource::None) | None => NIMK_CHECKPOINT_NONE,
        }
    })) {
        Ok(code) => code,
        Err(_) => NIMK_CHECKPOINT_NONE,
    }
}

/// Fills `out` with the counters of model `kind` (see [`NimkModelStats`]).
/// Returns `false` (and leaves `out` untouched) for a null core or `out` or
/// an unknown kind.
///
/// # Safety
/// `core`: null or live from `nimk_new`. `out`: null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_model_stats(
    core: *const NimkCore,
    kind: u8,
    out: *mut NimkModelStats,
) -> bool {
    // SAFETY: null or live per contract; only shared access is needed.
    let (Some(core), Some(out)) = (unsafe { core.as_ref() }, unsafe { out.as_mut() }) else {
        return false;
    };
    let stats = catch_unwind(AssertUnwindSafe(|| {
        model_from_code(kind).map(|k| core.engine.model_stats(k))
    }));
    match stats {
        Ok(Some(s)) => {
            *out = s.into();
            true
        }
        _ => false,
    }
}

/// Turns online training of model `kind` on or off (a frozen model is not
/// updated, saved or shadow-measured) and remembers the choice. Returns
/// `false` for a null core or an unknown kind.
///
/// # Safety
/// `core` must be null or live from `nimk_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_set_online_training(core: *mut NimkCore, kind: u8, on: bool) -> bool {
    // SAFETY: null or live per contract, exclusive for this call.
    let Some(core) = (unsafe { core.as_mut() }) else {
        return false;
    };
    let Some(kind) = model_from_code(kind) else {
        return false;
    };
    catch_unwind(AssertUnwindSafe(|| {
        core.engine.set_online_training(kind, on)
    }))
    .is_ok()
}

/// Whether online training of model `kind` is on (`false` for a null core
/// or an unknown kind).
///
/// # Safety
/// `core` must be null or live from `nimk_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_online_training(core: *const NimkCore, kind: u8) -> bool {
    // SAFETY: null or live per contract; only shared access is needed.
    let Some(core) = (unsafe { core.as_ref() }) else {
        return false;
    };
    // Shared access only, so a panic cannot be recorded; it reads as "off".
    catch_unwind(AssertUnwindSafe(|| {
        model_from_code(kind).is_some_and(|k| core.engine.online_training(k))
    }))
    .is_ok_and(|on| on)
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

/// Latest summary JSON of hook `name` ("model_stats", "usage"), or null.
/// The string stays valid until the next `nimk_hook_summary` call on `core`.
///
/// # Safety
/// `core` must be null or live from `nimk_new`; `name` null or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_hook_summary(
    core: *mut NimkCore,
    name: *const c_char,
) -> *const c_char {
    // SAFETY: null or live and exclusively used per contract.
    let Some(core) = (unsafe { core.as_mut() }) else {
        return ptr::null();
    };
    if name.is_null() {
        return ptr::null();
    }
    // SAFETY: non-null NUL-terminated string per contract.
    let Ok(name) = (unsafe { CStr::from_ptr(name) }).to_str() else {
        return ptr::null();
    };
    match catch_unwind(AssertUnwindSafe(|| core.hook_summary(name))) {
        Ok(p) => p,
        Err(payload) => {
            record_panic(core, "nimk_hook_summary", &*payload);
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

/// Applies the user's TOML override (`config.toml` text, or null for none)
/// on top of the embedded defaults and reconfigures the live engine.
/// Rejected values keep their default and are reported through
/// `nimk_take_diagnostics`; returns how many were rejected. Model, seed and
/// trainer settings apply at the next start. A null core returns 0.
///
/// # Safety
/// `core`: null or live from `nimk_new`. `config_toml`: null or a
/// NUL-terminated string valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_set_config(core: *mut NimkCore, config_toml: *const c_char) -> u32 {
    // SAFETY: null or live and exclusively used per contract.
    let Some(core) = (unsafe { core.as_mut() }) else {
        return 0;
    };
    // SAFETY: null or NUL-terminated and valid for this call (contract).
    let text = unsafe { read_cstr(config_toml) }.map(std::borrow::Cow::into_owned);
    match catch_unwind(AssertUnwindSafe(|| core.set_config(text.as_deref()))) {
        Ok(rejected) => rejected,
        Err(payload) => {
            record_panic(core, "nimk_set_config", &*payload);
            0
        }
    }
}

/// Seconds of silence after which the host should call
/// `nimk_commit_pending` while text is marked; 0 = never. Depends on the
/// focused app's category, so read it after each key. 0 for a null core.
///
/// # Safety
/// `core` must be null or live from `nimk_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_idle_commit_secs(core: *const NimkCore) -> f64 {
    // SAFETY: null or live per contract; only shared access is needed.
    let Some(core) = (unsafe { core.as_ref() }) else {
        return 0.0;
    };
    catch_unwind(AssertUnwindSafe(|| core.engine.idle_commit_secs())).map_or(0.0, |secs| secs)
}

/// The caret moved without a key (mouse click): raw mode ends. Null is a no-op.
///
/// # Safety
/// `core` must be null or live from `nimk_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_note_navigation(core: *mut NimkCore) {
    // SAFETY: null or live and exclusively used per contract.
    let Some(core) = (unsafe { core.as_mut() }) else {
        return;
    };
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| core.engine.note_navigation())) {
        record_panic(core, "nimk_note_navigation", &*payload);
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::{CStr, CString};

    use super::*;
    use crate::abi::{NIMK_KIND_KEY_DOWN, NIMK_MODE_UNCHANGED, NIMK_MODEL_GRU, NIMK_OVERLAY_NONE};
    use nimk_storage::{PlainFsStorage, Storage};

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
            assert!(nimk_hook_summary(ptr::null_mut(), c"usage".as_ptr()).is_null());
            assert!(!nimk_set_model(ptr::null_mut(), 1, 0.0, ptr::null_mut()));
            assert_eq!(nimk_model(ptr::null()), NIMK_MODEL_LINEAR);
            assert_eq!(nimk_checkpoint_source(ptr::null(), 0), NIMK_CHECKPOINT_NONE);
            assert!(!nimk_reset_model(ptr::null_mut(), 0.0, ptr::null_mut()));
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
            let core = nimk_new(path.as_ptr(), ptr::null(), c"test".as_ptr());
            nimk_free(core);
        }
    }

    /// ABI code of the product default model and of the other one.
    fn default_and_other() -> (u8, u8) {
        let default = model_code(nimk_engine::EngineConfig::default().policy);
        let other = if default == NIMK_MODEL_GRU {
            NIMK_MODEL_LINEAR
        } else {
            NIMK_MODEL_GRU
        };
        (default, other)
    }

    fn new_core(dir: &tempfile::TempDir) -> *mut NimkCore {
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();
        // SAFETY: valid strings for the call.
        let core = unsafe { nimk_new(path.as_ptr(), ptr::null(), c"test".as_ptr()) };
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
    fn hook_summary_reports_usage_after_a_word() {
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
            assert!(nimk_hook_summary(core, ptr::null()).is_null());
            assert!(nimk_hook_summary(core, c"nope".as_ptr()).is_null());
            nimk_context(core, &ctx, &mut out);
            nimk_key(core, &key(c"a", 1.1), &mut out);
            nimk_key(core, &key(c" ", 1.2), &mut out);
            // Hooks may run on a worker thread; their summaries are picked up
            // on later engine calls, so poll with a bounded wait.
            let mut p = ptr::null();
            for i in 0..200 {
                nimk_commit_pending(core, 1.3 + f64::from(i) * 0.01, &mut out);
                p = nimk_hook_summary(core, c"usage".as_ptr());
                if !p.is_null() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(!p.is_null());
            let json = CStr::from_ptr(p).to_str().unwrap();
            assert!(
                json.starts_with('{') && json.contains("\"total\""),
                "{json}"
            );
            assert!(nimk_shutdown(core));
        }
    }

    #[test]
    fn set_model_switches_and_reports_checkpoints() {
        let dir = tempfile::tempdir().unwrap();
        let core = new_core(&dir);
        let ctx = NimkCtx {
            bundle_id: c"com.apple.TextEdit".as_ptr(),
            field_id: 1,
            caret_rel_x: 0.5,
            caret_rel_y: 0.5,
            timestamp: 1.0,
        };
        let (default, other) = default_and_other();
        let mut out = NimkOut::pass_through();
        // SAFETY: live core and pointers.
        unsafe {
            assert_eq!(nimk_model(core), default);
            assert_eq!(nimk_checkpoint_source(core, other), NIMK_CHECKPOINT_NONE);
            nimk_context(core, &ctx, &mut out);
            nimk_key(core, &key(c"a", 1.1), &mut out);
            assert!(nimk_set_model(core, other, 1.2, &mut out));
            assert_eq!(CStr::from_ptr(out.commit).to_str().unwrap(), "a");
            assert_eq!(nimk_model(core), other);
            assert_eq!(nimk_checkpoint_source(core, default), NIMK_CHECKPOINT_USER);
            // The other model trained in the background as a shadow.
            assert_eq!(nimk_checkpoint_source(core, other), NIMK_CHECKPOINT_USER);
            assert_eq!(nimk_checkpoint_source(core, 9), NIMK_CHECKPOINT_NONE);
            // Unknown kind: passed through, nothing changes.
            let mut out = garbage_out();
            assert!(!nimk_set_model(core, 9, 1.3, &mut out));
            assert_pass_through(&out);
            assert_eq!(nimk_model(core), other);
            assert!(nimk_shutdown(core));
        }
        let core = new_core(&dir);
        // SAFETY: live core.
        unsafe {
            assert_eq!(nimk_model(core), other, "choice persisted");
            nimk_free(core);
        }
    }

    #[test]
    fn model_stats_and_online_training_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let core = new_core(&dir);
        let (default, other) = default_and_other();
        let mut stats = NimkModelStats::default();
        // SAFETY: live core and pointers.
        unsafe {
            assert!(nimk_online_training(core, default));
            assert!(nimk_online_training(core, other));
            assert!(nimk_model_stats(core, default, &mut stats));
            assert_eq!(stats, NimkModelStats::default());
            assert!(!nimk_model_stats(core, 9, &mut stats));
            assert!(!nimk_model_stats(core, default, ptr::null_mut()));
            assert!(!nimk_model_stats(ptr::null(), default, &mut stats));
            assert!(!nimk_online_training(core, 9));
            assert!(!nimk_set_online_training(core, 9, false));
            assert!(!nimk_set_online_training(ptr::null_mut(), other, false));
            assert!(nimk_set_online_training(core, other, false));
            assert!(!nimk_online_training(core, other));
            assert!(nimk_online_training(core, default));
            assert!(nimk_shutdown(core));
        }
        let core = new_core(&dir);
        // SAFETY: live core.
        unsafe {
            assert!(!nimk_online_training(core, other), "toggle persisted");
            assert!(nimk_online_training(core, default));
            nimk_free(core);
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

    #[test]
    fn bundled_models_load_and_reset_restores_them() {
        // Produce a checkpoint with one core, then bundle it for another.
        let seed_dir = tempfile::tempdir().unwrap();
        let core = new_core(&seed_dir);
        // SAFETY: live core.
        assert!(unsafe { nimk_shutdown(core) });
        let kind = nimk_engine::EngineConfig::default().policy;
        let (default, other) = default_and_other();
        let key = format!("model/{}/latest", kind.name());
        let ckpt = PlainFsStorage::open(seed_dir.path())
            .unwrap()
            .blobs()
            .get(&key)
            .unwrap()
            .unwrap();
        let models_root = tempfile::tempdir().unwrap();
        PlainFsStorage::open(models_root.path())
            .unwrap()
            .blobs()
            .put(&key, &ckpt)
            .unwrap();
        let models = models_root.path().join("blobs");

        let dir = tempfile::tempdir().unwrap();
        let data = CString::new(dir.path().to_str().unwrap()).unwrap();
        let models_c = CString::new(models.to_str().unwrap()).unwrap();
        // SAFETY: valid strings; the core is live until shut down.
        unsafe {
            let core = nimk_new(data.as_ptr(), models_c.as_ptr(), c"test".as_ptr());
            assert!(!core.is_null());
            assert_eq!(
                nimk_checkpoint_source(core, default),
                NIMK_CHECKPOINT_BUNDLED
            );
            assert_eq!(nimk_checkpoint_source(core, other), NIMK_CHECKPOINT_NONE);
            assert!(
                nimk_take_diagnostics(core).is_null(),
                "bundled loads cleanly"
            );
            assert!(nimk_shutdown(core));

            let core = nimk_new(data.as_ptr(), models_c.as_ptr(), c"test".as_ptr());
            assert_eq!(nimk_checkpoint_source(core, default), NIMK_CHECKPOINT_USER);
            let mut out = NimkOut::pass_through();
            assert!(nimk_reset_model(core, 2.0, &mut out));
            assert_eq!(
                nimk_checkpoint_source(core, default),
                NIMK_CHECKPOINT_BUNDLED
            );
            nimk_free(core);
        }
    }

    /// The models produced by `make models` load without diagnostics.
    /// Skipped when they have not been built.
    #[test]
    fn built_bundle_loads_cleanly() {
        let bundle =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/models/bundle");
        // arch-lint: allow(no-sync-io) reason="test skips when the bundle was not built"
        if !bundle.is_dir() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let data = CString::new(dir.path().to_str().unwrap()).unwrap();
        let models = CString::new(bundle.to_str().unwrap()).unwrap();
        // SAFETY: valid strings; the core is live until freed.
        unsafe {
            let core = nimk_new(data.as_ptr(), models.as_ptr(), c"test".as_ptr());
            assert!(!core.is_null());
            // Check every source first: switching away saves the old model as
            // a user checkpoint, which is not a bundle problem.
            for kind in [NIMK_MODEL_LINEAR, NIMK_MODEL_GRU] {
                assert_eq!(nimk_checkpoint_source(core, kind), NIMK_CHECKPOINT_BUNDLED);
            }
            assert!(nimk_take_diagnostics(core).is_null());
            for kind in [NIMK_MODEL_LINEAR, NIMK_MODEL_GRU] {
                let mut out = NimkOut::pass_through();
                assert!(nimk_set_model(core, kind, 1.0, &mut out));
                assert!(nimk_take_diagnostics(core).is_null());
            }
            nimk_free(core);
        }
    }

    #[test]
    fn config_idle_and_navigation_calls() {
        // SAFETY: null cores are allowed.
        unsafe {
            assert_eq!(nimk_set_config(ptr::null_mut(), c"x = 1".as_ptr()), 0);
            assert_eq!(nimk_idle_commit_secs(ptr::null()), 0.0);
            nimk_note_navigation(ptr::null_mut());
        }
        let dir = tempfile::tempdir().unwrap();
        let core = new_core(&dir);
        // SAFETY: live core and valid strings.
        unsafe {
            assert!((nimk_idle_commit_secs(core) - 1.5).abs() < 1e-9);
            let rejected =
                nimk_set_config(core, c"idle_commit_secs = 3.0\nswitch_margin = 9".as_ptr());
            assert_eq!(rejected, 1);
            assert!((nimk_idle_commit_secs(core) - 3.0).abs() < 1e-9);
            let diag = CStr::from_ptr(nimk_take_diagnostics(core))
                .to_str()
                .unwrap()
                .to_owned();
            assert!(diag.contains("switch_margin"), "{diag}");
            assert_eq!(nimk_set_config(core, ptr::null()), 0);
            assert!((nimk_idle_commit_secs(core) - 1.5).abs() < 1e-9);
            nimk_note_navigation(core);
            assert!(nimk_shutdown(core));
        }
    }
}
