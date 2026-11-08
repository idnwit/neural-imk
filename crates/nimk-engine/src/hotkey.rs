//! Switch hotkey state machine: double Shift tap (ADR 0008) or Ctrl+Space.
//!
//! Pure and clock-free: the only time source is [`KeyEvent::timestamp`].
//! Independent of the other engine modules; `engine.rs` decides what a hit
//! does and whether the key is consumed.
//!
//! A tap is a press and release of either Shift key with nothing else held
//! or typed, shorter than `tap_secs`. Two taps whose releases are at most
//! `double_secs` apart fire the hotkey on the second release. Any other key
//! or modifier between the first press and the second release cancels. A
//! second tap that is too slow starts a new pair instead.

use nimk_types::{KeyEvent, KeyKind, Modifiers, keycode};

/// A recognised switch hotkey.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HotkeyHit {
    /// Second lone Shift tap completed in time.
    DoubleShift,
    /// Ctrl+Space key down.
    CtrlSpace,
}

/// Detector tuning (from `EngineConfig`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HotkeyParams {
    /// Longest Shift hold that still counts as a tap.
    pub(crate) tap_secs: f64,
    /// Longest gap between the releases of the two taps.
    pub(crate) double_secs: f64,
    /// Enable the double Shift tap.
    pub(crate) double_shift: bool,
    /// Enable Ctrl+Space.
    pub(crate) ctrl_space: bool,
}

/// Tap tracking state.
#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    /// Nothing in progress.
    Idle,
    /// First Shift press, alone, at `t0`.
    Down1 { t0: f64 },
    /// First tap done; it was released at `t1`.
    Between { t1: f64 },
    /// Second Shift press at `t0`; first tap released at `t1`.
    Down2 { t0: f64, t1: f64 },
    /// Shift is held but the tap can no longer complete.
    Cancelled,
}

/// Detects the switch hotkey from the raw event stream.
#[derive(Debug, Clone)]
pub(crate) struct HotkeyDetector {
    params: HotkeyParams,
    state: State,
}

/// Modifiers other than Shift; any of them held makes a Shift press a chord.
const NON_SHIFT: u32 = Modifiers::CONTROL | Modifiers::OPTION | Modifiers::COMMAND;

impl HotkeyDetector {
    /// Creates a detector.
    #[must_use]
    pub(crate) fn new(params: HotkeyParams) -> Self {
        Self {
            params,
            state: State::Idle,
        }
    }

    /// Feeds one event; returns a hit when the hotkey completes on it.
    pub(crate) fn feed(&mut self, key: &KeyEvent) -> Option<HotkeyHit> {
        match key.kind {
            KeyKind::KeyDown => self.key_down(key),
            KeyKind::FlagsChanged => self.flags_changed(key),
        }
    }

    fn key_down(&mut self, key: &KeyEvent) -> Option<HotkeyHit> {
        self.state = match self.state {
            State::Idle | State::Between { .. } => State::Idle,
            State::Down1 { .. } | State::Down2 { .. } | State::Cancelled => State::Cancelled,
        };
        let ctrl_only = key.mods.has(Modifiers::CONTROL)
            && key.mods.0 & (Modifiers::OPTION | Modifiers::COMMAND) == 0;
        (self.params.ctrl_space && key.keycode == keycode::SPACE && ctrl_only)
            .then_some(HotkeyHit::CtrlSpace)
    }

    /// Whether `[t0, t1]` is a tap-length press.
    fn is_tap(&self, t0: f64, t1: f64) -> bool {
        let dt = t1 - t0;
        (0.0..=self.params.tap_secs).contains(&dt)
    }

    fn flags_changed(&mut self, key: &KeyEvent) -> Option<HotkeyHit> {
        if !self.params.double_shift {
            return None;
        }
        let shift_held = key.mods.has(Modifiers::SHIFT);
        let t = key.timestamp;
        if key.keycode != keycode::LEFT_SHIFT && key.keycode != keycode::RIGHT_SHIFT {
            // Another modifier changed: cancels a pending tap or pair; once
            // every Shift is up we are idle again (recovers from lost events).
            self.state = match self.state {
                _ if !shift_held => State::Idle,
                State::Idle | State::Between { .. } => State::Idle,
                State::Down1 { .. } | State::Down2 { .. } | State::Cancelled => State::Cancelled,
            };
            return None;
        }
        let alone = key.mods.0 & NON_SHIFT == 0;
        match (self.state, shift_held) {
            (State::Idle, true) => {
                self.state = if alone {
                    State::Down1 { t0: t }
                } else {
                    State::Cancelled
                };
                None
            }
            (State::Between { t1 }, true) => {
                self.state = if alone {
                    State::Down2 { t0: t, t1 }
                } else {
                    State::Cancelled
                };
                None
            }
            // Another Shift went down while one is held: never a lone tap.
            (State::Down1 { .. } | State::Down2 { .. } | State::Cancelled, true) => {
                self.state = State::Cancelled;
                None
            }
            (State::Down1 { t0 }, false) => {
                self.state = if self.is_tap(t0, t) {
                    State::Between { t1: t }
                } else {
                    State::Idle
                };
                None
            }
            (State::Down2 { t0, t1 }, false) => {
                if !self.is_tap(t0, t) {
                    self.state = State::Idle;
                    return None;
                }
                if (0.0..=self.params.double_secs).contains(&(t - t1)) {
                    self.state = State::Idle;
                    Some(HotkeyHit::DoubleShift)
                } else {
                    // Too slow to pair with the first tap: it is a first tap.
                    self.state = State::Between { t1: t };
                    None
                }
            }
            (State::Idle | State::Between { .. } | State::Cancelled, false) => {
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
    const LS: u16 = keycode::LEFT_SHIFT;
    const RS: u16 = keycode::RIGHT_SHIFT;

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

    fn detector() -> HotkeyDetector {
        HotkeyDetector::new(HotkeyParams {
            tap_secs: 0.25,
            double_secs: 0.4,
            double_shift: true,
            ctrl_space: true,
        })
    }

    /// Indices of the events on which the hotkey fired.
    fn hits(det: &mut HotkeyDetector, events: &[KeyEvent]) -> Vec<(usize, HotkeyHit)> {
        events
            .iter()
            .enumerate()
            .filter_map(|(i, e)| det.feed(e).map(|h| (i, h)))
            .collect()
    }

    #[test]
    fn table() {
        let dbl = HotkeyHit::DoubleShift;
        type Case = (&'static str, Vec<KeyEvent>, Vec<(usize, HotkeyHit)>);
        let cases: Vec<Case> = vec![
            (
                "single tap",
                vec![flags(LS, S, 1.0), flags(LS, 0, 1.1)],
                vec![],
            ),
            (
                "double tap",
                vec![
                    flags(LS, S, 1.0),
                    flags(LS, 0, 1.1),
                    flags(LS, S, 1.2),
                    flags(LS, 0, 1.3),
                ],
                vec![(3, dbl)],
            ),
            (
                "left then right",
                vec![
                    flags(LS, S, 1.0),
                    flags(LS, 0, 1.1),
                    flags(RS, S, 1.2),
                    flags(RS, 0, 1.3),
                ],
                vec![(3, dbl)],
            ),
            (
                "right twice",
                vec![
                    flags(RS, S, 1.0),
                    flags(RS, 0, 1.1),
                    flags(RS, S, 1.2),
                    flags(RS, 0, 1.3),
                ],
                vec![(3, dbl)],
            ),
            (
                "second release exactly at limit",
                vec![
                    flags(LS, S, 1.0),
                    flags(LS, 0, 1.1),
                    flags(LS, S, 1.3),
                    flags(LS, 0, 1.5),
                ],
                vec![(3, dbl)],
            ),
            (
                "second tap too slow",
                vec![
                    flags(LS, S, 1.0),
                    flags(LS, 0, 1.1),
                    flags(LS, S, 1.6),
                    flags(LS, 0, 1.7),
                ],
                vec![],
            ),
            (
                "too slow second tap pairs with a third",
                vec![
                    flags(LS, S, 1.0),
                    flags(LS, 0, 1.1),
                    flags(LS, S, 1.6),
                    flags(LS, 0, 1.7),
                    flags(LS, S, 1.8),
                    flags(LS, 0, 1.9),
                ],
                vec![(5, dbl)],
            ),
            (
                "first tap held too long",
                vec![
                    flags(LS, S, 1.0),
                    flags(LS, 0, 1.5),
                    flags(LS, S, 1.6),
                    flags(LS, 0, 1.7),
                ],
                vec![],
            ),
            (
                "second tap held too long",
                vec![
                    flags(LS, S, 1.0),
                    flags(LS, 0, 1.1),
                    flags(LS, S, 1.2),
                    flags(LS, 0, 1.6),
                ],
                vec![],
            ),
            (
                "key between taps cancels",
                vec![
                    flags(LS, S, 1.0),
                    flags(LS, 0, 1.1),
                    down(0, "a", 0, 1.15),
                    flags(LS, S, 1.2),
                    flags(LS, 0, 1.3),
                ],
                vec![],
            ),
            (
                "shifted letter during first tap cancels",
                vec![
                    flags(LS, S, 1.0),
                    down(0, "A", S, 1.05),
                    flags(LS, 0, 1.1),
                    flags(LS, S, 1.2),
                    flags(LS, 0, 1.3),
                ],
                vec![],
            ),
            (
                "key during second tap cancels",
                vec![
                    flags(LS, S, 1.0),
                    flags(LS, 0, 1.1),
                    flags(LS, S, 1.2),
                    down(0, "A", S, 1.25),
                    flags(LS, 0, 1.3),
                ],
                vec![],
            ),
            (
                "other modifier between taps cancels",
                vec![
                    flags(LS, S, 1.0),
                    flags(LS, 0, 1.1),
                    flags(55, Modifiers::COMMAND, 1.15),
                    flags(55, 0, 1.17),
                    flags(LS, S, 1.2),
                    flags(LS, 0, 1.3),
                ],
                vec![],
            ),
            (
                "shift with cmd held is a chord",
                vec![
                    flags(LS, S | Modifiers::COMMAND, 1.0),
                    flags(LS, Modifiers::COMMAND, 1.1),
                    flags(LS, S | Modifiers::COMMAND, 1.2),
                    flags(LS, Modifiers::COMMAND, 1.3),
                ],
                vec![],
            ),
            (
                "both shifts held is not a tap",
                vec![
                    flags(LS, S, 1.0),
                    flags(RS, S, 1.05),
                    flags(LS, S, 1.1),
                    flags(RS, 0, 1.2),
                    flags(LS, S, 1.25),
                    flags(LS, 0, 1.3),
                ],
                vec![],
            ),
            (
                "triple tap fires once then restarts",
                vec![
                    flags(LS, S, 1.0),
                    flags(LS, 0, 1.1),
                    flags(LS, S, 1.2),
                    flags(LS, 0, 1.3),
                    flags(LS, S, 1.4),
                    flags(LS, 0, 1.5),
                ],
                vec![(3, dbl)],
            ),
            (
                "time goes back",
                vec![flags(LS, S, 1.0), flags(LS, 0, 0.5)],
                vec![],
            ),
            (
                "ctrl space",
                vec![down(keycode::SPACE, " ", C, 1.0)],
                vec![(0, HotkeyHit::CtrlSpace)],
            ),
            (
                "cmd ctrl space is not",
                vec![down(keycode::SPACE, " ", C | Modifiers::COMMAND, 1.0)],
                vec![],
            ),
            (
                "plain space",
                vec![down(keycode::SPACE, " ", 0, 1.0)],
                vec![],
            ),
        ];
        for (name, events, want) in cases {
            assert_eq!(hits(&mut detector(), &events), want, "case {name}");
        }
    }

    #[test]
    fn toggles_are_independent() {
        let evs = [
            flags(LS, S, 1.0),
            flags(LS, 0, 1.1),
            flags(LS, S, 1.2),
            flags(LS, 0, 1.3),
        ];
        let mut det = HotkeyDetector::new(HotkeyParams {
            tap_secs: 0.3,
            double_secs: 0.4,
            double_shift: false,
            ctrl_space: true,
        });
        assert!(hits(&mut det, &evs).is_empty());
        assert_eq!(
            det.feed(&down(keycode::SPACE, " ", C, 2.0)),
            Some(HotkeyHit::CtrlSpace)
        );

        let mut det = HotkeyDetector::new(HotkeyParams {
            tap_secs: 0.3,
            double_secs: 0.4,
            double_shift: true,
            ctrl_space: false,
        });
        assert_eq!(det.feed(&down(keycode::SPACE, " ", C, 2.0)), None);
        assert_eq!(hits(&mut det, &evs), vec![(3, HotkeyHit::DoubleShift)]);
    }
}
