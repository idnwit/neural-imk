# Context I/O

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `crates/nimk-types/src/ctx.rs`, `macos/NeuralIMK/InputController.swift`

Context chỉ gồm `bundle_id` của app và vị trí con trỏ tương đối trong cửa sổ app.
Cả hai lấy được mà không cần quyền Accessibility hay Screen Recording; `field_id` chỉ dùng để nhận biết đổi ô nhập.

## Kiểu dữ liệu

Shim gửi `AppContext` khi activate và ở phím đầu của mỗi từ; `caret = None` khi không biết.

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

| Thuộc tính | Giá trị |
| --- | --- |
| Gốc toạ độ | Góc trên–trái cửa sổ app, mỗi trục trong `[0, 1]` |
| Unknown | Shim gửi `-1`; `CaretRel::new` trả `None` khi trục ngoài `[0, 1]` hoặc không hữu hạn |
| Đồng hồ | `timestamp` cùng đồng hồ với `KeyEvent::timestamp` (giây từ lúc boot) |

## Nguồn dữ liệu

Mỗi trường lấy từ một API công khai của IMK hoặc CoreGraphics.

| Trường | Lấy từ | Khi nào cập nhật |
| --- | --- | --- |
| `bundle_id` | `client.bundleIdentifier()` (`IMKTextInput`) | `activateServer`, so lại ở phím đầu mỗi từ |
| `field_id` | Hash của identity object IMK client (opaque) | Đổi khi focus sang ô nhập khác |
| Rect con trỏ (screen, Cocoa) | `client.attributes(forCharacterIndex: 0, lineHeightRectangle: &rect)` | Phím đầu mỗi từ |
| Rect cửa sổ app (screen, Quartz) | `CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID)` | Cùng lúc với rect con trỏ; cache 1 s theo PID |

Chọn cửa sổ trong kết quả `CGWindowListCopyWindowInfo`:

1. Lọc `kCGWindowOwnerPID` = PID của `NSWorkspace.shared.frontmostApplication`.
2. Lọc `kCGWindowLayer == 0` (cửa sổ thường, bỏ menu và panel nổi).
3. Đọc `kCGWindowBounds`, chọn cửa sổ chứa tâm rect con trỏ (sau khi đổi sang Quartz).
4. Không có cửa sổ nào chứa → `caret = None`.

## Đổi toạ độ Cocoa → Quartz

IMK trả rect con trỏ theo Cocoa (gốc dưới–trái màn hình chính), còn `CGWindowList` dùng Quartz (gốc trên–trái).
Phải lật trục y trước khi chia cho rect cửa sổ.

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

Chỉ trục y đổi; `H_main` luôn là chiều cao màn hình chính (`screens[0]`), kể cả khi con trỏ ở màn hình phụ.

Sau khi đổi:

```text
 x = clamp((caret.x_q - win.x) / win.width,  0, 1)
 y = clamp((caret.y_q - win.y) / win.height, 0, 1)
```

Rect con trỏ bằng 0 hoặc không tìm thấy cửa sổ → shim gửi `(-1, -1)`.

Mẫu Swift (minh hoạ):

```swift
// minh hoạ
var line = NSRect.zero
_ = client.attributes(forCharacterIndex: 0, lineHeightRectangle: &line)
let mainH = NSScreen.screens[0].frame.height
let caretQ = CGRect(x: line.minX, y: mainH - line.minY - line.height,
                    width: line.width, height: line.height)
```

## Lưới ô con trỏ

Policy lưới hoá `caret` thành ô 8×8 để có feature rời rạc "khu vực nhập"; log vẫn lưu số thực gốc.

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

`cell = qx + n * qy`, với `q(v) = min(floor(v * n), n - 1)`; vì vậy `x = 1.0` rơi vào cột 7, không tràn sang 8.

| Trường hợp | Feature |
| --- | --- |
| `caret = Some(c)` | `c.cell(8)` ∈ `0..=63` |
| `caret = None` | bucket riêng "unknown" |

## Rủi ro đã biết

| Rủi ro | Xử lý trong PoC |
| --- | --- |
| Terminal cũ, vài app Electron trả rect con trỏ = 0 hoặc sai | Chấp nhận `None` ("unknown") |
| Màn hình phụ nằm trên/trái màn hình chính (toạ độ âm) | Dùng `screens[0]` để đổi; cần test M3 |
| Cửa sổ chồng nhau cùng PID | Chọn cửa sổ chứa con trỏ; cache 1 s có thể lệch khi kéo cửa sổ |
