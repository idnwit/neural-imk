use serde::{Deserialize, Serialize};

/// macOS virtual keycodes the engine cares about (`kVK_*` in Carbon `Events.h`).
pub mod keycode {
    /// Return / Enter.
    pub const RETURN: u16 = 36;
    /// Tab.
    pub const TAB: u16 = 48;
    /// Space bar.
    pub const SPACE: u16 = 49;
    /// Backspace (`kVK_Delete`).
    pub const BACKSPACE: u16 = 51;
    /// Escape.
    pub const ESCAPE: u16 = 53;
    /// Left Shift.
    pub const LEFT_SHIFT: u16 = 56;
    /// Right Shift.
    pub const RIGHT_SHIFT: u16 = 60;
    /// Left arrow.
    pub const LEFT: u16 = 123;
    /// Right arrow.
    pub const RIGHT: u16 = 124;
    /// Down arrow.
    pub const DOWN: u16 = 125;
    /// Up arrow.
    pub const UP: u16 = 126;
}

/// Modifier bit set, using the same bit positions as `NSEvent.ModifierFlags`
/// so the Swift shim can pass `event.modifierFlags.rawValue` through unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Modifiers(pub u32);

impl Modifiers {
    /// Shift is held.
    pub const SHIFT: u32 = 1 << 17;
    /// Control is held.
    pub const CONTROL: u32 = 1 << 18;
    /// Option is held.
    pub const OPTION: u32 = 1 << 19;
    /// Command is held.
    pub const COMMAND: u32 = 1 << 20;

    /// Returns true if every bit in `mask` is set.
    #[must_use]
    pub const fn has(self, mask: u32) -> bool {
        self.0 & mask == mask
    }

    /// Returns true if Command, Control or Option is held (an app shortcut).
    #[must_use]
    pub const fn is_shortcut(self) -> bool {
        self.0 & (Self::COMMAND | Self::CONTROL | Self::OPTION) != 0
    }
}

/// Kind of keyboard event delivered by IMK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    /// A key was pressed (`NSEvent.EventType.keyDown`).
    KeyDown,
    /// Modifier state changed (`NSEvent.EventType.flagsChanged`).
    FlagsChanged,
}

/// One keyboard event as seen by the IME.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyEvent {
    /// Event kind.
    pub kind: KeyKind,
    /// Virtual keycode.
    pub keycode: u16,
    /// Characters produced by the key, ignoring modifiers except Shift
    /// (`NSEvent.characters`). Empty for `FlagsChanged`.
    pub chars: String,
    /// Modifier flags after this event.
    pub mods: Modifiers,
    /// Seconds since boot (`NSEvent.timestamp`); the engine's only clock.
    pub timestamp: f64,
}
