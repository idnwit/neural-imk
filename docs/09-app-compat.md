# Tương thích app: marked text

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `macos/NeuralIMK/InputController.swift`, `crates/nimk-engine/src/engine/flow.rs`

Sửa từ bằng nháy Shift hai lần không còn cần `replacementRange`: từ model kết thúc ở lại marked text và được lật tại chỗ ([ADR 0011](adr/0011-pending-word-commit.md)).
Điều IME còn phụ thuộc vào app là `setMarkedText` và `insertText` thông thường, kể cả marked text kéo dài qua dấu cách và dấu câu.
Chưa có mục nào dưới đây được thử trên máy thật.

## Điều IME cần từ app

| Hành vi | Vì sao |
| --- | --- |
| `setMarkedText` hiển thị marked text liên tục tới đúng 1 ký tự ranh giới sau từ | Từ pending gồm từ cộng một ký tự không phải chữ (ADR 0012) |
| Phím Return, Tab, mũi tên tới app sau khi `insertText` commit | Engine commit pending rồi trả phím, nên gửi tin nhắn và di chuyển vẫn hoạt động |
| `commitComposition` được gọi khi click chuột hoặc mất focus | Chốt từ pending; nếu app không gọi, `deactivateServer` cũng commit |
| Hai client kích hoạt xen kẽ khi click (Electron) | `Debouncer` gộp việc gửi context trong 120 ms; phím tới trước thì gửi ngay |

## Rủi ro theo loại app

Dự kiến, cần test tay trước khi tin; không có cột nào đã đo.

| Loại | App | Rủi ro | Mức chắc chắn |
| --- | --- | --- | --- |
| Editor Electron, trình duyệt Chromium | VS Code, Cursor, Chrome | Gửi mỗi `flagsChanged` hai lần (engine bỏ bản sao); marked text trong editor tuỳ biến có thể bị gạch chân hoặc nháy | Verified (log thật) cho `flagsChanged`; Unverified cho marked text |
| Terminal | Ghostty, Terminal.app, iTerm2, WezTerm, Kitty, Alacritty, Warp | Hiển thị preedit kiểu terminal có thể khác; marked text kéo dài qua dấu cách cần xem | Unverified |
| NSTextView và app Cocoa | TextEdit, Notes, Pages, Messages, Xcode | Dự kiến ổn | Unverified |
| Editor native khác | Zed | Tự cài `NSTextInputClient`, chưa rõ | Unverified |

## Kế hoạch test tay

Chạy trên bản đã cài, với model đã pretrain ([07](07-build-and-install.md)); ghi kết quả vào bảng cuối.

| # | Tình huống | Kỳ vọng |
| --- | --- | --- |
| 1 | `has ` rồi nháy Shift hai lần | Marked text đổi `has ` thành `há ` (hoặc ngược lại), chưa có gì chèn |
| 2 | `has ` rồi gõ chữ | Chèn `has ` (hoặc `há `), chữ mới bắt đầu từ mới |
| 3 | `has. ` rồi nháy Shift hai lần | `há. `, dấu câu giữ nguyên |
| 4 | `has ` rồi Return | Chèn `has `, Return tới app (gửi tin nhắn, xuống dòng) |
| 5 | `has ` rồi `@` hoặc chữ số | Chèn `has `, ký tự tới app |
| 6 | `has ` rồi Backspace hai lần | Về marked text `has`, rồi Backspace xoá chữ |
| 7 | `has ` rồi click chuột sang chỗ khác | Từ được chèn ở chỗ cũ |
| 8 | Nháy Shift hai lần ngay sau khi click vào ô, chưa gõ gì | Đổi mode, overlay hiện ở con trỏ (hoặc chuột nếu app không trả caret) |
| 9 | VS Code: nháy Shift hai lần | Hit đúng một lần dù sự kiện đến hai lần |

| App | Bundle id | Kết quả đo | Ngày |
| --- | --- | --- | --- |
| (chưa đo) | | | |
