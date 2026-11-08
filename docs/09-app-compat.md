# Tương thích app cho sửa ngược

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `macos/NeuralIMK/{ReplaceCompat,InputController}.swift`, `crates/nimk-engine/src/engine/switch.rs`

Sửa ngược (nháy Shift hai lần sau commit) chỉ chạy được ở app xử lý đúng `insertText(_:replacementRange:)`; còn lại IME chỉ đổi mode và học.
Chưa có mục nào dưới đây được thử trên máy thật; cột "Căn cứ" cho biết mức chắc chắn.

## Mức chắc chắn

Hai nhãn dùng xuyên suốt tài liệu này; không nhãn nào nghĩa là đã chạy thử trong app.

| Nhãn | Nghĩa |
| --- | --- |
| Verified (nguồn) | Đã đọc mã nguồn của app hoặc framework và thấy hành vi cần thiết |
| Unverified | Suy đoán từ kiến trúc app hoặc tài liệu bên thứ ba; cần test tay |

## Chiến lược

Ba nguyên tắc: không chạm vào text khi chưa chắc, nhớ kết quả theo app, và không bao giờ giả phím.

1. Engine luôn đổi mode và ghi nhãn `correction` cho từ vừa commit, dù text có sửa được hay không ([02](02-keyboard-io.md)).
2. Swift chỉ áp `replace` sau pre-check: `selectedRange` hợp lệ và đoạn text trước con trỏ đọc lại đúng bằng text IME đã chèn (`recent`).
3. Sau khi áp, post-check đọc lại; sai thì đánh dấu app là `unsupported` và không thử lại.
4. Verdict (`unknown`/`ok`/`unsupported`) cache theo bundle id trong `replace-compat.json`; seed gồm danh sách cấm và cho phép; người dùng ghi đè qua menu IME (`Retro-correction in this app`).
5. Không dùng fallback `CGEventPost` (backspace giả rồi gõ lại); người dùng đã quyết định không làm (2026-11-08), nên không cần quyền Accessibility.

```text
 engine: replace? -> verdict == unsupported? --yes--> skip, badge "khong sua duoc"
                          | no (unknown or ok)
                          v
                  pre-check readback (selectedRange, string(from:))
                          | fail: skip, verdict unchanged
                          v
                  insertText(replacementRange)  -> post-check: ok | unsupported
```

Seed cứng trong `ReplaceCompat.swift`; bundle id đánh dấu "đã thấy trên máy" là id đã kiểm tra tồn tại trên máy người dùng.

| Danh sách | Bundle id |
| --- | --- |
| Cấm (`unsupported`) | `com.mitchellh.ghostty`, `com.apple.Terminal`, `com.googlecode.iterm2` (thấy), `com.github.wez.wezterm` (thấy), `net.kovidgoyal.kitty`, `org.alacritty` (thấy), `dev.warp.Warp-Stable` (thấy), `dev.warp.Warp`, `org.mozilla.firefox` (thấy), tiền tố `com.jetbrains.` |
| Cho phép (`ok`) | `com.apple.TextEdit`, `com.apple.Notes`, `com.apple.iWork.Pages` (thấy), `com.apple.MobileSMS` (thấy), `com.apple.dt.Xcode` |
| Không seed (bắt đầu `unknown`, thử và học) | `com.google.Chrome` (thấy), `com.microsoft.VSCode` (thấy), `com.todesktop.230313mzl4w4u92` Cursor (thấy), `dev.zed.Zed` (thấy), mọi app khác |

Thứ tự ưu tiên khi tra: override của người dùng, danh sách cấm, danh sách cho phép, rồi verdict đã học.

## Mức hỗ trợ theo loại app

Dự kiến, theo cách mỗi loại app cài đặt `NSTextInputClient`; "Có thể" nghĩa là cần test tay trước khi tin.

| Loại | App | Dự kiến | Căn cứ | Mức chắc chắn |
| --- | --- | --- | --- | --- |
| Trình duyệt Chromium | Chrome, Edge, Arc | Một phần: ô nhập gốc (input, textarea) có thể; editor rich text chưa rõ | `render_widget_host_view_cocoa.mm`: `insertText:replacementRange:` chuyển `ImeCommitText(text, range)`; `selectedRange` cache bất đồng bộ nên có thể trễ; `attributedSubstring` giao với text đã cache | Verified (nguồn Chromium) cho cơ chế; Unverified cho từng trang |
| Safari | Safari | Có thể (WebKit dùng NSTextInputClient chuẩn cho ô gốc) | Suy đoán từ kiến trúc | Unverified |
| Firefox | Firefox | Khó: hỗ trợ IMKit yếu | Tài liệu thiết kế Mozc; Mozilla bug 1319018 (IME tiếng Việt xung đột autocomplete thanh URL) | Unverified (tài liệu bên thứ ba); trong danh sách cấm |
| Terminal Ghostty | Ghostty | Không | `SurfaceView` bỏ qua `replacementRange` (chỉ nối thêm), `attributedSubstring` trả vùng chọn | Verified (nguồn Ghostty); trong danh sách cấm |
| Terminal khác | Terminal.app, iTerm2, WezTerm, Kitty, Alacritty, Warp | Không (iTerm2 chưa rõ) | Terminal không có model text sửa được trước con trỏ | Unverified; trong danh sách cấm |
| Editor Electron | VS Code, Cursor, Obsidian, Slack, Notion | Một phần: ô nhập gốc có thể, editor tuỳ biến (Monaco, ProseMirror) chưa rõ | Cùng cơ chế Chromium | Unverified |
| IDE JetBrains | IntelliJ, PyCharm, ... | Có thể không hoặc một phần | Dùng widget Swing/JBR riêng | Unverified; cấm theo tiền tố |
| NSTextView và app Cocoa | TextEdit, Notes, Pages, Messages, Xcode | Có | `NSTextView` triển khai đầy đủ `NSTextInputClient` | Unverified (dự kiến cao nhất); trong danh sách cho phép |
| Editor native khác | Zed | Có thể | Tự cài `NSTextInputClient` | Unverified |

Tham khảo thêm: OpenKey (bộ gõ tiếng Việt khác) dùng kỹ thuật backspace giả và có sửa riêng cho Chrome, Safari, Firefox, Excel; đó là con đường NeuralIMK đã chọn không đi.

Với Chromium, `selectedRange` trễ là rủi ro đã biết: pre-check đọc lại text trước con trỏ nên chỉ bỏ qua (verdict không đổi), không gây hỏng text.

## Kế hoạch test tay

Mục tiêu là điền cột "Mức chắc chắn" thành kết quả đo; chạy trên bản đã cài, với model đã pretrain ([07](07-build-and-install.md)).
Với mỗi app: gõ `has ` rồi nháy Shift hai lần, kỳ vọng `há ` (hoặc ngược lại); ghi kết quả vào bảng dưới.

| # | Tình huống | Kỳ vọng |
| --- | --- | --- |
| 1 | `has ` ở cuối dòng, double Shift | `há `, ranh giới (dấu cách) giữ nguyên |
| 2 | `has ` giữa dòng (con trỏ giữa câu) | Chỉ từ vừa gõ đổi, text sau con trỏ không đổi |
| 3 | `has, ` hoặc `has.` (dấu câu làm ranh giới) | Từ đổi, dấu câu giữ nguyên |
| 4 | Gõ tiếp một chữ rồi double Shift | Không sửa ngược; chỉ đổi mode (`after_commit` không có `replace`) |
| 5 | Double Shift hai lần liên tiếp sau commit | Bật tắt qua lại, text về như cũ |
| 6 | Chromium: `<input>`, `<textarea>`, thanh URL, `contenteditable` | Ghi lại ô nào áp được, ô nào bị bỏ qua |
| 7 | Editor và ô tìm kiếm trong cùng app (VS Code, Cursor) | Ghi lại khác biệt giữa hai loại ô |
| 8 | Terminal: dòng lệnh shell | Không đổi text, overlay đỏ "không sửa được ở app này" |
| 9 | Menu IME: `Retro-correction in this app` bật/tắt | Tắt thì mọi lần sau bị bỏ qua; bật lại thì thử lại |
| 10 | `replace-compat.json` sau khi chạy | `learned` có `ok`/`unsupported` đúng với kết quả quan sát |

Khi một ca thất bại, ghi lại `selectedRange` và `attributedSubstring` mà app trả về (xem `Last replace issue` trong menu và `~/Library/Logs/NeuralIMK.log`), rồi cập nhật bảng mức hỗ trợ và seed trong `ReplaceCompat.swift` cùng commit.

| App | Bundle id | Kết quả đo | Ngày |
| --- | --- | --- | --- |
| (chưa đo) | | | |
