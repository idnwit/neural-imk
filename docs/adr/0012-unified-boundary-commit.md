# 0012. Commit và ranh giới thống nhất: Idle, Composing, Pending, Raw

> **Trạng thái:** accepted · **Ngày:** 2026-11-09 · **Supersedes:** phần ranh giới của [0011](0011-pending-word-commit.md) ("pending giữ tối đa 16 ký tự ranh giới")

## Bối cảnh

ADR 0011 cho từ pending nối nhiều ký tự ranh giới và để model quyết định mọi phím, kể cả `Pass` giữa từ.
Hệ quả: ở terminal, IDE và trình duyệt, `ls -la`, `user_name`, `http://x.com` hay `/help` cho kết quả khó đoán và marked text kéo dài ngoài ý muốn.
Cần một quy tắc duy nhất, giải thích được bằng một bảng, và mọi tham số nằm trong cấu hình thay vì hằng số trong code.

## Quyết định

Máy trạng thái 4 trạng thái là nguồn sự thật duy nhất; bảng chuyển trạng thái nằm ở `crates/nimk-engine/src/engine/machine.rs` (hàm `transition`, có test đối chiếu) và được trình bày ở [02](../02-keyboard-io.md#máy-trạng-thái).

- Trạng thái: `Idle`, `Composing(word)`, `Pending(word + đúng một ký tự ranh giới)`, `Raw`.
- Phím chia lớp: chữ cái (ngôn ngữ), ký tự in được không phải chữ (ranh giới; một số là `command_triggers`), Backspace, phím commit (Return kể cả Shift/⌘/⌃/⌥, Tab, Escape, mũi tên, Home/End/PageUp/PageDown, Delete tới, phím chức năng, mọi phím tắt ⌘/⌃/⌥), còn lại bị bỏ qua.
- Cạnh tranh giữa các quy tắc không còn: edit do phím quyết định (chữ = `Append`, Backspace = `Pop`, phím kết thúc = `Commit`); model chỉ chọn rendering và `p_vi`.
- Ký tự không phải chữ khi `Composing` thành `Pending` với đúng một ký tự (vẫn lật được bằng nháy Shift). Ký tự không phải chữ thứ hai khi `Pending` commit từ rồi cho ký tự đó đi thẳng tới app, về `Idle`, kể cả khi nó là trigger: `http:/` commit `http:` rồi `/` sau đó từ `Idle` mới vào `Raw`.
- `Idle` + trigger (mặc định `/`, `@`, `!`) vào `Raw`: phím đi thẳng tới app, không buffer, không Telex, không gọi model, chỉ ghi record `key`. `Raw` kết thúc ở ký tự trong `command_end_chars` (mặc định space) hoặc keycode trong `command_end_keycodes` (Return, Tab, Escape, điều hướng) hoặc phím tắt; phím kết thúc vẫn tới app. Backspace trong `Raw` đi qua và ở lại `Raw`. Trigger giữa từ chỉ là ranh giới (`và/hoặc`).
- Nháy Shift: `Composing` lật từ và khoá; `Pending` lật từ pending (nhãn `Correction`, sticky mode); `Idle` đổi sticky mode (trước khi gõ thì nhãn prior của focus); `Raw` chỉ đổi sticky mode, không nhãn.
- Mọi lần commit (Return, Tab, Escape, điều hướng, phím tắt, click chuột, mất focus, đổi model, timer rảnh) theo thứ tự: `insertText` trước, rồi phím tới app (`consumed = false`). `Composing` commit quyết định mới nhất của model.
- Timer rảnh nằm ở Swift (engine không có đồng hồ): sau `idle_commit_secs` không có phím, khi đang `Composing` hoặc `Pending`, Swift gọi `nimk_commit_pending`. Mỗi phím đặt lại timer; commit và deactivate huỷ nó. `Raw` không buffer gì nên không cần timer.
- Nhãn: commit bằng timer, Return, ký tự ranh giới thứ hai hoặc chữ cái là `Implicit` (không sửa); nháy Shift là `Correction`.
- Cấu hình: mọi tham số nằm trong TOML, mặc định ở `crates/nimk-engine/config/default.toml` (nhúng bằng `include_str!`, `EngineConfig::default()` là kết quả parse của nó), ghi đè của người dùng ở `<Application Support>/<bundle id>/config.toml` do Swift đọc và đưa văn bản qua `nimk_set_config`. Ghi đè theo nhóm app (`[[app_category]]`: terminal, ide, browser, editor) được chọn theo `bundle_id` mỗi lần focus đổi. Giá trị sai kiểu hoặc ngoài khoảng bị bỏ, báo diagnostic, giữ mặc định; không bao giờ làm hỏng khởi tạo. Chi tiết ở [11](../11-config.md).

## Hệ quả

- Hành vi dự đoán được trong mọi app: tối đa một ký tự ranh giới nằm trong marked text, app thấy trễ không quá một ký tự hoặc `idle_commit_secs`.
- Dữ liệu huấn luyện: `ActionMask` của quan sát giờ chỉ còn một quyền; phím không phải chữ khi `Idle` và mọi phím `Raw` không còn mẫu nào; từ có ký tự không phải chữ giữa chừng (`user_name`) tách thành nhiều từ. Model đã pretrain vẫn dùng được (đầu rendering không đổi) nhưng nên pretrain lại sau khi generator (`nimk-synth`) khớp ngữ nghĩa mới; dòng `/src/home/config` trong log synthetic nay là `Raw`, không sinh `word_commit`.
- Log: `word_commit` vẫn đến muộn hơn phím ranh giới; word id vẫn độc lập với mode.
- Thay đổi ABI: thêm `nimk_set_config`, `nimk_idle_commit_secs`, `nimk_note_navigation`.
- Rủi ro: URL và đường dẫn gõ sau trigger là `Raw` nên không bao giờ được Telex (đúng ý); ngược lại `/` đầu câu tiếng Việt ("/" làm dấu gạch) cũng vào `Raw` tới khi gõ space.
