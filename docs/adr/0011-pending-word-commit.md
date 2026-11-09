# 0011. Từ pending: commit khi bắt đầu từ kế tiếp

> **Trạng thái:** accepted, partially superseded by [0012](0012-unified-boundary-commit.md) (phần ranh giới) · **Ngày:** 2026-11-08 · **Supersedes:** phần sửa ngược bằng `replacementRange` của [0008](0008-double-shift-retro-correction.md)

## Bối cảnh

ADR 0008 sửa từ đã commit bằng `insertText(_:replacementRange:)`, kèm pre-check, post-check và verdict cache theo app.
Nhiều app (terminal, Firefox, Electron) bỏ qua hoặc xử lý sai `replacementRange`, nên tính năng chỉ chạy ở một phần app và cần nhiều mã bảo vệ.

## Quyết định

- Khi model kết thúc từ (`Commit` trên space hoặc dấu câu), engine không chèn gì vào app: rendering đã chọn cộng ký tự ranh giới ở lại marked text ("pending").
- Từ pending chỉ commit thật khi người dùng bắt đầu từ kế tiếp (chữ cái), hoặc khi có phím điều hướng, Return, Tab, Escape, phím tắt, ký tự không phải chữ, `commitComposition`, mất focus, đổi hoặc reset model.
- Space và dấu câu nối thêm vào ranh giới pending; Backspace bớt ký tự ranh giới, và khi hết thì từ trở lại thành từ đang gõ.
- Nháy Shift hai lần khi có từ pending lật rendering trong marked text, ghi nhãn `Correction` và đặt sticky mode.
- `word_commit` và nhãn ghi lúc commit thật; từ bị lật ghi nhãn `Correction` thay vì `Implicit`.
- Mặt nạ cấu trúc: chỉ chữ cái mới được `Append` khi buffer rỗng. Phím không phải chữ khi không có gì đang gõ hay pending không gọi model, chỉ ghi record `key`.
- Gỡ: `EngineOutput.replace`, bộ nhớ từ cuối, các trường `replace_*` của C ABI, pre/post-check và `ReplaceCompat` ở Swift, `replace-compat.json`, hai mục menu và overlay đỏ.

Bảng hành vi đầy đủ và máy trạng thái nằm ở [02](../02-keyboard-io.md#từ-pending).

## Hệ quả

- Không còn phụ thuộc `replacementRange`; mọi app hỗ trợ marked text đều lật được từ ([09](../09-app-compat.md)).
- Ký tự ranh giới nằm trong marked text, nên app thấy chúng muộn hơn một nhịp; Return vẫn tới app sau khi commit nên gửi tin nhắn không đổi.
- Log: `word_commit` đến muộn hơn phím ranh giới; replay bắt buộc phải commit từ pending trước khi nạp nhãn của nó (`Engine::inject_label` làm việc này).
- Dữ liệu huấn luyện: phím không phải chữ khi buffer rỗng, và ranh giới nối vào từ pending, không còn mẫu `Pass` nào; bước `Commit` của từ bị Backspace quay lại bị loại khỏi nhãn.
- Nháy Shift sau khi đã gõ qua từ chỉ còn đổi sticky mode, không sửa được từ đã chèn.
