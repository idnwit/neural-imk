# GUI tối thiểu

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `macos/NeuralIMK/Overlay.swift`, `crates/nimk-engine/src/engine.rs`

PoC chỉ có ba bề mặt UI, đều viết trong Swift shim vì chúng là AppKit thuần và ngắn.
Rust chỉ báo khi nào hiện và vì sao (`EngineOutput::overlay`, `EngineOutput::mode`); Swift lo vẽ.

## Bề mặt

| Bề mặt | Cách làm | Kích hoạt |
| --- | --- | --- |
| Overlay tại con trỏ | `NSPanel` borderless, `.nonactivatingPanel`, level `.statusBar`, `ignoresMouseEvents = true`; đặt ngay dưới rect con trỏ; hiện "VI"/"EN" rồi mờ dần trong 600 ms | `overlay = Some(_)` |
| Status item menu bar | `NSStatusItem` hiển thị "VI"/"EN", thêm dấu chấm khi mode do model chọn | Mỗi khi `mode` đổi |
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
| `Auto` | Policy tự đổi mode | Màu A (để dễ quan sát khi dogfood) |
| `Manual` | Người dùng bấm hotkey | Màu B |

`NimkOut` trong C ABI phải mang được hai giá trị này (ví dụ `uint8_t overlay`: 0 none, 1 auto, 2 manual); một cờ `bool` là không đủ.

Vị trí overlay dùng lại rect con trỏ Cocoa đã đọc cho context ([03](03-context-io.md)), không cần đổi sang Quartz vì `NSPanel` cũng dùng toạ độ Cocoa.

## Status item

Thử tạo `NSStatusItem` ngay trong process IME trước; nếu không hiện được thì tách agent app.

| Phương án | Điều kiện | Cách truyền mode |
| --- | --- | --- |
| A: trong process IME | IME `LSBackgroundOnly` vẫn hiện được status item | Gọi trực tiếp |
| B: agent app riêng | Phương án A thất bại | Agent `LSUIElement` nhận mode qua `DistributedNotificationCenter` |

## Menu IME

| Mục | Nguồn |
| --- | --- |
| Chế độ hiện tại | `Engine::mode` |
| Policy đang dùng | `Engine::policy_id`, ví dụ `tiny_mlp@v3` |
| Mở thư mục dữ liệu | Mở `~/Library/Application Support/NeuralIMK/` trong Finder |
| Reset model | Xoá snapshot, dựng lại `EngineConfig::policy` |

## Dashboard (sau M4)

Dashboard metric dùng `egui` đọc `report.json` của `nimk-replay`; trước đó đọc metric từ output CLI ([05](05-storage-and-log.md)).
