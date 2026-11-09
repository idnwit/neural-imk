# Keyboard I/O qua IMK

> **Trạng thái:** draft · **Cập nhật:** 2026-11-09 · **Code:** `crates/nimk-engine/src/{composer,hotkey,engine}.rs`, `crates/nimk-engine/src/engine/{flow,machine,switch}.rs`, `macos/NeuralIMK/*.swift`

IME chạy một máy 4 trạng thái (`Idle`, `Composing`, `Pending`, `Raw`); từ đang gõ là marked text, và từ vừa kết thúc ("pending") giữ đúng một ký tự ranh giới cho tới phím kế tiếp ([ADR 0012](adr/0012-unified-boundary-commit.md)).
Nháy Shift hai lần lật rendering của từ đang gõ hoặc pending ngay trong marked text, nên không cần `replacementRange` ([ADR 0011](adr/0011-pending-word-commit.md)).

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

Composer chỉ là buffer ngu: phím quyết định `EditAction` (máy trạng thái bên dưới), model chọn rendering cho mỗi phím, engine áp cơ học.
Buffer giữ `raw` của từ, bản `vi = telex(raw)` (công cụ, là feature) và `en = raw`; tối đa 64 phím (`MAX_RAW`).

| `EditAction` | Engine làm gì | Marked / commit |
| --- | --- | --- |
| `Append` | Thêm ký tự của phím vào `raw`, tính lại `vi` và cờ `valid_vi`/`viable_vi`, nuốt phím | `marked` = bản đang hiển thị |
| `Pop` | Bỏ phím cuối; buffer rỗng thì reset từ | `marked` = bản mới, hoặc `""` |
| `Commit` | Kết thúc từ: ký tự không phải chữ thành ranh giới của từ pending (bản đang hiển thị); phím commit (Return, Tab, phím tắt...) commit ngay và đi tiếp tới app | `marked` = từ + ranh giới, `consumed = true`; hoặc `commit = Some(text)`, `consumed = false` |
| `Pass` | Trả phím cho app, buffer giữ nguyên | `marked` chỉ đổi khi rendering đổi |

`ActionMask` của quan sát giờ chỉ còn đúng một quyền, suy ra từ bảng chuyển trạng thái ([ADR 0012](adr/0012-unified-boundary-commit.md)); model chọn `p_vi` chứ không chọn edit.

| Quyền | Khi nào |
| --- | --- |
| `append` | Chữ cái (kể cả khi `Idle` hay `Pending`, lúc đó mở từ mới); buffer chưa đầy |
| `pop` | Backspace khi `Composing` |
| `commit` | Ký tự không phải chữ, phím commit, phím tắt, hoặc chữ cái khi buffer đầy, khi `Composing` |
| (không quyền) | `Idle` + phím khác chữ, và mọi phím `Raw`: engine bỏ qua model, không ghi `decision`, chỉ ghi `key` |

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

## Máy trạng thái

Bảng chuyển trạng thái trong `engine/machine.rs` là nguồn sự thật duy nhất; mọi phím chỉ làm đúng một việc theo (trạng thái, lớp phím) ([ADR 0012](adr/0012-unified-boundary-commit.md)).

```text
   +------+ letter  +-----------+ non-language char +---------+
   | Idle | ------> | Composing | ----------------> | Pending |
   +------+         +-----------+ <---------------- +---------+
      |  ^            |  ^   Backspace (drops the boundary) |  |
      |  |  commit key|  +-- letter / Backspace (self loop) |  |
      |  +------------+                                     |  |
      |  +--------------------------------------------------+  |
      |    commit key, or a 2nd non-language char: commit it,  |
      |    the key goes on to the app, back to Idle            |
      |           letter: commit, then a new Composing --------+
      | trigger (/ @ !)
      v
   +-----+ any other key passes through (self loop)
   | Raw | ---> Idle on a terminator (space, Return, Tab, Esc, arrows, chord)
   +-----+
```

Chỉ `Composing` và `Pending` giữ marked text; `Raw` không buffer gì và `Idle` thì trống.

| Trạng thái | Chữ cái | Ký tự không phải chữ | Trigger (`/ @ !`) | Backspace | Phím commit / phím tắt | Nháy Shift hai lần |
| --- | --- | --- | --- | --- | --- | --- |
| `Idle` | `Composing` (model chọn rendering) | Đi thẳng, không gọi model | Vào `Raw`, trigger đi thẳng | Đi thẳng | Đi thẳng | Đổi sticky mode (trước khi gõ: nhãn prior) |
| `Composing` | Thêm vào từ (model chọn rendering từng phím) | `Pending` giữ đúng 1 ký tự (marked text = rendering + ký tự) | Như ký tự không phải chữ (`và/hoặc`) | Bỏ 1 phím; rỗng thì `Idle` | Commit quyết định mới của model, phím tới app, `Idle` | Lật từ và khoá (`in_word`) |
| `Pending` | Commit từ + ranh giới, chữ cái mở từ mới | Commit, ký tự thứ hai đi thẳng tới app (không marked), `Idle`, kể cả khi là trigger | Như bên trái | Bỏ ranh giới, về `Composing` | Commit, phím tới app, `Idle` | Lật từ pending, nhãn `Correction`, sticky mode |
| `Raw` | Đi thẳng | Đi thẳng (trừ `command_end_chars`) | Đi thẳng | Đi thẳng, ở lại `Raw` | Phím trong `command_end_keycodes` hoặc phím tắt: đi thẳng, về `Idle` | Chỉ đổi sticky mode, không nhãn |

Mọi lần commit theo thứ tự `insertText` rồi phím tới app (`consumed = false`), nên Return, Tab hay Cmd-C vẫn tác dụng sau khi từ đã vào app.
Các đường commit khác: `commitComposition` (click chuột, kèm `nimk_note_navigation` thoát `Raw`), `deactivateServer`, đổi focus, đổi hoặc reset model, shutdown, và timer rảnh (`idle_commit_secs`, mặc định 1.5 s, theo nhóm app; Swift gọi `nimk_commit_pending`).

Ví dụ:

| Gõ | Kết quả |
| --- | --- |
| `has.` | marked text `has.`; nháy Shift thành `há.` |
| `Hello, world` | `,` giữ `Hello,` pending; space commit `Hello,` và đi thẳng tới app; `world` là từ mới |
| `ls -la⏎` | space giữ `ls ` pending; `-` commit `ls ` và đi thẳng; `la` đang gõ; Return commit `la` rồi tới app |
| `user_name` | `_` giữ `user_` pending; `n` commit `user_`, `name` là từ mới |
| `cd ~/Docs` | `~` commit `cd ` và đi thẳng; `/` từ `Idle` vào `Raw`, `Docs` đi thẳng |
| `http://x.com` | `:` giữ `http:`; `/` đầu commit `http:` và đi thẳng; `/` sau vào `Raw`, phần còn lại đi thẳng |
| `/help⏎`, `@src/main.rs`, `!ls -la` | `Raw` tới terminator: `/help` tới Return, `@src/main.rs` hết dòng, `!ls` tới space (rồi `-` đi thẳng, `la` là từ) |
| `/ ` | vào `Raw` rồi space thoát ngay, không tác dụng gì |
| `và/hoặc` | `/` giữa từ chỉ là ranh giới |

Ghi `word_commit` và nhãn của từ diễn ra lúc commit thật, nên Backspace quay lại từ không để lại dấu vết trong log và không đổi word id.

## UX theo loại app

Một quy tắc cho mọi app; chỉ `idle_commit_secs` đổi theo nhóm (`[[app_category]]`, [11](../11-config.md)).

| Loại app | Mặc định rảnh | Tình huống | Kết quả |
| --- | --- | --- | --- |
| Terminal (iTerm2, Terminal, WezTerm, Ghostty, kitty, Alacritty, Warp) | 1.0 s | `ls -la⏎` | `ls ` commit khi gõ `-`, `la` commit ở Return rồi Return tới shell |
| | | Tab hoàn thành | Từ đang gõ commit trước, rồi Tab tới shell nên shell hoàn thành đúng chuỗi đã chèn |
| | | Ctrl-C | Phím tắt: commit từ đang gõ rồi Ctrl-C tới app |
| | | ↑ (lịch sử) | Mũi tên: commit rồi tới shell; `/`, `@`, `!` đầu lệnh vào `Raw` nên đường dẫn và `!!` không bị Telex |
| IDE (VS Code, Cursor, Zed, Xcode, JetBrains) | 1.0 s | Định danh `user_name` | `user_` commit khi gõ `n`, `name` là từ mới; không còn marked text dài qua dấu gạch dưới |
| | | `foo.bar(` | `foo.` pending; `b` commit `foo.`; `bar(` pending; ký tự kế commit `bar(` |
| | | Popup hoàn thành: Enter hoặc Tab | Commit từ đang gõ trước, phím tới IDE nên popup chọn đúng mục |
| | | Shift-Enter, ⌘-Enter | Cũng commit rồi tới app |
| Trình duyệt (Chrome, Safari, Edge, Firefox) | 0.8 s | Thanh URL, ô tìm kiếm có gợi ý | Gợi ý tự điền trễ tối đa 1 ký tự (ranh giới đang pending) hoặc `idle_commit_secs`; Return điều hướng sau khi đã commit |
| | | `https://...` | `Raw` sau `//`, không Telex |
| Soạn thảo (Notes, TextEdit) | 2.0 s | Văn xuôi | Cửa sổ sửa đầy đủ: nháy Shift sửa được từ đang gõ và từ pending trước khi commit; timer dài hơn cho phép sửa sau khi ngừng gõ ngắn |

## Ngữ nghĩa hotkey

Hotkey vừa đổi mode vừa là nguồn nhãn chắc chắn; hành vi phụ thuộc trạng thái engine lúc bấm (`SwitchScope`).
Kết quả luôn thành sticky mode, và mỗi lần ghi một record `manual_switch` kèm `overlay = Some(Overlay::Manual)`.

| Tình huống | `scope` | Hành động hiển thị | Nhãn sinh ra |
| --- | --- | --- | --- |
| Đang gõ dở một từ | `in_word` | Lật rendering của từ, khoá rendering đến hết từ (`marked` đổi) | Từ hiện tại = rendering mới, `correction` weight 1.0, cho mọi bước của từ |
| Từ pending | `after_commit` | Lật rendering của từ pending trong marked text; bấm lặp lại bật tắt qua lại | Lúc commit thật: `correction` weight 1.0 theo rendering cuối; không lật thì nhãn ngầm như cũ |
| Không có từ pending, đã commit từ kể từ khi focus | `after_commit` | Chỉ đổi sticky mode | Không |
| `Raw` (đang gõ lệnh sau `/ @ !`) | `after_commit` (không có `word_id`) | Chỉ đổi sticky mode | Không |
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
| Khởi động process (`Core`) | `nimk_new` → `Engine::new(cfg, storage, session)`, rồi `nimk_set_config` với văn bản `config.toml` của người dùng ([11](11-config.md)) | Không bao giờ lỗi; snapshot hỏng hay sink lỗi thì fallback và báo qua `Engine::take_diagnostics` |
| `activateServer(_:)` | `nimk_context` → `Engine::on_context`, trễ 120 ms (debounce) | App hoặc ô nhập (`field_id`) đổi → focus id mới, model chọn prior; kích hoạt mới hoặc `deactivateServer` huỷ lần chờ; phím tới trước thì gửi ngay; caret rỗng thì dò lại sau 80 ms |
| `handle(_:client:)` | `nimk_context` (nếu đang chờ debounce, hoặc phím chữ mở đầu từ khi không có marked text), rồi `nimk_key` → `Engine::on_key` | Xử lý một phím; `event.characters` chỉ đọc với `keyDown` |
| `deactivateServer(_:)` | `nimk_commit_pending` → `Engine::commit_pending` | Không mất preedit hay từ pending khi mất focus |
| `commitComposition(_:)` | `nimk_commit_pending` → `Engine::commit_pending`, rồi `nimk_note_navigation` | Click chuột, IMK yêu cầu commit; thoát `Raw` |
| Sau mỗi phím (`IdleCommit.swift`) | `nimk_idle_commit_secs` | Timer rảnh: `idle_commit_secs` không phím khi còn marked text thì gọi `nimk_commit_pending`; mỗi phím đặt lại, commit và deactivate huỷ |
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
| `IdleCommit.swift` | Timer commit khi rảnh, dựng trên `Debouncer` (test: `make swift-test`) |
| `Overlay.swift` | Overlay và status item |
| `NeuralIMK-Bridging.h` | Import `nimk_ffi.h` cho `swiftc -import-objc-header` |

Quy tắc FFI:

- Mỗi hàm `extern "C"` bọc `catch_unwind`; lỗi → `consumed = false` để phím đi thẳng vào app.
- Chuỗi trong `NimkOut` do Rust sở hữu, sống đến lần gọi kế tiếp; Swift copy ngay sang `String`.
- `Engine` không `Sync`; chỉ gọi từ main thread của IMK.

Chưa kiểm chứng trên máy thật (cần cài IME): IME có hiện trong Input Sources, đường phím → marked → commit, status item từ `LSBackgroundOnly`, overlay trên màn hình phụ, độ ổn định của `field_id`, và marked text kéo dài sau dấu cách theo app ([09](09-app-compat.md)).
