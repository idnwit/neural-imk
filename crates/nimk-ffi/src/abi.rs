//! `#[repr(C)]` types shared with Swift and their conversions to engine types.
//!
//! Everything here is pure: no engine calls, so it is fully unit-testable.

use std::ffi::{CStr, CString, c_char};
use std::ptr;

use nimk_engine::{AppContext, EngineOutput, KeyEvent, Lang, ModelKind, Overlay};
use nimk_types::{CaretRel, KeyKind, Modifiers};

/// `NimkKey::kind`: a key was pressed.
pub const NIMK_KIND_KEY_DOWN: u8 = 0;
/// `NimkKey::kind`: modifier flags changed.
pub const NIMK_KIND_FLAGS_CHANGED: u8 = 1;

/// `NimkOut::mode` / `nimk_mode`: English (pass-through).
pub const NIMK_MODE_EN: u8 = 0;
/// `NimkOut::mode` / `nimk_mode`: Vietnamese (Telex).
pub const NIMK_MODE_VI: u8 = 1;
/// `NimkOut::mode`: mode did not change.
pub const NIMK_MODE_UNCHANGED: u8 = 255;

/// `NimkOut::overlay`: do not show the overlay.
pub const NIMK_OVERLAY_NONE: u8 = 0;
/// `NimkOut::overlay`: the policy changed the mode.
pub const NIMK_OVERLAY_AUTO: u8 = 1;
/// `NimkOut::overlay`: the user pressed the hotkey.
pub const NIMK_OVERLAY_MANUAL: u8 = 2;

/// `nimk_set_model` / `nimk_model` kind: the linear action model.
pub const NIMK_MODEL_LINEAR: u8 = 0;
/// `nimk_set_model` / `nimk_model` kind: the GRU action model.
pub const NIMK_MODEL_GRU: u8 = 1;

/// `nimk_checkpoint_source`: no checkpoint, the model starts untrained.
pub const NIMK_CHECKPOINT_NONE: u8 = 0;
/// `nimk_checkpoint_source`: the pretrained model shipped inside the app.
pub const NIMK_CHECKPOINT_BUNDLED: u8 = 1;
/// `nimk_checkpoint_source`: a checkpoint saved by this installation.
pub const NIMK_CHECKPOINT_USER: u8 = 2;

/// Maps a C model code to the engine's [`ModelKind`].
pub(crate) const fn model_from_code(code: u8) -> Option<ModelKind> {
    match code {
        NIMK_MODEL_LINEAR => Some(ModelKind::LinearAction),
        NIMK_MODEL_GRU => Some(ModelKind::GruAction),
        _ => None,
    }
}

/// Maps a [`ModelKind`] to its C model code.
pub(crate) const fn model_code(kind: ModelKind) -> u8 {
    match kind {
        ModelKind::LinearAction => NIMK_MODEL_LINEAR,
        ModelKind::GruAction => NIMK_MODEL_GRU,
    }
}

/// One keyboard event from `IMKInputController.handle(_:client:)`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NimkKey {
    /// `NIMK_KIND_KEY_DOWN` or `NIMK_KIND_FLAGS_CHANGED`.
    pub kind: u8,
    /// macOS virtual keycode (`NSEvent.keyCode`).
    pub keycode: u16,
    /// `NSEvent.modifierFlags.rawValue` truncated to 32 bits.
    pub modifiers: u32,
    /// `NSEvent.characters` as NUL-terminated UTF-8; may be null.
    pub chars: *const c_char,
    /// `NSEvent.timestamp` (seconds since boot).
    pub timestamp: f64,
}

/// App context on activation and at word starts.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NimkCtx {
    /// Bundle id of the focused app as UTF-8; may be null (unknown).
    pub bundle_id: *const c_char,
    /// Opaque id of the focused text input client.
    pub field_id: u64,
    /// Caret x relative to the app window in `[0, 1]`; `-1` = unknown.
    pub caret_rel_x: f32,
    /// Caret y relative to the app window in `[0, 1]`; `-1` = unknown.
    pub caret_rel_y: f32,
    /// Seconds since boot, same clock as `NimkKey::timestamp`.
    pub timestamp: f64,
}

/// Quality counters of one model, filled by `nimk_model_stats`. Counted per
/// committed word with a final label: for the active model corrected = the
/// user corrected the word; for a shadow model (online training on, not
/// serving) = its own fresh decision at the commit step would have been
/// corrected. `*_recent` cover the last 500 words. Keystroke counters measure
/// how often the model's display decision (with its own hysteresis) equalled
/// the final label; flips are display changes within a word.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NimkModelStats {
    /// Words counted, lifetime.
    pub words_total: u64,
    /// Corrected (or would-have-been corrected) words, lifetime.
    pub corrected_total: u64,
    /// Words in the rolling window (at most 500).
    pub words_recent: u32,
    /// Corrected words in the rolling window.
    pub corrected_recent: u32,
    /// Words (of `words_total`) typed while this model was the active one.
    pub served_words_total: u64,
    /// Lifetime corrected words by app category: terminal, ide, browser,
    /// editor, other.
    pub corrected_by_category: [u64; 5],
    /// Keystrokes counted for display accuracy, lifetime.
    pub keys_total: u64,
    /// Of those, keystrokes whose display equalled the final label.
    pub keys_match_total: u64,
    /// Display flips within words, lifetime.
    pub flips_total: u64,
    /// `keys_total` over the rolling window.
    pub keys_recent: u32,
    /// `keys_match_total` over the rolling window.
    pub keys_match_recent: u32,
    /// `flips_total` over the rolling window.
    pub flips_recent: u32,
}

impl From<nimk_engine::ModelStats> for NimkModelStats {
    fn from(s: nimk_engine::ModelStats) -> Self {
        Self {
            words_total: s.words_total,
            corrected_total: s.corrected_total,
            words_recent: s.words_recent,
            corrected_recent: s.corrected_recent,
            served_words_total: s.served_words_total,
            corrected_by_category: s.corrected_by_category,
            keys_total: s.keys_total,
            keys_match_total: s.keys_match_total,
            flips_total: s.flips_total,
            keys_recent: s.keys_recent,
            keys_match_recent: s.keys_match_recent,
            flips_recent: s.flips_recent,
        }
    }
}

/// What Swift must do after a call. Strings are owned by the core and stay
/// valid until the next call on the same core; copy them immediately.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct NimkOut {
    /// `false`: return the key to the app untouched.
    pub consumed: bool,
    /// Text to `insertText`, or null for none. Applied before `marked`.
    pub commit: *const c_char,
    /// New marked text; null = no change, `""` = clear.
    pub marked: *const c_char,
    /// `NIMK_MODE_EN`, `NIMK_MODE_VI` or `NIMK_MODE_UNCHANGED`.
    pub mode: u8,
    /// `NIMK_OVERLAY_NONE`, `NIMK_OVERLAY_AUTO` or `NIMK_OVERLAY_MANUAL`.
    pub overlay: u8,
}

impl NimkOut {
    /// "Pass the key through, change nothing": the safe answer on any error.
    #[must_use]
    pub const fn pass_through() -> Self {
        Self {
            consumed: false,
            commit: ptr::null(),
            marked: ptr::null(),
            mode: NIMK_MODE_UNCHANGED,
            overlay: NIMK_OVERLAY_NONE,
        }
    }
}

/// Maps a mode to its C code.
#[must_use]
pub(crate) const fn mode_code(lang: Lang) -> u8 {
    match lang {
        Lang::En => NIMK_MODE_EN,
        Lang::Vi => NIMK_MODE_VI,
    }
}

/// Maps an overlay request to its C code.
#[must_use]
pub(crate) const fn overlay_code(overlay: Option<Overlay>) -> u8 {
    match overlay {
        None => NIMK_OVERLAY_NONE,
        Some(Overlay::Auto) => NIMK_OVERLAY_AUTO,
        Some(Overlay::Manual) => NIMK_OVERLAY_MANUAL,
    }
}

/// Builds a C string, dropping interior NUL bytes instead of failing.
#[must_use]
pub(crate) fn to_cstring(s: String) -> CString {
    let mut bytes = s.into_bytes();
    bytes.retain(|&b| b != 0);
    // SAFETY: every NUL byte was removed just above, which is the only
    // invariant `from_vec_unchecked` requires.
    unsafe { CString::from_vec_unchecked(bytes) }
}

/// Reads a nullable C string into an owned `String` (`""` for null).
///
/// # Safety
/// Same contract as [`read_cstr`].
#[must_use]
pub(crate) unsafe fn read_owned(p: *const c_char) -> String {
    // SAFETY: forwarded from this function's contract.
    match unsafe { read_cstr(p) } {
        Some(s) => s.into_owned(),
        None => String::new(),
    }
}

/// Reads a nullable C string as UTF-8, replacing invalid sequences.
///
/// # Safety
/// `p` must be null or point to a NUL-terminated string that stays valid and
/// unmodified for the returned lifetime.
#[must_use]
pub(crate) unsafe fn read_cstr<'a>(p: *const c_char) -> Option<std::borrow::Cow<'a, str>> {
    if p.is_null() {
        return None;
    }
    // SAFETY: non-null and NUL-terminated per this function's contract.
    let s = unsafe { CStr::from_ptr(p) };
    Some(s.to_string_lossy())
}

/// Converts a C key into an engine key; `None` for an unknown `kind`.
///
/// # Safety
/// `key.chars` must satisfy [`read_cstr`]'s contract.
#[must_use]
pub(crate) unsafe fn key_from_c(key: &NimkKey) -> Option<KeyEvent> {
    let kind = match key.kind {
        NIMK_KIND_KEY_DOWN => KeyKind::KeyDown,
        NIMK_KIND_FLAGS_CHANGED => KeyKind::FlagsChanged,
        _ => return None,
    };
    let chars = match kind {
        KeyKind::FlagsChanged => String::new(),
        // SAFETY: forwarded from this function's contract.
        KeyKind::KeyDown => unsafe { read_owned(key.chars) },
    };
    Some(KeyEvent {
        kind,
        keycode: key.keycode,
        chars,
        mods: Modifiers(key.modifiers),
        timestamp: key.timestamp,
    })
}

/// Converts a C context into an engine context.
///
/// # Safety
/// `ctx.bundle_id` must satisfy [`read_cstr`]'s contract.
#[must_use]
pub(crate) unsafe fn ctx_from_c(ctx: &NimkCtx) -> AppContext {
    // SAFETY: forwarded from this function's contract.
    let bundle_id = unsafe { read_owned(ctx.bundle_id) };
    AppContext {
        bundle_id,
        field_id: ctx.field_id,
        caret: CaretRel::new(ctx.caret_rel_x, ctx.caret_rel_y),
        timestamp: ctx.timestamp,
    }
}

/// Owns the strings referenced by the last `NimkOut`.
#[derive(Debug, Default)]
pub(crate) struct OutBuffers {
    commit: Option<CString>,
    marked: Option<CString>,
}

impl OutBuffers {
    /// Stores `output`'s strings (replacing the previous ones) and returns
    /// the C view of it. Pointers stay valid until the next call.
    pub(crate) fn fill(&mut self, output: EngineOutput) -> NimkOut {
        self.commit = output.commit.map(to_cstring);
        self.marked = output.marked.map(to_cstring);
        NimkOut {
            consumed: output.consumed,
            commit: self.commit.as_deref().map_or(ptr::null(), CStr::as_ptr),
            marked: self.marked.as_deref().map_or(ptr::null(), CStr::as_ptr),
            mode: output.mode.map_or(NIMK_MODE_UNCHANGED, mode_code),
            overlay: overlay_code(output.overlay),
        }
    }

    /// Drops the previous strings (used before a pass-through answer).
    pub(crate) fn clear(&mut self) {
        self.commit = None;
        self.marked = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(p: *const c_char) -> Option<String> {
        // SAFETY: tests only pass null or pointers into live `CString`s.
        unsafe { read_cstr(p) }.map(|c| c.into_owned())
    }

    #[test]
    fn key_down_converts_chars_and_mods() {
        let chars = CString::new("a").unwrap();
        let key = NimkKey {
            kind: NIMK_KIND_KEY_DOWN,
            keycode: 0,
            modifiers: Modifiers::SHIFT,
            chars: chars.as_ptr(),
            timestamp: 1.5,
        };
        // SAFETY: `chars` outlives the call.
        let ev = unsafe { key_from_c(&key) }.unwrap();
        assert_eq!(ev.kind, KeyKind::KeyDown);
        assert_eq!(ev.chars, "a");
        assert!(ev.mods.has(Modifiers::SHIFT));
        assert_eq!(ev.timestamp, 1.5);
    }

    #[test]
    fn key_null_chars_is_empty() {
        let key = NimkKey {
            kind: NIMK_KIND_KEY_DOWN,
            keycode: 49,
            modifiers: 0,
            chars: ptr::null(),
            timestamp: 0.0,
        };
        // SAFETY: null is allowed.
        let ev = unsafe { key_from_c(&key) }.unwrap();
        assert_eq!(ev.chars, "");
    }

    #[test]
    fn flags_changed_ignores_chars() {
        let chars = CString::new("x").unwrap();
        let key = NimkKey {
            kind: NIMK_KIND_FLAGS_CHANGED,
            keycode: 56,
            modifiers: Modifiers::SHIFT,
            chars: chars.as_ptr(),
            timestamp: 0.0,
        };
        // SAFETY: `chars` outlives the call.
        let ev = unsafe { key_from_c(&key) }.unwrap();
        assert_eq!(ev.kind, KeyKind::FlagsChanged);
        assert_eq!(ev.chars, "");
    }

    #[test]
    fn unknown_kind_is_rejected() {
        let key = NimkKey {
            kind: 7,
            keycode: 0,
            modifiers: 0,
            chars: ptr::null(),
            timestamp: 0.0,
        };
        // SAFETY: null is allowed.
        assert!(unsafe { key_from_c(&key) }.is_none());
    }

    #[test]
    fn invalid_utf8_is_lossy() {
        let bad = CString::new(vec![0x61, 0xff]).unwrap();
        assert_eq!(read(bad.as_ptr()).as_deref(), Some("a\u{fffd}"));
        assert_eq!(read(ptr::null()), None);
    }

    #[test]
    fn ctx_unknown_caret_and_null_bundle() {
        let ctx = NimkCtx {
            bundle_id: ptr::null(),
            field_id: 9,
            caret_rel_x: -1.0,
            caret_rel_y: -1.0,
            timestamp: 2.0,
        };
        // SAFETY: null is allowed.
        let app = unsafe { ctx_from_c(&ctx) };
        assert_eq!(app.bundle_id, "");
        assert_eq!(app.field_id, 9);
        assert!(app.caret.is_none());
    }

    #[test]
    fn ctx_known_caret() {
        let id = CString::new("com.apple.TextEdit").unwrap();
        let ctx = NimkCtx {
            bundle_id: id.as_ptr(),
            field_id: 1,
            caret_rel_x: 0.25,
            caret_rel_y: 1.0,
            timestamp: 0.0,
        };
        // SAFETY: `id` outlives the call.
        let app = unsafe { ctx_from_c(&ctx) };
        assert_eq!(app.bundle_id, "com.apple.TextEdit");
        assert_eq!(app.caret, CaretRel::new(0.25, 1.0));
    }

    #[test]
    fn fill_maps_every_field() {
        let mut bufs = OutBuffers::default();
        let out = bufs.fill(EngineOutput {
            consumed: true,
            commit: Some("việt ".into()),
            marked: Some(String::new()),
            mode: Some(Lang::Vi),
            overlay: Some(Overlay::Manual),
        });
        assert!(out.consumed);
        assert_eq!(read(out.commit).as_deref(), Some("việt "));
        assert_eq!(read(out.marked).as_deref(), Some(""));
        assert_eq!(out.mode, NIMK_MODE_VI);
        assert_eq!(out.overlay, NIMK_OVERLAY_MANUAL);
    }

    #[test]
    fn fill_none_is_null_and_unchanged() {
        let mut bufs = OutBuffers::default();
        let out = bufs.fill(EngineOutput::default());
        assert!(!out.consumed);
        assert!(out.commit.is_null());
        assert!(out.marked.is_null());
        assert_eq!(out.mode, NIMK_MODE_UNCHANGED);
        assert_eq!(out.overlay, NIMK_OVERLAY_NONE);
        assert_eq!(overlay_code(Some(Overlay::Auto)), NIMK_OVERLAY_AUTO);
        assert_eq!(mode_code(Lang::En), NIMK_MODE_EN);
    }

    #[test]
    fn interior_nul_is_dropped() {
        assert_eq!(to_cstring("a\0b".into()).as_bytes(), b"ab");
    }
}
