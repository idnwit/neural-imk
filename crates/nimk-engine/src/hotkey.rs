//! Switch hotkey state machine: a lone Left-Shift tap or Ctrl+Space.
//!
//! Pure and clock-free: the only time source is [`KeyEvent::timestamp`].
//! Independent of the other engine modules; `engine.rs` decides what a hit
//! does and whether the key is consumed.

use nimk_types::{KeyEvent, KeyKind, Modifiers, keycode};

/// A recognised switch hotkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HotkeyHit {
    /// Left Shift pressed and released alone within the tap window.
    ShiftTap,
    /// Ctrl+Space key down.
    CtrlSpace,
}

/// Shift-tap tracking state.
#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    /// Nothing in progress.
    Idle,
    /// Left Shift went down alone at `t0`.
    ShiftDown {
        /// Timestamp of the press.
        t0: f64,
    },
    /// Left Shift is (probably) held but something else happened; the tap
    /// can no longer complete.
    Cancelled,
}

/// Detects the switch hotkey from the raw event stream.
#[derive(Debug, Clone)]
pub(crate) struct HotkeyDetector {
    tap_secs: f64,
    shift_tap: bool,
    ctrl_space: bool,
    state: State,
}

/// Modifiers other than Shift; any of them held makes a Shift press a chord.
const NON_SHIFT: u32 = Modifiers::CONTROL | Modifiers::OPTION | Modifiers::COMMAND;

impl HotkeyDetector {
    /// Creates a detector. `tap_secs` = max Shift hold time; `shift_tap`
    /// enables the lone Left-Shift tap; `ctrl_space` enables Ctrl+Space.
    #[must_use]
    pub(crate) fn new(tap_secs: f64, shift_tap: bool, ctrl_space: bool) -> Self {
        Self {
            tap_secs,
            shift_tap,
            ctrl_space,
            state: State::Idle,
        }
    }

    /// Feeds one event; returns a hit when the hotkey completes on it.
    /// Any key down, and any other modifier change, between Shift down and
    /// up cancels the tap.
    pub(crate) fn feed(&mut self, key: &KeyEvent) -> Option<HotkeyHit> {
        match key.kind {
            KeyKind::KeyDown => self.key_down(key),
            KeyKind::FlagsChanged => self.flags_changed(key),
        }
    }

    fn key_down(&mut self, key: &KeyEvent) -> Option<HotkeyHit> {
        if self.state != State::Idle {
            self.state = State::Cancelled;
        }
        let ctrl_only = key.mods.has(Modifiers::CONTROL)
            && key.mods.0 & (Modifiers::OPTION | Modifiers::COMMAND) == 0;
        (self.ctrl_space && key.keycode == keycode::SPACE && ctrl_only)
            .then_some(HotkeyHit::CtrlSpace)
    }

    fn flags_changed(&mut self, key: &KeyEvent) -> Option<HotkeyHit> {
        if !self.shift_tap {
            return None;
        }
        let shift_held = key.mods.has(Modifiers::SHIFT);
        if key.keycode != keycode::LEFT_SHIFT {
            // Another modifier changed: cancels a pending tap; once every
            // Shift is up we are idle again (recovers from lost events).
            self.state = match self.state {
                _ if !shift_held => State::Idle,
                State::Idle => State::Idle,
                State::ShiftDown { .. } | State::Cancelled => State::Cancelled,
            };
            return None;
        }
        match (self.state, shift_held) {
            (State::Idle, true) => {
                self.state = if key.mods.0 & NON_SHIFT == 0 {
                    State::ShiftDown { t0: key.timestamp }
                } else {
                    State::Cancelled
                };
                None
            }
            (State::ShiftDown { t0 }, false) => {
                self.state = State::Idle;
                let dt = key.timestamp - t0;
                (dt >= 0.0 && dt <= self.tap_secs).then_some(HotkeyHit::ShiftTap)
            }
            // Left Shift event while Shift stays set (e.g. Right Shift held):
            // ambiguous, never a lone tap.
            (State::ShiftDown { .. } | State::Cancelled, true) => {
                self.state = State::Cancelled;
                None
            }
            (_, false) => {
                self.state = State::Idle;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u32 = Modifiers::SHIFT;
    const C: u32 = Modifiers::CONTROL;

    fn flags(code: u16, mods: u32, t: f64) -> KeyEvent {
        KeyEvent {
            kind: KeyKind::FlagsChanged,
            keycode: code,
            chars: String::new(),
            mods: Modifiers(mods),
            timestamp: t,
        }
    }

    fn down(code: u16, chars: &str, mods: u32, t: f64) -> KeyEvent {
        KeyEvent {
            kind: KeyKind::KeyDown,
            keycode: code,
            chars: chars.to_owned(),
            mods: Modifiers(mods),
            timestamp: t,
        }
    }

    fn run(det: &mut HotkeyDetector, events: &[KeyEvent]) -> Vec<Option<HotkeyHit>> {
        events.iter().map(|e| det.feed(e)).collect()
    }

    #[test]
    fn table() {
        let ls = keycode::LEFT_SHIFT;
        let rs = keycode::RIGHT_SHIFT;
        let tap = Some(HotkeyHit::ShiftTap);
        type Case = (&'static str, Vec<KeyEvent>, Vec<Option<HotkeyHit>>);
        let cases: Vec<Case> = vec![
            (
                "quick tap",
                vec![flags(ls, S, 1.0), flags(ls, 0, 1.1)],
                vec![None, tap],
            ),
            (
                "exactly at limit",
                vec![flags(ls, S, 1.0), flags(ls, 0, 1.25)],
                vec![None, tap],
            ),
            (
                "held too long",
                vec![flags(ls, S, 1.0), flags(ls, 0, 1.5)],
                vec![None, None],
            ),
            (
                "time goes back",
                vec![flags(ls, S, 1.0), flags(ls, 0, 0.5)],
                vec![None, None],
            ),
            (
                "shifted letter cancels",
                vec![flags(ls, S, 1.0), down(0, "A", S, 1.05), flags(ls, 0, 1.1)],
                vec![None, None, None],
            ),
            (
                "other modifier cancels",
                vec![
                    flags(ls, S, 1.0),
                    flags(55, S | Modifiers::COMMAND, 1.02),
                    flags(55, S, 1.04),
                    flags(ls, 0, 1.1),
                ],
                vec![None, None, None, None],
            ),
            (
                "shift with cmd held is a chord",
                vec![
                    flags(ls, S | Modifiers::COMMAND, 1.0),
                    flags(ls, Modifiers::COMMAND, 1.1),
                ],
                vec![None, None],
            ),
            (
                "right shift is not the hotkey",
                vec![flags(rs, S, 1.0), flags(rs, 0, 1.1)],
                vec![None, None],
            ),
            (
                "left released while right held",
                vec![
                    flags(ls, S, 1.0),
                    flags(rs, S, 1.05),
                    flags(ls, S, 1.1),
                    flags(rs, 0, 1.2),
                ],
                vec![None, None, None, None],
            ),
            (
                "recovers after cancel",
                vec![
                    flags(ls, S, 1.0),
                    down(0, "A", S, 1.05),
                    flags(ls, 0, 1.1),
                    flags(ls, S, 2.0),
                    flags(ls, 0, 2.1),
                ],
                vec![None, None, None, None, tap],
            ),
            (
                "ctrl space",
                vec![down(keycode::SPACE, " ", C, 1.0)],
                vec![Some(HotkeyHit::CtrlSpace)],
            ),
            (
                "cmd ctrl space is not",
                vec![down(keycode::SPACE, " ", C | Modifiers::COMMAND, 1.0)],
                vec![None],
            ),
            (
                "plain space",
                vec![down(keycode::SPACE, " ", 0, 1.0)],
                vec![None],
            ),
        ];
        for (name, events, want) in cases {
            let mut det = HotkeyDetector::new(0.25, true, true);
            assert_eq!(run(&mut det, &events), want, "case {name}");
        }
    }

    #[test]
    fn toggles_are_independent() {
        let mut det = HotkeyDetector::new(0.3, false, true);
        let evs = [
            flags(keycode::LEFT_SHIFT, S, 1.0),
            flags(keycode::LEFT_SHIFT, 0, 1.1),
        ];
        assert_eq!(run(&mut det, &evs), vec![None, None]);
        assert_eq!(
            det.feed(&down(keycode::SPACE, " ", C, 2.0)),
            Some(HotkeyHit::CtrlSpace)
        );

        let mut det = HotkeyDetector::new(0.3, true, false);
        assert_eq!(det.feed(&down(keycode::SPACE, " ", C, 2.0)), None);
        assert_eq!(run(&mut det, &evs), vec![None, Some(HotkeyHit::ShiftTap)]);
    }
}
