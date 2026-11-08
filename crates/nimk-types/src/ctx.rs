use serde::{Deserialize, Serialize};

use crate::{Features, Lang};

/// Caret position relative to the focused app window, each axis in `[0, 1]`,
/// origin at the window's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CaretRel {
    /// Horizontal position, 0 = left edge.
    pub x: f32,
    /// Vertical position, 0 = top edge.
    pub y: f32,
}

impl CaretRel {
    /// Builds a position, returning `None` if either axis is outside `[0, 1]`
    /// or not finite (the shim sends `-1` for "unknown").
    #[must_use]
    pub fn new(x: f32, y: f32) -> Option<Self> {
        let ok = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        (ok(x) && ok(y)).then_some(Self { x, y })
    }

    /// Cell index on an `n x n` grid, row-major from the top-left.
    #[must_use]
    pub fn cell(self, n: u32) -> u32 {
        let q = |v: f32| ((v * n as f32) as u32).min(n.saturating_sub(1));
        q(self.x) + n * q(self.y)
    }
}

/// App context reported by the shim on activation and at word starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppContext {
    /// Bundle identifier of the focused app, e.g. `com.apple.TextEdit`.
    pub bundle_id: String,
    /// Opaque id of the focused text input client (the shim hashes the IMK
    /// client object identity). Changes when focus moves to another field.
    pub field_id: u64,
    /// Caret position inside the app window, if known.
    pub caret: Option<CaretRel>,
    /// Seconds since boot, same clock as [`crate::KeyEvent::timestamp`].
    pub timestamp: f64,
}

/// Everything a feature extractor may look at for one decision.
#[derive(Debug, Clone, Copy)]
pub struct DecisionCtx<'a> {
    /// Bundle id of the focused app.
    pub bundle_id: &'a str,
    /// Caret position, if known.
    pub caret: Option<CaretRel>,
    /// Raw keys of the current word; empty for the prior made on focus.
    pub raw: &'a str,
    /// Whether `telex(raw)` is a valid Vietnamese syllable.
    pub valid_vi: bool,
    /// Whether `raw` contains any Telex transform key.
    pub has_telex: bool,
    /// Modes of up to 3 previous words in this field, most recent first.
    pub prev_langs: &'a [Lang],
    /// Mode currently in effect.
    pub current: Lang,
    /// Seconds since the last manual switch (`f32::INFINITY` if none).
    pub secs_since_switch: f32,
}

/// One labelled training example.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// Model input.
    pub features: Features,
    /// Language the user wanted.
    pub label: Lang,
    /// Importance weight: 1.0 for corrections, ~0.3 for implicit labels.
    pub weight: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_rejects_unknown() {
        assert!(CaretRel::new(-1.0, -1.0).is_none());
        assert!(CaretRel::new(f32::NAN, 0.5).is_none());
    }

    #[test]
    fn caret_cell_corners() {
        let tl = CaretRel::new(0.0, 0.0).map(|c| c.cell(8));
        let br = CaretRel::new(1.0, 1.0).map(|c| c.cell(8));
        assert_eq!(tl, Some(0));
        assert_eq!(br, Some(63));
    }
}
