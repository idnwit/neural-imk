use serde::{Deserialize, Serialize};

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
