# GUI tối thiểu

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `macos/NeuralIMK/{Overlay,InputController}.swift`, `crates/nimk-engine/src/engine.rs`

PoC chỉ có ba bề mặt UI, đều viết trong Swift shim vì chúng là AppKit thuần và ngắn.
Rust chỉ báo khi nào hiện và vì sao (`EngineOutput::overlay`, `EngineOutput::mode`); Swift lo vẽ.

## Bề mặt

| Bề mặt | Cách làm | Kích hoạt |
| --- | --- | --- |
| Overlay tại con trỏ | `NSPanel` borderless, `.nonactivatingPanel`, level `.statusBar`, `ignoresMouseEvents = true`; đặt ngay dưới rect con trỏ; hiện "VI"/"EN" khoảng 350 ms rồi mờ dần trong 600 ms | `overlay = Some(_)`, tức là khi sticky mode đổi |
| Status item menu bar | `NSStatusItem` hiển thị "VI"/"EN", thêm dấu chấm khi mode do model chọn (`Overlay::Auto`) | Mỗi khi `mode` đổi |
| Menu IME | `IMKInputController.menu()` trong input menu hệ thống | Người dùng mở menu |

## Overlay

Overlay phải không bao giờ lấy focus hay chặn chuột, vì nó hiện giữa lúc người dùng đang gõ.

```rust
pub enum Overlay {
    Auto,
    Manual,
}
```

| `Overlay` | Nghĩa | Hiển thị |
| --- | --- | --- |
| `Auto` | Model đổi sticky mode (lúc commit hoặc focus) | Xanh dương |
| `Manual` | Người dùng nháy Shift hai lần hoặc Ctrl+Space | Cam |
| `Manual` + sửa ngược bị bỏ qua | `replace` không áp được ([09](09-app-compat.md)) | Đỏ, chữ mode gạch ngang, kèm "không sửa được ở app này" |

`NimkOut::overlay` mang cả hai giá trị (`uint8_t`: 0 none, 1 auto, 2 manual).

Vị trí overlay dùng lại rect con trỏ Cocoa đã đọc cho context ([03](03-context-io.md)), không cần đổi sang Quartz vì `NSPanel` cũng dùng toạ độ Cocoa.

## Status item

Thử tạo `NSStatusItem` ngay trong process IME trước; nếu không hiện được thì tách agent app.

| Phương án | Điều kiện | Cách truyền mode |
| --- | --- | --- |
| A: trong process IME | IME `LSBackgroundOnly` vẫn hiện được status item | Gọi trực tiếp |
| B: agent app riêng | Phương án A thất bại | Agent `LSUIElement` nhận mode qua `DistributedNotificationCenter` |

## Menu IME

Menu dựng trong `InputController.menu()`; mục thông tin không bấm được.

| Mục | Nguồn |
| --- | --- |
| `Mode: Tiếng Việt (VI)` / `English (EN)` | `nimk_mode` |
| `Policy: <id>`, ví dụ `gru_action@v1` | `nimk_policy_id` |
| `Last replace issue: ...` (chỉ khi có) | Chẩn đoán sửa ngược gần nhất |
| `Retro-correction in this app: on/off` | `ReplaceCompat.allows(bundle)`; bấm để đảo override ([09](09-app-compat.md)) |
| `Open data folder` | Mở `~/Library/Application Support/NeuralIMK/` trong Finder |
| `Show diagnostics log` | Mở `~/Library/Logs/NeuralIMK.log` |

Mục "Reset model" trong thiết kế cũ chưa có; muốn reset thì xoá `blobs/model/<kind>/latest` trong thư mục dữ liệu ([07](07-build-and-install.md)).

## Dashboard (sau M4)

Dashboard metric dùng `egui` đọc `report.json` của `nimk-replay`; trước đó đọc metric từ output CLI ([05](05-storage-and-log.md)).
