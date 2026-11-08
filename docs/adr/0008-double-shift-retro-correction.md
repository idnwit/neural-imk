# 0008. Nháy Shift hai lần, sửa ngược từ vừa commit

> **Trạng thái:** partially superseded by 0011 (phần sửa ngược bằng `replacementRange`; hotkey nháy Shift hai lần vẫn hiệu lực) · **Ngày:** 2026-11-08 · **Supersedes:** 0003

## Bối cảnh

ADR 0003 chọn nháy một lần Shift trái làm hotkey. Switch sau commit chỉ đổi mode cho từ kế tiếp, không viết lại text. Cách này có hai vấn đề:

- Một lần nháy Shift dễ bấm nhầm.
- Người dùng thường chỉ nhận ra từ sai *sau khi* gõ dấu cách. Lúc đó hotkey không sửa được từ sai, nên nhãn sinh ra mờ: người dùng đổi mode cho từ sau, chứ không xác nhận rằng từ vừa gõ bị sai.

## Quyết định

- **Hotkey:** nháy Shift hai lần. Hai lần nhấn-nhả một phím Shift bất kỳ (trái hoặc phải), mỗi lần đứng một mình và giữ ≤ `shift_tap_secs` (0,3 s); hai lần nhả cách nhau ≤ `double_shift_secs` (0,4 s). Phím hay modifier khác xen giữa thì huỷ; nháy thứ hai quá chậm bắt đầu một cặp mới. Một lần nháy đơn không làm gì. Ctrl+Space giữ làm dự phòng.
- **Ngữ nghĩa theo thời điểm:**

| Lúc bấm | Hành động | Nhãn |
| --- | --- | --- |
| Đang gõ dở một từ | Lật rendering của từ hiện tại, khoá đến hết từ | Correction cho từ hiện tại, trọng số 1.0 |
| Ngay sau khi commit: chỉ gõ thêm ký tự biên in được (space, dấu câu) | **Thay từ vừa commit** bằng rendering kia, giữ nguyên phần biên đã gõ | Correction cho từ đó, trọng số 1.0 |
| Đã qua từ vừa commit (Return, Tab, phím khác làm quên biên) nhưng đã có commit kể từ focus | Chỉ đổi sticky mode, không thay text | Không có nhãn |
| Chưa commit từ nào kể từ khi focus | Đổi mode prior | Correction cho focus prior |

- **Engine** trả `EngineOutput.replace = Some(Replace { delete_before, text })`: xoá `delete_before` ký tự (đơn vị UTF-16, theo đơn vị của `NSString`) ngay trước con trỏ, rồi chèn `text`. `delete_before` gồm từ vừa commit cộng các ký tự biên đã gõ sau nó; `text` là rendering mới cộng lại đúng các ký tự biên đó.
- **Swift** áp dụng bằng `client.insertText(text, replacementRange: NSRange(location: caret - delete_before, length: delete_before))`, với `caret` lấy từ `client.selectedRange()`.
- **App không hỗ trợ thì chỉ đổi mode và học** (người dùng chốt ngày 2026-11-08).
  - Trước khi thay, đọc lại đoạn text trước con trỏ. Nếu không khớp, hoặc `selectedRange` là `NSNotFound`, thì không đụng vào text.
  - Engine vẫn đổi mode và ghi nhãn correction. Overlay hiện kiểu "không sửa được ở app này".
  - **Không** dùng backspace giả qua `CGEvent`, nên không cần quyền Accessibility.
  - Kết quả hỗ trợ được cache theo bundle id (`replace-compat.json`): `unknown` / `ok` / `unsupported`. Có sẵn danh sách cấm (các terminal, Firefox, JetBrains) và danh sách cho phép (các app NSTextView). Đọc lại sau khi thay mà sai thì đánh dấu app là `unsupported`. Người dùng ghi đè được qua menu.
  - Mức hỗ trợ dự kiến theo từng app: xem `docs/09-app-compat.md`.
- **Học:** sửa ngược sinh correction cho mọi bước của từ đó, kể cả khi nhãn ngầm đã chốt (labeler giữ steps của 4 từ cuối). Bấm lặp lại bật tắt rendering của từ và mỗi lần sinh một nhãn mới. Bỏ `correction_prev_weight`: chỉ từ vừa commit được sửa.

## Hệ quả

- Tín hiệu online learning rõ: mỗi lần sửa ngược là một nhãn chắc chắn cho đúng một từ.
- Phải kiểm tra thực tế từng app xem có hỗ trợ `replacementRange` không: TextEdit, Notes, Chrome, VS Code, Terminal.
- Engine phải nhớ từ vừa commit (raw, hai rendering, chuỗi biên, tối đa 16 ký tự) cho tới khi người dùng gõ phím không phải biên; Return và Tab xoá bộ nhớ này.
- Tình trạng thực tế theo app và kế hoạch test: [09-app-compat.md](../09-app-compat.md).
