# Tổng quan NeuralIMK

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `Cargo.toml`, `crates/` · **Kiến trúc:** [01](01-architecture.md)

NeuralIMK là một IME duy nhất trên macOS tự chọn VI (Telex) hay EN cho từng từ.
Nó học online từ mỗi lần người dùng bấm hotkey chuyển, mục tiêu đưa số lần chuyển tay về gần 0.

## Mục tiêu

PoC phải chứng minh ba điều: auto-switch theo ngữ cảnh khả thi, học online đủ nhanh, và hot path đủ rẻ.

| # | Mục tiêu | Tài liệu |
| --- | --- | --- |
| 1 | Một input source (IMK), VI/EN là chế độ nội bộ, Telex là kiểu gõ duy nhất | [02](02-keyboard-io.md), [ADR 0001](adr/0001-single-imk-input-source.md) |
| 2 | Hotkey chuyển nhận ngay trong IME: gõ nhanh Shift trái, dự phòng Ctrl+Space | [02](02-keyboard-io.md), [ADR 0003](adr/0003-shift-tap-hotkey.md) |
| 3 | Context chỉ gồm `bundle_id` và vị trí con trỏ tương đối trong cửa sổ app | [03](03-context-io.md) |
| 4 | Policy học online, thay model qua trait; pretrain bằng dữ liệu synthetic | [04](04-engine-ml.md), [ADR 0005](adr/0005-online-supervised-not-exploratory-rl.md), [ADR 0006](adr/0006-synthetic-pretraining.md) |
| 5 | Ghi toàn bộ sự kiện để replay offline và so sánh model | [05](05-storage-and-log.md), [ADR 0004](adr/0004-storage-behind-trait.md) |
| 6 | Overlay VI/EN tại con trỏ và status item trên menu bar | [06](06-gui.md) |

## Ngoài phạm vi

Phase research cố ý bỏ qua các mục sau; mỗi mục có đường mở rộng đã tính trước.

| Bỏ qua | Lý do / đường mở rộng |
| --- | --- |
| Mã hoá log, blocklist app nhạy cảm | Log plaintext; mã hoá sau bằng decorator `EncryptedStorage<S: Storage>` |
| Ẩn gạch chân marked text, sửa từ đã commit | Chấp nhận gạch chân để không phải backspace giả ([ADR 0002](adr/0002-swift-shim-rust-core.md)) |
| VNI, gõ tắt, macro | Chỉ Telex |
| Codesign/notarize | Chỉ chạy máy dev, ad-hoc sign (`codesign -s -`) |
| Đa người dùng, đồng bộ model | Một máy, một model |

## Chỉ số thành công

PoC đạt khi cả ba chỉ số sau đúng sau 1 tuần dùng thật (M5).

| Chỉ số | Ngưỡng | Đo bằng |
| --- | --- | --- |
| Manual switch / 1000 từ | Giảm ≥ 50% so với baseline "nhớ mode cuối theo app" | `nimk-replay` + log thật |
| Latency `nimk_key` p99 | < 2 ms, gồm inference | Đo trong process thật |
| Độ ổn định | Không crash trong 1 ngày gõ liên tục | Dogfood |

## Lộ trình milestone

Milestone chạy tuần tự; mỗi milestone chỉ xong khi đạt tiêu chí và `make check` xanh.

| Milestone | Nội dung | Tiêu chí xong |
| --- | --- | --- |
| M0 | Workspace `crates/*` (spec gốc 7 crate, nay thêm `nimk-synth`), `arch-lint.toml`, `make check` | fmt + clippy + test + arch-lint chạy được |
| M1 | Rust core không UI: Telex, composer, hotkey, storage, baseline, log writer | Golden test xanh |
| M2 | Hello IMK: Swift shim + `libnimk_ffi.a` | Gõ "tiếng Việt" trong TextEdit, Notes, Chrome, VS Code; Shift tap lật từ đang gõ |
| M3 | Context & log thật, overlay, status item | Log có `bundle_id`, `caret` đúng khi đổi app và kéo cửa sổ |
| M4 | `Linear`, `TinyMlp`, cửa sổ tăng cường | `nimk-replay` vượt baseline trên log thật |
| M5 | Dogfood 1 tuần | Đạt chỉ số thành công; ghi kết luận vào [devlog](devlog.md) |

## Thuật ngữ

| Thuật ngữ | Nghĩa |
| --- | --- |
| `raw` | Chuỗi phím thô của từ đang gõ, ví dụ `tieengs` |
| marked text / preedit | Text đang soạn, hiển thị gạch chân, chưa commit vào app |
| prior | Quyết định mode lúc focus app, trước khi gõ từ nào |
| manual switch | Người dùng bấm hotkey; là nguồn nhãn chắc chắn |
| `p_vi` | Xác suất người dùng muốn VI do `Policy::predict` trả về |
| luật cứng | Quy tắc quyết định trước model (`DecisionRule`) |
