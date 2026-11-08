# 0003. Hotkey chuyển = gõ nhanh Shift trái, dự phòng Ctrl+Space

> **Trạng thái:** superseded by 0008 · **Ngày:** 2026-11-08

## Bối cảnh

Hotkey chuyển vừa là thao tác người dùng vừa là nguồn nhãn chắc chắn, nên IME phải nhận được nó trực tiếp.
⌘+Space bị hệ thống nuốt trước khi tới IME. Event tap toàn cục cần quyền Accessibility, trái với mục tiêu không xin quyền.

## Quyết định

- Mặc định: nhấn và nhả Shift trái một mình trong ≤ `shift_tap_secs` (0.3 s), không xen `keyDown` nào.
- IME khai báo `recognizedEvents(_:)` = `[.keyDown, .flagsChanged]` để nhận sự kiện modifier.
- Dự phòng: Ctrl+Space qua `keyDown`, bật bằng `EngineConfig::ctrl_space` (mặc định `true`).
- Nhận diện là state machine trong Rust (`HotkeyDetector`), theo `KeyEvent::timestamp`; Swift không xử lý.
- Hotkey ngay sau commit (`after_commit`) chỉ đổi mode cho từ kế tiếp và sinh nhãn sửa; không viết lại text đã commit.

## Hệ quả

- Không cần quyền Accessibility hay event tap.
- Shift giữ để gõ hoa không kích hoạt vì luôn có `keyDown` xen giữa.
- App nào nuốt `flagsChanged` thì chỉ còn Ctrl+Space, và Ctrl+Space trùng shortcut của một số app.
- Từ vừa commit sai mode vẫn sai trên màn hình; người dùng tự xoá nếu muốn. Sửa text đã commit nằm ngoài phạm vi PoC dù review kiến trúc đã nêu.
- Vì hotkey sinh nhãn, ngữ nghĩa của nó theo `SwitchScope` là một phần của hợp đồng học ([02](../02-keyboard-io.md)).
