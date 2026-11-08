# Keyboard I/O qua IMK

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `crates/nimk-engine/src/{composer,hotkey,engine}.rs`, `crates/nimk-engine/src/engine/{flow,switch}.rs`, `macos/NeuralIMK/*.swift`

IME giữ từ đang gõ dưới dạng marked text; khi model kết thúc từ, từ vẫn là marked text ("pending") cho tới khi người dùng bắt đầu từ kế tiếp.
Nháy Shift hai lần lật rendering của từ pending ngay trong marked text, nên không cần `replacementRange` ([ADR 0011](adr/0011-pending-word-commit.md)).

## Luồng một phím

Swift chỉ chuyển `NSEvent` sang Rust và thực thi `EngineOutput` trả về; mọi quyết định nằm trong `nimk-engine`.

```text
+-----------+ handle(_:client:) +----------------------+  nimk_key   +-------------------+
| App focus | ----------------> | InputController.swift| ----------> | Engine::on_key    |
+-----------+                   +----------------------+             | hotkey -> model   |
      ^                                    |                         | -> apply -> log   |
      |     insertText / setMarkedText     |  NimkOut (EngineOutput) +-------------------+
      +------------------------------------+ <-------------------------------+
```

Swift áp `commit` trước, rồi `marked`; `consumed = false` nghĩa là trả phím nguyên vẹn cho app.

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
| `commit` | `client.insertText(_:replacementRange:)` với range `NSNotFound`, đồng thời xoá marked text; là từ pending cộng ranh giới, hoặc từ đang gõ |
| `marked` | `client.setMarkedText(_:selectionRange:replacementRange:)`; `Some("")` xoá preedit |
| `mode` | `Some(_)` → cập nhật status item; đây là sticky mode (mode từ kế tiếp bắt đầu), `None` = không đổi |
| `overlay` | Hiện overlay tại con trỏ khi sticky mode đổi do model ở cuối từ (`Auto`) hoặc do hotkey (`Manual`); prior khi focus chỉ đổi mode, không overlay ([06](06-gui.md)) |

## Composer và action

Composer chỉ là buffer ngu: model chọn `EditAction` cho mỗi phím, engine áp cơ học.
Buffer giữ `raw` của từ, bản `vi = telex(raw)` (công cụ, là feature) và `en = raw`; tối đa 64 phím (`MAX_RAW`).

| `EditAction` | Engine làm gì | Marked / commit |
| --- | --- | --- |
| `Append` | Thêm ký tự của phím vào `raw`, tính lại `vi` và cờ `valid_vi`/`viable_vi`, nuốt phím | `marked` = bản đang hiển thị |
| `Pop` | Bỏ phím cuối; buffer rỗng thì reset từ | `marked` = bản mới, hoặc `""` |
| `Commit` | Kết thúc từ: từ thành pending (bản đang hiển thị); space và dấu câu nối vào ranh giới pending, phím khác commit ngay và đi tiếp tới app | `marked` = từ + ranh giới, `consumed = true`; hoặc `commit = Some(text)`, `consumed = false` |
| `Pass` | Trả phím cho app, buffer giữ nguyên | `marked` chỉ đổi khi rendering đổi |

`ActionMask` chỉ chặn action vô nghĩa về vật lý ([ADR 0007](adr/0007-neural-action-model.md)).

| Quyền | Điều kiện |
| --- | --- |
| `append` | Phím sinh đúng một ký tự in được, không có ⌘/⌃/⌥, buffer chưa đầy; buffer rỗng thì phải là chữ cái |
| `pop` | Buffer không rỗng |
| `commit` | Buffer không rỗng |
| `Pass` | Luôn hợp lệ; phím không phải chữ khi buffer rỗng chỉ có `Pass`, nên engine bỏ qua model, không ghi `decision`; action bị chặn rơi về `Pass` (hoặc action hợp lệ có điểm cao nhất, xem `decide`) |

Phím ranh giới được ghi vào `word_commit.boundary` theo `boundary_of`: `space`, `return`, `tab`, `punct`, `navigation` (mũi tên, Escape), `shortcut` (có ⌘/⌃/⌥), `external` (click chuột, mất focus, `commit_pending`).
`FlagsChanged` không bao giờ vào model hay cửa sổ `recent`; chỉ hotkey nhìn thấy nó.

Telex do `nimk-telex::transform` đảm nhận (engine tự viết, không dùng `vi-rs` để `encode` là nghịch đảo chính xác của `transform`):

```rust
pub fn transform(raw: &str) -> Transformed;
pub fn is_valid_syllable(word: &str) -> bool;
pub fn encode(word: &str) -> String;               // dạng gõ chuẩn
pub fn encode_variants(word: &str) -> Vec<String>; // dạng gõ khác (`w` đứng riêng)
```

`Transformed { text, has_telex, valid_vi, viable_vi }` cấp input cho `Observation`, không còn là luật cứng ([04](04-engine-ml.md)).
`valid_vi` là âm tiết đầy đủ; `viable_vi` là âm tiết hoặc tiền tố của âm tiết (`viê` viable, chưa valid).

## Hotkey chuyển

Hotkey mặc định là nháy Shift hai lần; dự phòng Ctrl+Space cho app nuốt `flagsChanged`.
Không dùng ⌘+Space vì hệ thống nuốt trước khi tới IME ([ADR 0008](adr/0008-double-shift-retro-correction.md); phần sửa ngược đã được [ADR 0011](adr/0011-pending-word-commit.md) thay).

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
- `flagsChanged` lặp lại đúng trạng thái modifier trước đó (VS Code/Electron gửi mỗi sự kiện hai lần, cách nhau vài ms) là bản sao: bỏ qua hoàn toàn khi nhận nháy, `consumed = false`, vẫn ghi record `key`.
- Ctrl+Space (không ⌘/⌥) trả `consumed = true`.
- Với `hotkeys = false`, hotkey vẫn được nhận và nuốt (không đổi mode, không sinh nhãn) để word id khớp log gốc.

## Từ pending

Từ model kết thúc không được chèn vào app ngay: nó cùng ranh giới ở lại marked text và chỉ commit thật khi người dùng bắt đầu từ kế tiếp ([ADR 0011](adr/0011-pending-word-commit.md)).

```text
              letter (model: Append)
   +------+ ------------------------> +-----------+
   | Idle |                           | Composing | <--+  letter / Backspace (Pop)
   +------+ <-----------+             +-----------+ ---+
      ^  ^              |                  |  |
      |  | commit       |  model Commit on |  | Backspace removes the last
      |  | (see table)  |  space/punct     v  | boundary char (word restored)
      |  |              |            +-----------+
      |  +--------------+----------- |  Pending  | <--+  space / punct (boundary grows)
      |        commit pending         +-----------+ ---+  double Shift (flip + Correction)
      |                                    |
      +------------------------------------+
        letter: commit, then Append the letter -> Composing
```

Pending nằm giữa "đã kết thúc" và "đã chèn": mọi phím không bắt đầu từ mới đều hoặc kéo dài pending, hoặc commit nó rồi đi tiếp.

| Sự kiện khi có từ pending | Kết quả |
| --- | --- |
| Chữ cái | `commit` = từ + ranh giới, rồi chữ cái bắt đầu từ mới (luồng model bình thường) |
| Space, `. , ? ! : ; ' " ( )`, `…`, dấu nháy cong | Nối vào ranh giới (tối đa 16), vẫn lật được ("has. " thành "há. "); không gọi model |
| Backspace | Bỏ ký tự ranh giới cuối; hết ranh giới thì từ trở lại thành từ đang gõ (raw, id và rendering hiện tại giữ nguyên) |
| Return, Tab, mũi tên, Escape, phím có ⌘/⌃/⌥ | Commit pending ngay, phím tới app |
| Ký tự in được khác (`@ # $ %`, chữ số...) | Commit pending, phím tới app, không gọi model |
| `commitComposition`, `deactivateServer`, đổi focus, đổi hoặc reset model, shutdown | Commit pending |
| Nháy Shift hai lần | Lật rendering trong marked text, ghi nhãn `Correction`, đặt sticky mode |

Khi không có gì pending hay đang gõ, phím không phải chữ cái đi thẳng tới app mà không gọi model; chỉ ghi record `key`.
Ghi `word_commit` và nhãn của từ diễn ra lúc commit thật, nên Backspace quay lại từ không để lại dấu vết trong log và không đổi word id.

## Ngữ nghĩa hotkey

Hotkey vừa đổi mode vừa là nguồn nhãn chắc chắn; hành vi phụ thuộc trạng thái engine lúc bấm (`SwitchScope`).
Kết quả luôn thành sticky mode, và mỗi lần ghi một record `manual_switch` kèm `overlay = Some(Overlay::Manual)`.

| Tình huống | `scope` | Hành động hiển thị | Nhãn sinh ra |
| --- | --- | --- | --- |
| Đang gõ dở một từ | `in_word` | Lật rendering của từ, khoá rendering đến hết từ (`marked` đổi) | Từ hiện tại = rendering mới, `correction` weight 1.0, cho mọi bước của từ |
| Từ pending | `after_commit` | Lật rendering của từ pending trong marked text; bấm lặp lại bật tắt qua lại | Lúc commit thật: `correction` weight 1.0 theo rendering cuối; không lật thì nhãn ngầm như cũ |
| Không có từ pending, đã commit từ kể từ khi focus | `after_commit` | Chỉ đổi sticky mode | Không |
| Chưa commit từ nào kể từ khi focus | `before_typing` | Đổi sticky mode (prior) | Prior của focus = sai, `correction` weight 1.0 |

Chỉ từ pending (từ vừa kết thúc, chưa chèn) sửa được; từ đã chèn vào app không bị đụng tới.

```text
 typed:   h a s <space>     model picks Commit with EN: marked text "has "  (nothing inserted)
 Shift Shift:               marked text -> "<vi> " (ha + acute), sticky mode = Vi
 typed:   x                 insertText("<vi> "), then "x" starts the next word
                            word_commit(mode Vi) + label Correction(Vi, 1.0) written here
```

Hotkey chỉ đổi cái đang là marked text, nên ranh giới không bao giờ bị mất.

## Lifecycle IMK

Mỗi callback IMK gọi đúng một hàm Engine qua C ABI; không callback nào chứa logic quyết định.

| Callback (`InputController.swift`) | Gọi | Mục đích |
| --- | --- | --- |
| Khởi động process (`Core`) | `nimk_new` → `Engine::new(cfg, storage, session)` | Không bao giờ lỗi; snapshot hỏng hay sink lỗi thì fallback và báo qua `Engine::take_diagnostics` |
| `activateServer(_:)` | `nimk_context` → `Engine::on_context`, trễ 120 ms (debounce) | App hoặc ô nhập (`field_id`) đổi → focus id mới, model chọn prior; kích hoạt mới hoặc `deactivateServer` huỷ lần chờ; phím tới trước thì gửi ngay; caret rỗng thì dò lại sau 80 ms |
| `handle(_:client:)` | `nimk_context` (nếu đang chờ debounce, hoặc phím chữ mở đầu từ khi không có marked text), rồi `nimk_key` → `Engine::on_key` | Xử lý một phím; `event.characters` chỉ đọc với `keyDown` |
| `deactivateServer(_:)` | `nimk_commit_pending` → `Engine::commit_pending` | Không mất preedit hay từ pending khi mất focus |
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
| `InputController.swift` | `@objc(InputController) class InputController: IMKInputController`; chuyển event, áp `EngineOutput`, menu |
| `Debouncer.swift` | Trễ và gộp việc gửi context khi kích hoạt liên tiếp (test: `make swift-test`) |
| `Overlay.swift` | Overlay và status item |
| `NeuralIMK-Bridging.h` | Import `nimk_ffi.h` cho `swiftc -import-objc-header` |

Quy tắc FFI:

- Mỗi hàm `extern "C"` bọc `catch_unwind`; lỗi → `consumed = false` để phím đi thẳng vào app.
- Chuỗi trong `NimkOut` do Rust sở hữu, sống đến lần gọi kế tiếp; Swift copy ngay sang `String`.
- `Engine` không `Sync`; chỉ gọi từ main thread của IMK.

Chưa kiểm chứng trên máy thật (cần cài IME): IME có hiện trong Input Sources, đường phím → marked → commit, status item từ `LSBackgroundOnly`, overlay trên màn hình phụ, độ ổn định của `field_id`, và marked text kéo dài sau dấu cách theo app ([09](09-app-compat.md)).
