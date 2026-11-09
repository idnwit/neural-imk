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
| `Auto` | Model đổi sticky mode lúc kết thúc từ (prior khi focus chỉ đổi mode và status item, không overlay) | Xanh dương |
| `Manual` | Người dùng nháy Shift hai lần hoặc Ctrl+Space | Cam |

`NimkOut::overlay` mang cả hai giá trị (`uint8_t`: 0 none, 1 auto, 2 manual).

Overlay dò con trỏ ngay lúc hiện; rect rỗng thì dò lại một lần sau 80 ms, vẫn rỗng mới dùng vị trí chuột. Vị trí overlay dùng lại rect con trỏ Cocoa đã đọc cho context ([03](03-context-io.md)), không cần đổi sang Quartz vì `NSPanel` cũng dùng toạ độ Cocoa.

## Status item

Tooltip của status item hiện model đang dùng (`Model: GRU`).

Thử tạo `NSStatusItem` ngay trong process IME trước; nếu không hiện được thì tách agent app.

| Phương án | Điều kiện | Cách truyền mode |
| --- | --- | --- |
| A: trong process IME | IME `LSBackgroundOnly` vẫn hiện được status item | Gọi trực tiếp |
| B: agent app riêng | Phương án A thất bại | Agent `LSUIElement` nhận mode qua `DistributedNotificationCenter` |

## Menu IME

Menu dựng trong `InputController.menu()`; mục thông tin không bấm được. IMK gọi action với `sender` là dictionary chứa `NSMenuItem` dưới khoá `kIMKCommandMenuItemName`, nên mọi action khai báo `(_ sender: Any?)` và mỗi lần chọn đều ghi một dòng `menu: ...` vào log.

| Mục | Nguồn |
| --- | --- |
| `Mode: Tiếng Việt (VI)` / `English (EN)` | `nimk_mode` |
| `Policy: <id>`, ví dụ `gru_action@v1` | `nimk_policy_id` |
| `Model: Linear (nhanh) (gốc)`, `Model: GRU (chưa pre-train)` | Hai mục cùng cấp (không submenu: IMK không đảm bảo giao action của submenu); dấu `✓` ở model đang dùng (`nimk_model`); hậu tố theo `nimk_checkpoint_source`: `(gốc)` model đóng gói, `(đã học thêm)` checkpoint của người dùng, `(chưa pre-train)` khi không có. Chọn → `nimk_set_model`, áp `NimkOut` (commit từ đang gõ), cập nhật status item, hiện overlay tên model |
| `Model: GRU ✓ — sửa 1,2% (500 từ gần nhất) · 2,0% tổng (3 450 từ)` | One line per model (replaces the checkpoint suffix; `(chưa pre-train)` is still appended when none exists). Shadow models with online training on are tagged `(chạy ngầm)`, frozen ones `(đóng băng)`. Counters from `nimk_model_stats`, rebuilt every time the menu opens; text in `StatsText.swift` |
| `Học online: GRU ✓` | One toggle per model: `nimk_set_online_training`, persisted by the engine in `settings/models` |
| `Reload config` | `Core.reloadConfig()`: đọc lại `<Application Support>/<bundle id>/config.toml` (không có file cũng được), gọi `nimk_set_config`, hiện overlay `Đã nạp config` hoặc số giá trị bị bỏ ([11](11-config.md)) |
| `Khôi phục model gốc` | `nimk_reset_model`: xoá checkpoint người dùng của model đang dùng và nạp lại model gốc; áp `NimkOut`, hiện overlay |
| `Open data folder` | Mở data dir (`<Application Support>/<bundle id>`, do `FileManager` trả về) trong Finder |
| `Show diagnostics log` | Mở `<Library>/Logs/<bundle id>.log` (đường dẫn do `FileManager` trả về) |

Không còn phải xoá file thủ công ([ADR 0010](adr/0010-sdk-storage-and-bundled-models.md)).

## Dashboard (sau M4)

Dashboard metric dùng `egui` đọc `report.json` của `nimk-replay`; trước đó đọc metric từ output CLI ([05](05-storage-and-log.md)).
