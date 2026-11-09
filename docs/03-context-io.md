# Context I/O

> **Status:** draft · **Updated:** 2026-11-09 · **Code:** `crates/nimk-types/src/ctx.rs`, `macos/NeuralIMK/{InputController,Context}.swift`

The context is only the app's `bundle_id` and the caret position relative to the app window.
Both come without Accessibility or Screen Recording permission. `field_id` is used only to detect a change of input field.

## Types

The shim sends an `AppContext` on activation and at the first key of each word. `caret = None` means unknown.

```rust
pub struct CaretRel {
    pub x: f32,
    pub y: f32,
}

impl CaretRel {
    pub fn new(x: f32, y: f32) -> Option<Self>;
    pub fn cell(self, n: u32) -> u32;
}

pub struct AppContext {
    pub bundle_id: String,
    pub field_id: u64,
    pub caret: Option<CaretRel>,
    pub timestamp: f64,
}
```

| Property | Value |
| --- | --- |
| Origin | Top-left of the app window; each axis in `[0, 1]` |
| Unknown | The shim sends `-1`; `CaretRel::new` returns `None` for an axis outside `[0, 1]` or not finite |
| Clock | `timestamp` shares the clock of `KeyEvent::timestamp` (seconds since boot) |

## Sources

Each field comes from a public IMK or CoreGraphics API.

| Field | Source | Updated |
| --- | --- | --- |
| `bundle_id` | `client.bundleIdentifier()` (`IMKTextInput`) | `activateServer`; rechecked at the first key of each word |
| `field_id` | Hash of the IMK client object identity (opaque) | Changes when focus moves to another input field |
| Caret rect (screen, Cocoa) | `client.attributes(forCharacterIndex: 0, lineHeightRectangle: &rect)` | First key of each word |
| Window rects (screen, Quartz) | `CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)` | With the caret rect; cached 1 s per PID |

Window selection:

1. Keep windows whose `kCGWindowOwnerPID` is the client app's PID (the frontmost app if its bundle id matches).
2. Keep `kCGWindowLayer == 0` (normal windows, not menus or floating panels).
3. Read `kCGWindowBounds` and pick the window containing the caret rect's center, after converting to Quartz.
4. No window contains it: `caret = None`.

## Cocoa to Quartz

IMK returns the caret rect in Cocoa coordinates (origin bottom-left of the main screen). `CGWindowList` uses Quartz (origin top-left). Flip y before dividing by the window rect.

```text
 Cocoa (IMK caret rect)                      Quartz (CGWindowList bounds)
 origin = bottom-left of main screen         origin = top-left of main screen

 ^ y                                         +--------------------------> x
 |                                           |   (0,0)
 |   +------------------+                    |   +------------------+
 |   |  caret  [#]      |  y_c + h           |   |  caret  [#]      |  y_q
 |   |                  |                    |   |                  |
 |   +------------------+                    |   +------------------+
 |                                           |
 +--------------------------> x              v y
 (0,0)

            y_q = H_main - y_c - h        (H_main = NSScreen.screens[0].frame.height)
            x_q = x_c
```

Only y changes. `H_main` is always the main screen's height (`screens[0]`), even if the caret is on another display.

After the flip, using the caret rect's top-left corner:

```text
 x = clamp((caret.x_q - win.x) / win.width,  0, 1)
 y = clamp((caret.y_q - win.y) / win.height, 0, 1)
```

An empty caret rect or no matching window makes the shim send `(-1, -1)`.

## Caret grid

`nimk_policy::encode` bins `caret` into an 8x8 grid: 65 values, where 64 means unknown. This gives the model an "input area" feature. The log keeps the raw floats.

```text
        x:  0    1    2    3    4    5    6    7
          +----+----+----+----+----+----+----+----+
   y: 0   |  0 |  1 |  2 |  3 |  4 |  5 |  6 |  7 |   <- address bar, tab strip
          +----+----+----+----+----+----+----+----+
      1   |  8 |  9 | 10 | 11 | 12 | 13 | 14 | 15 |
          +----+----+----+----+----+----+----+----+
     ...  |                 ...                   |   <- editor body
          +----+----+----+----+----+----+----+----+
      7   | 56 | 57 | 58 | 59 | 60 | 61 | 62 | 63 |   <- chat input at bottom
          +----+----+----+----+----+----+----+----+
```

`cell = qx + n * qy` with `q(v) = min(floor(v * n), n - 1)`, so `x = 1.0` lands in column 7.

| Case | Feature |
| --- | --- |
| `caret = Some(c)` | `c.cell(8)`, in `0..=63` |
| `caret = None` | Its own "unknown" bucket (64) |

## Known risks

| Risk | Handling |
| --- | --- |
| Some terminals and Electron apps return a zero or wrong caret rect | Accept `None` (unknown) |
| A second display above or left of the main one (negative coordinates) | Convert with `screens[0]`; not yet tested on hardware |
| Overlapping windows of one PID | Pick the window containing the caret; the 1 s cache can lag while a window is dragged |
