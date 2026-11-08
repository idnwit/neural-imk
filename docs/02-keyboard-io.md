# Keyboard I/O qua IMK

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `crates/nimk-engine/src/{composer,hotkey,engine}.rs`, `crates/nimk-engine/src/engine/{flow,switch}.rs`, `macos/NeuralIMK/*.swift`

IME giữ từ đang gõ dưới dạng marked text, model quyết định khi nào commit, và nháy Shift hai lần sửa ngược từ vừa commit.
Đổi VI/EN cho từ đang gõ là thay preedit; chỉ sửa từ đã commit mới cần `replacementRange` ([ADR 0008](adr/0008-double-shift-retro-correction.md)).

## Luồng một phím

Swift chỉ chuyển `NSEvent` sang Rust và thực thi `EngineOutput` trả về; mọi quyết định nằm trong `nimk-engine`.

```text
+-----------+ handle(_:client:) +----------------------+  nimk_key   +-------------------+
| App focus | ----------------> | InputController.swift| ----------> | Engine::on_key    |
+-----------+                   +----------------------+             | hotkey -> model   |
      ^                                    |                         | -> apply -> log   |
      |  replace / insertText / setMarked  |  NimkOut (EngineOutput) +-------------------+
      +------------------------------------+ <-------------------------------+
```

Swift áp `replace` trước, rồi `commit`, rồi `marked`; `consumed = false` nghĩa là trả phím nguyên vẹn cho app.

`EngineOutput` (mirror của `NimkOut` trong C ABI):

```rust
pub struct EngineOutput {
    pub consumed: bool,
    pub commit: Option<String>,
    pub marked: Option<String>,
    pub mode: Option<Lang>,
    pub overlay: Option<Overlay>,
    pub replace: Option<Replace>,
}

pub struct Replace {
    pub delete_before: u32,
    pub text: String,
}
```

| Trường | Swift làm gì |
| --- | --- |
| `consumed` | `false` → `handle` trả `false`, app nhận phím |
| `replace` | `applyReplace`: pre-check, `insertText(_:replacementRange:)`, post-check (xem [Sửa ngược](#phía-swift-pre-check-post-check-và-verdict-cache)) |
| `commit` | `client.insertText(_:replacementRange:)` với range `NSNotFound`, đồng thời xoá marked text |
| `marked` | `client.setMarkedText(_:selectionRange:replacementRange:)`; `Some("")` xoá preedit |
| `mode` | `Some(_)` → cập nhật status item; đây là sticky mode (mode từ kế tiếp bắt đầu), `None` = không đổi |
| `overlay` | Hiện overlay tại con trỏ; chỉ khi sticky mode đổi; `Auto` và `Manual` khác màu ([06](06-gui.md)) |

## Composer và action

Composer chỉ là buffer ngu: model chọn `EditAction` cho mỗi phím, engine áp cơ học.
Buffer giữ `raw` của từ, bản `vi = telex(raw)` (công cụ, là feature) và `en = raw`; tối đa 64 phím (`MAX_RAW`).

| `EditAction` | Engine làm gì | Marked / commit |
| --- | --- | --- |
| `Append` | Thêm ký tự của phím vào `raw`, tính lại `vi` và cờ `valid_vi`/`viable_vi`, nuốt phím | `marked` = bản đang hiển thị |
| `Pop` | Bỏ phím cuối; buffer rỗng thì reset từ | `marked` = bản mới, hoặc `""` |
| `Commit` | Commit bản đang hiển thị, ghi `word_commit`, trả phím ranh giới cho app | `commit = Some(text)`, `consumed = false` |
| `Pass` | Trả phím cho app, buffer giữ nguyên | `marked` chỉ đổi khi rendering đổi |

`ActionMask` chỉ chặn action vô nghĩa về vật lý ([ADR 0007](adr/0007-neural-action-model.md)).

| Quyền | Điều kiện |
| --- | --- |
| `append` | Phím sinh đúng một ký tự in được, không có ⌘/⌃/⌥, buffer chưa đầy |
| `pop` | Buffer không rỗng |
| `commit` | Buffer không rỗng |
| `Pass` | Luôn hợp lệ; action bị chặn rơi về `Pass` (hoặc action hợp lệ có điểm cao nhất, xem `decide`) |

Phím ranh giới được ghi vào `word_commit.boundary` theo `boundary_of`: `space`, `return`, `tab`, `punct`, `navigation` (mũi tên, Escape), `shortcut` (có ⌘/⌃/⌥), `external` (click chuột, mất focus, `commit_pending`).
`FlagsChanged` không bao giờ vào model hay cửa sổ `recent`; chỉ hotkey nhìn thấy nó.

Telex do `nimk-telex::transform` đảm nhận (engine tự viết, không dùng `vi-rs` để `encode` là nghịch đảo chính xác của `transform`):

```rust
pub fn transform(raw: &str) -> Transformed;
pub fn is_valid_syllable(word: &str) -> bool;
```

`Transformed { text, has_telex, valid_vi, viable_vi }` cấp input cho `Observation`, không còn là luật cứng ([04](04-engine-ml.md)).
`valid_vi` là âm tiết đầy đủ; `viable_vi` là âm tiết hoặc tiền tố của âm tiết (`viê` viable, chưa valid).

## Hotkey chuyển

Hotkey mặc định là nháy Shift hai lần; dự phòng Ctrl+Space cho app nuốt `flagsChanged`.
Không dùng ⌘+Space vì hệ thống nuốt trước khi tới IME ([ADR 0008](adr/0008-double-shift-retro-correction.md)).

`recognizedEvents(_:)` phải trả `[.keyDown, .flagsChanged]` để IME nhận sự kiện modifier.
Nhận diện nằm trong Rust, theo `KeyEvent::timestamp`:

```rust
pub(crate) enum HotkeyHit {
    DoubleShift,
    CtrlSpace,
}

impl HotkeyDetector {
    pub(crate) fn new(params: HotkeyParams) -> Self;
    pub(crate) fn feed(&mut self, key: &KeyEvent) -> Option<HotkeyHit>;
}
```

Một nháy (tap) là nhấn rồi nhả một phím Shift (trái hoặc phải) đứng một mình, giữ không quá `shift_tap_secs`.
Hai nháy có thời điểm nhả cách nhau không quá `double_shift_secs` thì phát `DoubleShift` ngay lúc nhả lần hai.

```text
          Shift down, alone                       Shift up, held <= tap_secs
 +------+ ------------------> +-------+  ---------------------------------> +---------+
 | Idle |                     | Down1 |                                      | Between |
 +------+ <-----------------  +-------+                                      +---------+
   ^  ^     Shift up, too long    |                                     Shift down, |alone
   |  |                           | other key / modifier                            v
   |  |                           v                                            +---------+
   |  |   all Shift up       +-----------+     other key / modifier           | Down2   |
   |  +--------------------- | Cancelled | <----------------------------------+---------+
   |                         +-----------+                                         |
   |   Shift up, held <= tap_secs and (release - first release) <= double_secs     |
   +---------------------------- emit DoubleShift <-------------------------------+
        (too slow: release becomes a new first tap -> Between)
```

Mọi phím hoặc modifier khác xen giữa lần nhấn đầu và lần nhả thứ hai đều huỷ; một nháy thứ hai quá chậm bắt đầu cặp mới.

| Tham số | Nguồn | Mặc định |
| --- | --- | --- |
| Bật nhận diện hotkey | `EngineConfig::hotkeys` | `true` (replay đặt `false`, nạp nhãn qua `Engine::inject_label`) |
| `tap_secs` | `EngineConfig::shift_tap_secs` | `0.3` |
| `double_secs` | `EngineConfig::double_shift_secs` | `0.4` |
| `double_shift` | `EngineConfig::double_shift` | `true` |
| `ctrl_space` | `EngineConfig::ctrl_space` | `true`; độc lập với `double_shift` |

Hành vi `consumed` đã chốt:

- Phím Shift đứng một mình sau khi đã có ⌘/⌃/⌥ đang giữ không bao giờ là nháy.
- Hotkey hit trả `consumed = true`; `flagsChanged` còn lại trả `consumed = false` để app giữ đúng trạng thái modifier.
- Ctrl+Space (không ⌘/⌥) trả `consumed = true`.
- Với `hotkeys = false`, hotkey vẫn được nhận và nuốt (không đổi mode, không sinh nhãn) để word id khớp log gốc.

## Ngữ nghĩa hotkey và sửa ngược

Hotkey vừa đổi mode vừa là nguồn nhãn chắc chắn; hành vi phụ thuộc trạng thái engine lúc bấm (`SwitchScope`).
Kết quả luôn thành sticky mode, và mỗi lần ghi một record `manual_switch` kèm `overlay = Some(Overlay::Manual)`.

| Tình huống | `scope` | Hành động hiển thị | Nhãn sinh ra |
| --- | --- | --- | --- |
| Đang gõ dở một từ | `in_word` | Lật rendering của từ, khoá rendering đến hết từ (`marked` đổi) | Từ hiện tại = rendering mới, `correction` weight 1.0, cho mọi bước của từ |
| Vừa commit, chỉ gõ thêm ký tự ranh giới in được (space, dấu câu) | `after_commit` | `replace`: xoá `từ + ranh giới`, chèn rendering kia + đúng ranh giới | Từ vừa commit = rendering mới, `correction` weight 1.0; ghi đè nhãn ngầm đã chốt nếu còn trong cache 4 từ |
| Đã gõ qua từ vừa commit, ranh giới bị quên (Return/Tab, phím khác) | `after_commit` | Chỉ đổi sticky mode, không `replace` | Không |
| Chưa commit từ nào kể từ khi focus | `before_typing` | Đổi sticky mode (prior) | Prior của focus = sai, `correction` weight 1.0 |

Bấm lặp lại ngay sau commit bật tắt rendering của từ đó qua lại; mỗi lần phát một `replace` và một nhãn mới (nhãn sau cùng thắng).
Không còn `correction_prev_weight`: chỉ từ vừa commit được sửa.

```text
 typed:   h a s <space>     model picks Commit with VI: app now shows "<vi> " (2 + 1 chars)
 memory:  LastWord { raw "has", vi = <vi>, lang Vi, boundary " " }   (<vi> = "ha" + acute)
 Shift Shift:
   old = <vi> + " "   -> 3 UTF-16 units  -> delete_before = 3
   new = "has" + " "   -> Replace { delete_before: 3, text: "has " }
   label: Word(id) = En, correction, weight 1.0 ; sticky mode = En
```

Sửa ngược thay đúng chuỗi `từ + ranh giới`, nên ranh giới do app tự chèn không bị mất.

### Bộ nhớ ranh giới

`LastWord` (raw, vi, rendering đã commit, chuỗi ranh giới) tồn tại cho tới khi người dùng gõ phím không phải ranh giới.

| Sự kiện | Bộ nhớ |
| --- | --- |
| Model `Commit` bằng phím in được | Tạo `LastWord`, ranh giới = ký tự đó |
| Phím in được tới app khi buffer rỗng | Nối vào ranh giới (tối đa 16 ký tự) |
| Return, Tab (ký tự điều khiển), Backspace, mũi tên, shortcut | Xoá |
| `Append` (bắt đầu từ mới) | Xoá |
| Đổi focus, `commit_pending` | Xoá |

### Phía Swift: pre-check, post-check và verdict cache

Swift chỉ chạm vào text khi chắc chắn; không chắc thì bỏ qua, engine vẫn đã đổi mode và học.

```text
 EngineOutput.replace --> allows(bundle)? --no--> skip + overlay "khong sua duoc"
                              | yes
                              v
                    pre-check: selectedRange != NSNotFound, caret >= delete,
                    string(from: range) == tail of `recent` (text we inserted)
                              | pass                       fail --> skip, verdict unchanged
                              v
                    insertText(text, replacementRange: range)
                              |
                              v
                    post-check: string(from: new range) == text
                       | yes: verdict ok, update `recent`    | no: verdict unsupported
```

Hai lần đọc lại (trước và sau) ngăn ghi nhầm vào app bỏ qua `replacementRange` hoặc có caret cũ (Chromium).

| Thành phần | Hành vi (`InputController.swift`, `ReplaceCompat.swift`) |
| --- | --- |
| `recent` | Text Swift tự theo dõi trước con trỏ: commit của engine và ký tự thường cho qua, tối đa 256; Backspace bớt 1; mũi tên, shortcut xoá sạch |
| Pre-check | `selectedRange().location != NSNotFound`, `delete > 0`, `location >= delete`, đủ ký tự trong `recent`, đọc lại `string(from:actualRange:)` khớp đúng đoạn cần thay |
| Post-check | Đọc lại khoảng mới; khớp → ghi `ok`, sai → ghi `unsupported` và không thử lại ở app đó |
| Verdict | `unknown` / `ok` / `unsupported`; `unknown` vẫn được thử; thứ tự ưu tiên: override người dùng > deny list (và tiền tố `com.jetbrains.`) > allow list > học được |
| Lưu | `replace-compat.json` trong data dir, hai bảng `learned` và `overrides` |
| Menu IME | Mục `Retro-correction in this app: on/off` đảo override cho bundle id hiện tại; mục `Last replace issue` hiện chẩn đoán gần nhất |
| Overlay | Khi bỏ qua: badge đỏ, chữ gạch ngang và dòng "không sửa được ở app này" |

Không có fallback `CGEvent` (backspace giả); quyết định này và danh sách seed nằm ở [09](09-app-compat.md).

## Lifecycle IMK

Mỗi callback IMK gọi đúng một hàm Engine qua C ABI; không callback nào chứa logic quyết định.

| Callback (`InputController.swift`) | Gọi | Mục đích |
| --- | --- | --- |
| Khởi động process (`Core`) | `nimk_new` → `Engine::new(cfg, storage, session)` | Không bao giờ lỗi; snapshot hỏng hay sink lỗi thì fallback và báo qua `Engine::take_diagnostics` |
| `activateServer(_:)` | `nimk_context` → `Engine::on_context` | App hoặc ô nhập (`field_id`) đổi → focus id mới, model chọn prior |
| `handle(_:client:)` | `nimk_context` khi phím chữ mở đầu từ (không marked text), rồi `nimk_key` → `Engine::on_key` | Xử lý một phím; `event.characters` chỉ đọc với `keyDown` |
| `deactivateServer(_:)` | `nimk_commit_pending` → `Engine::commit_pending` | Không mất preedit khi mất focus |
| `commitComposition(_:)` | `nimk_commit_pending` → `Engine::commit_pending` | Click chuột, IMK yêu cầu commit |
| `recognizedEvents(_:)` | — | Trả `[.keyDown, .flagsChanged]` |
| `menu()` | `nimk_mode`, `nimk_policy_id` | Menu IME ([06](06-gui.md)) |
| SIGTERM | `nimk_shutdown` | Flush log, lưu snapshot |

File Swift (`macos/NeuralIMK/`):

| File | Vai trò |
| --- | --- |
| `main.swift` | Khởi tạo `IMKServer`, chạy run loop, bắt SIGTERM |
| `Core.swift` | Giữ `NimkCore`, copy `NimkOut` sang `CoreOutput`, log chẩn đoán |
| `Context.swift` | Đọc con trỏ và cửa sổ, cache 1 s ([03](03-context-io.md)) |
| `InputController.swift` | `@objc(InputController) class InputController: IMKInputController`; chuyển event, `applyReplace`, menu |
| `ReplaceCompat.swift` | Verdict cache theo bundle id |
| `Overlay.swift` | Overlay và status item |
| `NeuralIMK-Bridging.h` | Import `nimk_ffi.h` cho `swiftc -import-objc-header` |

Quy tắc FFI:

- Mỗi hàm `extern "C"` bọc `catch_unwind`; lỗi → `consumed = false` để phím đi thẳng vào app.
- Chuỗi trong `NimkOut` do Rust sở hữu, sống đến lần gọi kế tiếp; Swift copy ngay sang `String`.
- `Engine` không `Sync`; chỉ gọi từ main thread của IMK.

Chưa kiểm chứng trên máy thật (cần cài IME): IME có hiện trong Input Sources, đường phím → marked → commit, status item từ `LSBackgroundOnly`, overlay trên màn hình phụ, độ ổn định của `field_id`, và mọi hành vi sửa ngược theo app ([09](09-app-compat.md)).
