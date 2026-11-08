# 0001. Một input source IMK duy nhất, VI/EN là chế độ nội bộ

> **Trạng thái:** accepted · **Ngày:** 2026-11-08

## Bối cảnh

Muốn tự chọn VI/EN cho từng từ, IME phải thấy mọi phím ở cả hai chế độ và đổi chế độ mà không cần macOS can thiệp.
Nếu VI và EN là hai input source riêng, đổi chế độ phải gọi `TISSelectInputSource`, chậm và làm mất preedit.
Khi ở source EN (ABC), IME không nhận phím nào nên không thể học hay quyết định.

## Quyết định

- NeuralIMK là một input source InputMethodKit duy nhất; EN là chế độ pass-through bên trong IME.
- Telex là kiểu gõ VI duy nhất trong PoC.
- Mode (`Lang::En` / `Lang::Vi`) là trạng thái của `Engine`, không phải của hệ thống.

## Hệ quả

- IME thấy mọi phím, kể cả khi đang EN, nên policy có thể lật mode giữa từ và log đủ dữ liệu.
- Ở EN, mọi phím vẫn đi qua IME và composer; hot path phải rẻ ở cả hai chế độ (p99 < 2 ms).
- Hotkey hệ thống ⌘+Space không còn dùng để đổi VI/EN; cần hotkey riêng ([0003](0003-shift-tap-hotkey.md)).
- IME crash thì mất cả EN; người dùng phải giữ ABC làm đường lùi.
