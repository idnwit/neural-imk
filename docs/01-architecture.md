# Kiến trúc

> **Trạng thái:** stable · **Cập nhật:** 2026-11-08 · **Code:** `Cargo.toml`, `arch-lint.toml`, `crates/nimk-engine/src/{lib,engine}.rs`, `crates/nimk-ffi/src`

NeuralIMK là một process `NeuralIMK.app`: Swift shim mỏng nói chuyện với IMK/AppKit, mọi logic nằm trong Rust core link tĩnh qua C ABI.
Hot path chỉ đọc snapshot model; trainer và log writer chạy nền nên phím không bao giờ chờ I/O hay training.

## Process và luồng

Một process, ba luồng Rust; chỉ module `trainer` và `logger` được tạo thread.

| Luồng | Chạy gì | Giao tiếp |
| --- | --- | --- |
| Main (IMK, gọi từ Swift) | `Engine::on_key` / `on_context` / `commit_pending` / `teach`: hotkey → composer → `ImeModel::score` → áp action | Đọc `Arc<dyn ImeModel>` hiện tại; gửi `Record` và `Example` qua channel |
| Trainer | Replay buffer, cửa sổ correction, `ImeLearner::update`, lưu snapshot | Gửi `Arc<dyn ImeModel>` mới qua mpsc; engine `try_recv` ở ranh giới từ |
| Log writer | Serialize `Record` → `EventSink::append` | Nhận record qua channel |

Với `Threading::Inline` (test, replay, synth), cả ba chạy trên luồng gọi, kết quả tất định.

```text
+------------+  C ABI   +-----------------------------+ Record  +--------+ bytes +---------+
| Swift shim | -------> | main thread: Engine         | ------> | logger | ----> | Storage |
|   (IMK)    | <------- | hotkey > composer > model   |         +--------+       +---------+
+------------+ NimkOut  +-----------------------------+                               ^
                           |                    ^                                     |
                  Example  |                    | Arc<dyn ImeModel> (mpsc, try_recv)  |
                  (batch)  v                    |                                     |
                        +-------------------------+   snapshot (ImeLearner::save)     |
                        |         trainer         | ----------------------------------+
                        +-------------------------+
```

Mũi tên duy nhất đi vào main thread từ nền là snapshot model, và main thread chỉ nhận nó ở ranh giới từ.

## Vòng lặp action

Mỗi phím đi qua một vòng cố định: engine dựng `Observation`, model trả điểm, engine áp `Action` một cách cơ học.

```text
 KeyEvent
    |
    v
+--------------+ hit   +--------------------+
| HotkeyDetect | ----> | manual_switch      | --> EngineOutput { consumed, replace?, marked?, mode }
+--------------+       +--------------------+
    | no hit
    v
+-----------------+  Observation   +-------------------+  ActionScores  +----------------+
| flow: mask_for, | -------------> | ImeModel::score   | -------------> | decide(mask)   |
| observation()   |                | (LinearAction or  |                | + hysteresis   |
+-----------------+                |  GruAction)       |                +----------------+
                                   +-------------------+                        |
                                                                           Action v
+-----------------------------------------------------------------------------------+
| apply: Append -> push + Telex + marked | Pop -> pop | Commit -> commit_word | Pass   |
+-----------------------------------------------------------------------------------+
    |                                   |
    v                                   v
 EngineOutput { consumed, commit,     Decision / WordCommit / Label records,
   marked, mode, overlay, replace }   Example -> trainer
```

Hotkey đứng trước model và không bao giờ vào `Observation`; mọi lựa chọn còn lại do model quyết định, engine chỉ che action vô nghĩa bằng `ActionMask`.

## Crate và tầng

Workspace `crates/*` gồm 8 crate, edition 2024, phụ thuộc chỉ đi một chiều từ trên xuống.

```text
 +--------------+  +--------------+  +--------------+      +-------------------------+
 |  nimk-ffi    |  | nimk-replay  |  | nimk-synth   |      | macos/NeuralIMK (Swift) |
 |  staticlib   |  |  bin         |  |  lib + bin   |      | links libnimk_ffi.a     |
 +--------------+  +--------------+  +--------------+      +-------------------------+
        |                 |                 |                    |  C ABI (nimk_ffi.h)
        |                 |                 |    <---------------+  calls nimk-ffi only
        +-----------------+-----------------+
                          |
                          v
                 +-----------------+
                 |   nimk-engine   |
                 +-----------------+
                   |       |      \
                   v       v       v
        +------------+ +-------------+ +----------------+
        | nimk-telex | | nimk-policy | |  nimk-storage  |
        +------------+ |  (burn)     | |  (true leaf,   |
                   |   +-------------+ |  no nimk deps) |
                   v       v           +----------------+
                 +-----------------+
                 |   nimk-types    |
                 +-----------------+
```

Tầng trên được bỏ qua tầng giữa (ví dụ `nimk-synth` dùng thẳng `nimk-policy`), nhưng không bao giờ phụ thuộc ngược lên.

| Crate | Loại | Trách nhiệm | Dependency nội bộ (`Cargo.toml`) |
| --- | --- | --- | --- |
| `nimk-types` | lib | `Lang`, `KeyEvent`, `AppContext`, `CaretRel`, hợp đồng action (`Observation`, `Action`, `EditAction`, `ActionMask`, `ActionScores`, `Example`), schema sự kiện | — |
| `nimk-storage` | lib | `Storage`, `EventSink`, `EventSource`, `BlobStore`; `PlainFsStorage`, `MemStorage`; chỉ biết bytes | — |
| `nimk-telex` | lib | `transform`, `encode`, `is_valid_syllable`; Telex là công cụ, không phải luật | `nimk-types` |
| `nimk-policy` | lib | `ImeModel`, `ImeLearner`, `ModelKind`, `build_action`, `decide`, `encode`; `LinearAction`, `GruAction` (backend `burn`) | `nimk-types` |
| `nimk-engine` | lib | `Engine`: composer, hotkey, labeler, trainer, logger, sửa ngược | `types`, `storage`, `telex`, `policy` |
| `nimk-ffi` | staticlib | C ABI cho Swift; crate duy nhất có `extern "C"`/`unsafe` | `types`, `engine`, `storage` |
| `nimk-replay` | bin + lib | Replay log qua model, xuất metric ([05](05-storage-and-log.md)) | `types`, `storage`, `policy`, `engine` |
| `nimk-synth` | lib + bin | Dữ liệu gõ synthetic, `write_log`, `pretrain` ([ADR 0006](adr/0006-synthetic-pretraining.md), [ADR 0007](adr/0007-neural-action-model.md)) | `types`, `telex`, `storage`, `policy`, `engine` |
| `macos/NeuralIMK` | Swift | IMK shim, overlay, status item, `ReplaceCompat` ([09](09-app-compat.md)) | `libnimk_ffi.a` qua header |

Quy tắc tầng được khoá bằng `arch-lint.toml` và chạy trong `make check`; bảng đầy đủ ở [08](08-coding-standard.md#contract-arch-lint).

## Module trong `nimk-engine`

Năm module nội bộ độc lập với nhau; `engine.rs` nối chúng, và hai file con `engine/flow.rs`, `engine/switch.rs` giữ phần logic của `Engine`.

```text
                      +-------------------------------------------+
                      |                engine.rs                  |
                      | Engine, EngineOutput, Replace, Overlay    |
                      +-------------------------------------------+
                        |  engine/flow.rs     engine/switch.rs  |
                        |  (per-key loop)     (hotkey + replace) |
                        v        v         v          v         v
                  +--------+ +-------+ +--------+ +--------+ +--------+
                  |composer| |hotkey | |labeler | |trainer | |logger  |
                  +--------+ +-------+ +--------+ +--------+ +--------+
                                                   thread     thread
```

Module con không `use` nhau (`eng-*-independent`); thread chỉ được tạo trong `trainer` và `logger` (`hot-path-no-io`).

| Module | Trách nhiệm |
| --- | --- |
| `composer` | Buffer `raw` của từ hiện tại, bản `vi`, word id; không biết Telex hay biên từ ([02](02-keyboard-io.md)) |
| `hotkey` | State machine double Shift / Ctrl+Space ([02](02-keyboard-io.md)) |
| `labeler` | Commit, focus prior, correction → nhãn chốt và `Example` ([04](04-engine-ml.md)) |
| `trainer` (+ `trainer/tests.rs`) | Replay buffer, cửa sổ correction, snapshot; nền hoặc inline |
| `logger` | Serialize `Record` vào `EventSink` |
| `engine/flow.rs` | Vòng lặp mỗi phím: `Observation`, gọi model, áp action, commit, nhãn |
| `engine/switch.rs` | `manual_switch`: hotkey theo thời điểm, sinh `Replace` |

## API công khai của `nimk-engine`

`Engine` là cửa duy nhất mà FFI, replay và synth dùng; không `Sync`, chỉ gọi từ một luồng.

```rust
impl Engine {
    pub fn new(cfg: EngineConfig, storage: Arc<dyn Storage>, session: SessionInfo) -> Self;
    pub fn with_model(
        cfg: EngineConfig,
        storage: Arc<dyn Storage>,
        session: SessionInfo,
        learner: Box<dyn ImeLearner>,
    ) -> Self;
    pub fn on_context(&mut self, ctx: &AppContext) -> EngineOutput;
    pub fn on_key(&mut self, key: &KeyEvent) -> EngineOutput;
    pub fn teach(
        &mut self,
        key: &KeyEvent,
        target: Action,
        edit_weight: f32,
        render_weight: f32,
    ) -> EngineOutput;
    pub fn commit_pending(&mut self, timestamp: f64) -> EngineOutput;
    pub fn inject_label(&mut self, target: Target, label: Lang, weight: f32, source: LabelSource);
    pub fn mode(&self) -> Lang;
    pub fn policy_id(&self) -> Arc<str>;
    pub fn take_diagnostics(&mut self) -> Vec<String>;
    pub fn shutdown(self) -> Result<(), EngineError>;
}

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

pub struct SessionInfo {
    pub id: String,
    pub wall_unix_ms: u64,
    pub app_version: String,
}
```

| Hành vi | Nội dung |
| --- | --- |
| `new` không bao giờ lỗi | Đọc `model/<kind>/latest` (`linear_action` hoặc `gru_action`); thiếu hoặc hỏng → model mới từ `cfg.policy` (mặc định `LinearAction`); sink lỗi → null sink; cả hai báo qua `take_diagnostics` |
| `with_model` | Không nạp snapshot; dùng cho benchmark, pretrain và test |
| `new` ghi `session_start` | Record đầu tiên chứa `EngineConfig` dạng JSON để replay dựng lại |
| `on_key` không panic | Lỗi nội bộ → `consumed = false`, phím tới app |
| `teach` | Như `on_key` nhưng áp `target` thay cho quyết định của model và xếp một `Example`; model không được gọi, `Decision.p_vi = None`. Edit bị mask chặn rơi về `Pass`; hotkey vẫn được nhận. Dùng cho pretrain ([04](04-engine-ml.md)) |
| `on_context` | Đổi app hoặc `field_id` → commit từ đang gõ (`Boundary::External`), id focus mới, model chọn prior |
| `shutdown` | Chốt nhãn còn treo, huấn luyện nốt phần dư của `teach`, flush log, lưu snapshot; lỗi duy nhất là `EngineError::Storage` |

`mode()` là sticky mode: mode mà từ kế tiếp bắt đầu, không phải rendering đang hiện của từ dở.

## Đồng hồ và tất định

Engine không đọc wall clock; đồng hồ duy nhất là `KeyEvent::timestamp` / `AppContext::timestamp` (giây từ lúc boot).
Wall clock chỉ do host cấp (`SessionInfo::wall_unix_ms`) hoặc do sink đọc để đặt tên segment.

| Ai | Được đọc wall clock? | Khoá bởi |
| --- | --- | --- |
| `nimk-types`, `nimk-telex`, `nimk-policy` | Không (cũng không I/O, thread, lock) | `pure-no-side-effects` |
| `nimk-engine` | Không (`std::time`, `std::env` bị cấm) | `engine-event-clock` |
| `nimk-storage` (sink) | Có, để đặt tên segment | — |
| `nimk-ffi` | Có, để dựng `SessionInfo` | — |

Nhờ vậy cùng log + cùng config cho ra cùng quyết định khi replay.
`GruAction` tự giữ RNG theo seed vì RNG thiết bị của `burn` là global của process ([04](04-engine-ml.md)).

## Hợp đồng C ABI

`nimk-ffi` xuất 9 hàm; header sinh bằng `make header` (cbindgen) ra `target/nimk_ffi.h`, bản dưới đây suy ra từ `exports.rs` và `abi.rs`.

```c
// minh hoạ từ exports.rs; header thật do cbindgen sinh
NimkCore   *nimk_new(const char *data_dir, const char *app_version);
void        nimk_free(NimkCore *core);
bool        nimk_shutdown(NimkCore *core);
void        nimk_context(NimkCore *core, const NimkCtx *ctx, NimkOut *out);
void        nimk_key(NimkCore *core, const NimkKey *key, NimkOut *out);
void        nimk_commit_pending(NimkCore *core, double timestamp, NimkOut *out);
uint8_t     nimk_mode(const NimkCore *core);
const char *nimk_policy_id(NimkCore *core);
const char *nimk_take_diagnostics(NimkCore *core);
```

`NimkOut` mirror `EngineOutput`; `replace_delete` và `replace_text` mang `Replace`.

| Trường `NimkOut` | Nghĩa |
| --- | --- |
| `consumed` | `false` → trả phím nguyên vẹn cho app |
| `replace_delete`, `replace_text` | Số UTF-16 unit trước con trỏ cần thay và text thay; áp trước `commit`/`marked`; `replace_text = null` là không thay |
| `commit`, `marked` | Null = không đổi; `marked = ""` xoá preedit |
| `mode` | `NIMK_MODE_EN` (0), `NIMK_MODE_VI` (1), `NIMK_MODE_UNCHANGED` (255) |
| `overlay` | `NIMK_OVERLAY_NONE` (0), `NIMK_OVERLAY_AUTO` (1), `NIMK_OVERLAY_MANUAL` (2) |

Quy tắc FFI: mỗi hàm bọc `catch_unwind`, lỗi → `NimkOut::pass_through()` (`consumed = false`); chuỗi trong `NimkOut` sống đến lần gọi kế.
`nimk_new` trả null khi engine không dựng được; mọi hàm khác chấp nhận null và cho phím đi qua. Panic ghi vào diagnostics (tối đa 256 dòng).
Không mở được data dir thì dùng `MemStorage` và báo qua diagnostics.
