# Keyboard I/O qua IMK

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `crates/nimk-engine/src/{composer,hotkey,engine}.rs`, `macos/NeuralIMK/*.swift`

IME giữ từ đang gõ dưới dạng marked text và chỉ commit ở ranh giới từ.
Vì vậy đổi VI/EN cho từ hiện tại luôn là thay preedit, không cần backspace giả.

## Luồng một phím

Swift chỉ chuyển `NSEvent` sang Rust và thực thi `EngineOutput` trả về; mọi quyết định nằm trong `nimk-engine`.

```text
+-----------+ handle(_:client:) +----------------------+  nimk_key   +-------------------+
| App focus | ----------------> | InputController.swift| ----------> | Engine::on_key    |
+-----------+                   +----------------------+             | hotkey -> composer|
      ^                                    |                         | -> policy -> log  |
      |   insertText / setMarkedText       |  NimkOut (EngineOutput) +-------------------+
      +------------------------------------+ <-------------------------------+
```

Swift áp `commit` trước rồi mới áp `marked`; `consumed = false` nghĩa là trả phím nguyên vẹn cho app.

`EngineOutput` (mirror của `NimkOut` trong C ABI):

```rust
pub struct EngineOutput {
    pub consumed: bool,
    pub commit: Option<String>,
    pub marked: Option<String>,
    pub mode: Option<Lang>,
    pub overlay: Option<Overlay>,
}
```

| Trường | Swift làm gì |
| --- | --- |
| `consumed` | `false` → `handle` trả `false`, app nhận phím |
| `commit` | `client.insertText(_:replacementRange:)`, đồng thời xoá marked text |
| `marked` | `client.setMarkedText(_:selectionRange:replacementRange:)`; `Some("")` xoá preedit |
| `mode` | `Some(_)` → cập nhật status item; `None` = không đổi |
| `overlay` | Hiện overlay tại con trỏ; `Auto` và `Manual` khác màu ([06](06-gui.md)) |

## Composer

Composer giữ `raw` của từ hiện tại và luôn tính sẵn hai bản: `vi = telex(raw)`, `en = raw`.
Marked text hiển thị bản theo mode hiện tại của từ; policy được hỏi lại sau mỗi phím và có thể lật mode giữa chừng.

| Phím | Hành vi | `Boundary` |
| --- | --- | --- |
| Chữ cái | Thêm vào `raw`, tính lại `vi`/`en`, hỏi policy, cập nhật marked | — |
| Space | Commit bản đang hiển thị, ghi `word_commit`, reset `raw`, trả Space cho app | `space` |
| Return | Như trên | `return` |
| Tab | Như trên | `tab` |
| Dấu câu, ký tự không phải chữ | Như trên | `punct` |
| Mũi tên, điều hướng | Như trên | `navigation` |
| Phím có ⌘/⌃/⌥ (trừ hotkey) | Commit preedit rồi trả phím cho app để shortcut chạy | `shortcut` |
| `commitComposition` (click chuột, mất focus) | Commit preedit | `external` |
| Backspace | Bớt 1 phím trong `raw`, tính lại; `raw` rỗng → `consumed = false` | — |

Telex do `nimk-telex::transform` đảm nhận (dự kiến bọc crate `vi-rs`, engine của goxkey):

```rust
pub fn transform(raw: &str) -> Transformed;
pub fn is_valid_syllable(word: &str) -> bool;
```

`Transformed { text, has_telex, valid_vi }` cấp luôn input cho luật cứng ở [04](04-engine-ml.md).

## Hotkey chuyển

Hotkey mặc định là gõ nhanh Shift trái một mình; dự phòng Ctrl+Space cho app nuốt `flagsChanged`.
Không dùng ⌘+Space vì hệ thống nuốt trước khi tới IME ([ADR 0003](adr/0003-shift-tap-hotkey.md)).

`recognizedEvents(_:)` phải trả `[.keyDown, .flagsChanged]` để IME nhận sự kiện modifier.
Nhận diện nằm trong Rust, theo `KeyEvent::timestamp`:

```rust
pub(crate) enum HotkeyHit {
    ShiftTap,
    CtrlSpace,
}

impl HotkeyDetector {
    pub(crate) fn new(tap_secs: f64, ctrl_space: bool) -> Self;
    pub(crate) fn feed(&mut self, key: &KeyEvent) -> Option<HotkeyHit>;
}
```

`HotkeyDetector` là module nội bộ của `nimk-engine`; chỉ `engine.rs` gọi nó.

```text
                      flagsChanged, keycode == LEFT_SHIFT (56), SHIFT bit set
         +--------+  --------------------------------------------------->  +-------------+
         |  Idle  |                                                        | ShiftDown   |
         |        |  <---------------------------------------------------  | t0 = ts     |
         +--------+   flagsChanged LEFT_SHIFT up, ts - t0 >  tap_secs      +-------------+
           ^    |                                                            |         |
           |    |                     flagsChanged LEFT_SHIFT up,            |         |
           |    |                     ts - t0 <= tap_secs                    |         |
           |    |   emit ShiftTap  <-----------------------------------------+         |
           |    |   (back to Idle)                                                     |
           |    |                                                                      |
           |    | keyDown SPACE + CONTROL, ctrl_space        any keyDown while held    |
           |    +--> emit CtrlSpace (stay Idle)                                        v
           |                                                                  +-------------+
           +----------------------------------------------------------------- | Cancelled   |
                         flagsChanged LEFT_SHIFT up (no hit)                  +-------------+
```

Chỉ một nhấn-nhả Shift trái không xen phím nào trong ≤ `shift_tap_secs` (0.3 s) mới sinh `ShiftTap`.

| Tham số | Nguồn | Mặc định |
| --- | --- | --- |
| Bật nhận diện hotkey | `EngineConfig::hotkeys` | `true` (replay đặt `false`, nạp nhãn qua `Engine::inject_label`) |
| `tap_secs` | `EngineConfig::shift_tap_secs` | `0.3` |
| `ctrl_space` | `EngineConfig::ctrl_space` | `true` |

Không có cờ riêng để tắt Shift tap mà giữ Ctrl+Space.

Điểm chưa chốt (giải quyết khi viết golden test M1):

- Modifier khác (⌘, ⌥, Shift phải) đổi trạng thái khi đang `ShiftDown`: đề xuất chuyển sang `Cancelled`.
- `flagsChanged` luôn trả `consumed = false` (đề xuất) để app giữ đúng trạng thái modifier.
- Ctrl+Space trả `consumed = true` để app không nhận.

## Ngữ nghĩa hotkey

Hotkey vừa đổi mode vừa là nguồn nhãn chắc chắn; hành vi phụ thuộc vị trí trong vòng đời từ (`SwitchScope`).

| Tình huống | `scope` | Hành động hiển thị | Nhãn sinh ra |
| --- | --- | --- | --- |
| Đang gõ dở một từ | `in_word` | Lật rendering của từ hiện tại, đổi mode | Từ hiện tại = mode mới, weight 1.0 |
| Ngay sau commit, chưa gõ từ mới (≤ 2 từ trước) | `after_commit` | Chỉ đổi mode cho từ kế tiếp, không sửa text đã commit | Từ vừa commit = mode mới, weight 1.0; từ trước đó 0.5 |
| Chưa gõ gì từ khi focus/đổi app | `before_typing` | Đổi mode mặc định | Prior lúc focus = sai, weight 1.0 |

Mỗi lần hotkey ghi một record `manual_switch` và trả `overlay = Some(Overlay::Manual)`.

## Lifecycle IMK

Mỗi callback IMK gọi đúng một hàm Engine qua C ABI; không callback nào chứa logic quyết định.

| Callback (`InputController.swift`) | Gọi | Mục đích |
| --- | --- | --- |
| Khởi động process | `nimk_new` → `Engine::new(cfg, storage, session)` | Không bao giờ lỗi; snapshot hỏng hay sink lỗi thì fallback và báo qua `Engine::take_diagnostics` |
| `activateServer(_:)` | `nimk_context` → `Engine::on_context` | App hoặc ô nhập (`field_id`) đổi → focus id mới, policy chọn prior |
| `handle(_:client:)` | `nimk_key` → `Engine::on_key` | Xử lý một phím |
| `deactivateServer(_:)` | `nimk_commit_pending` → `Engine::commit_pending` | Không mất preedit khi mất focus |
| `commitComposition(_:)` | `nimk_commit_pending` → `Engine::commit_pending` | Click chuột, IMK yêu cầu commit |
| `recognizedEvents(_:)` | — | Trả `[.keyDown, .flagsChanged]` |
| `menu()` | `Engine::mode`, `Engine::policy_id` qua FFI | Menu IME ([06](06-gui.md)) |
| Sau mỗi lời gọi (đề xuất) | `Engine::take_diagnostics` | Ghi cảnh báo vào `~/Library/Logs/NeuralIMK.log` |

File Swift (chưa tạo, thuộc M2):

| File | Vai trò |
| --- | --- |
| `macos/NeuralIMK/main.swift` | Khởi tạo `IMKServer` với `InputMethodConnectionName`, chạy run loop |
| `macos/NeuralIMK/InputController.swift` | `@objc(InputController) class InputController: IMKInputController`; chuyển event, đọc context |
| `macos/NeuralIMK/Overlay.swift` | Overlay và status item |
| `macos/NeuralIMK/NeuralIMK-Bridging.h` | Import `nimk_ffi.h` cho `swiftc -import-objc-header` |

Quy tắc FFI:

- Mỗi hàm `extern "C"` bọc `catch_unwind`; lỗi → `consumed = false` để phím đi thẳng vào app.
- Chuỗi trong `NimkOut` do Rust sở hữu, sống đến lần gọi kế tiếp; Swift copy ngay sang `String`.
- `Engine` không `Sync`; chỉ gọi từ main thread của IMK.
