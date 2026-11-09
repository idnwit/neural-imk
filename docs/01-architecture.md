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
| HotkeyDetect | ----> | manual_switch      | --> EngineOutput { consumed, marked?, mode }
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
| apply: Append -> push + Telex + marked | Pop -> pop | Commit -> pending | Pass     |
+-----------------------------------------------------------------------------------+
    |                                   |
    v                                   v
 EngineOutput { consumed, commit,     Decision / WordCommit / Label records,
   marked, mode, overlay }            Example -> trainer
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
| `nimk-storage` | lib | `Storage`, `EventSink`, `EventSource`, `BlobStore`; `PlainFsStorage`, `MemStorage`, `LayeredStorage`, `ReadOnlyDirBlobs`; chỉ biết bytes | — |
| `nimk-telex` | lib | `transform`, `encode`, `is_valid_syllable`; Telex là công cụ, không phải luật | `nimk-types` |
| `nimk-policy` | lib | `ImeModel`, `ImeLearner`, `ModelKind`, `build_action`, `decide`, `encode`; `LinearAction`, `GruAction` (backend `burn`) | `nimk-types` |
| `nimk-engine` | lib | `Engine`: composer, hotkey, labeler, trainer, logger, từ pending ([02](02-keyboard-io.md)) | `types`, `storage`, `telex`, `policy` |
| `nimk-ffi` | staticlib | C ABI cho Swift; crate duy nhất có `extern "C"`/`unsafe` | `types`, `engine`, `storage` |
| `nimk-replay` | bin + lib | Replay log qua model, xuất metric ([05](05-storage-and-log.md)) | `types`, `storage`, `policy`, `engine` |
| `nimk-synth` | lib + bin | Dữ liệu gõ synthetic, `write_log`, `pretrain` ([ADR 0006](adr/0006-synthetic-pretraining.md), [ADR 0007](adr/0007-neural-action-model.md)) | `types`, `telex`, `storage`, `policy`, `engine` |
| `macos/NeuralIMK` | Swift | IMK shim, overlay, status item, `Debouncer` ([09](09-app-compat.md)) | `libnimk_ffi.a` qua header |

Quy tắc tầng được khoá bằng `arch-lint.toml` và chạy trong `make check`; bảng đầy đủ ở [08](08-coding-standard.md#contract-arch-lint).

## Module trong `nimk-engine`

Năm module nội bộ độc lập với nhau; `engine.rs` nối chúng, và hai file con `engine/flow.rs`, `engine/switch.rs` giữ phần logic của `Engine`.

```text
                      +-------------------------------------------+
                      |                engine.rs                  |
                      | Engine, EngineOutput, Overlay             |
                      +-------------------------------------------+
                        |  engine/flow.rs     engine/switch.rs  |
                        |  (per-key loop)     (hotkey, flip)     |
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
| `engine/switch.rs` | `manual_switch`: hotkey theo thời điểm, lật từ pending |

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
    pub fn switch_model(&mut self, kind: ModelKind, timestamp: f64) -> EngineOutput;
    pub fn model_kind(&self) -> ModelKind;
    pub fn has_checkpoint(&self, kind: ModelKind) -> bool;
    pub fn checkpoint_source(&self, kind: ModelKind) -> CheckpointSource; // User | Bundled | None
    pub fn reset_model(&mut self, timestamp: f64) -> EngineOutput;
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
}

pub struct SessionInfo {
    pub id: String,
    pub wall_unix_ms: u64,
    pub app_version: String,
}
```

| Hành vi | Nội dung |
| --- | --- |
| `new` không bao giờ lỗi | Đọc blob `settings/model` (nếu hợp lệ thì ghi đè `cfg.policy`), rồi `model/<kind>/latest` (`linear_action` hoặc `gru_action`); thiếu hoặc hỏng → model mới từ `cfg.policy` (mặc định `GruAction`); sink lỗi → null sink; cả hai báo qua `take_diagnostics` |
| `with_model` | Không nạp snapshot; dùng cho benchmark, pretrain và test |
| `new` ghi `session_start` | Record đầu tiên chứa `EngineConfig` dạng JSON để replay dựng lại |
| `on_key` không panic | Lỗi nội bộ → `consumed = false`, phím tới app |
| `teach` | Như `on_key` nhưng áp `target` thay cho quyết định của model và xếp một `Example`; model không được gọi, `Decision.p_vi = None`. Edit bị mask chặn rơi về `Pass`; hotkey vẫn được nhận. Dùng cho pretrain ([04](04-engine-ml.md)) |
| `on_context` | Đổi app hoặc `field_id` → commit từ đang gõ (`Boundary::External`), id focus mới, model chọn prior |
| `switch_model` | Commit từ đang gõ như `commit_pending`, chốt nhãn treo, lưu model hiện tại vào checkpoint riêng, nạp `model/<kind>/latest` của model mới (thiếu/hỏng → model mới + diagnostic), publish snapshot, ghi `settings/model`, log `model_switch`. Cùng kind chỉ commit. Chạy được với cả `Background` và `Inline` ([04](04-engine-ml.md#chọn-model)) |
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

`nimk-ffi` xuất 16 hàm; header sinh bằng `make header` (cbindgen) ra `target/nimk_ffi.h`, bản dưới đây suy ra từ `exports.rs` và `abi.rs`.

```c
// minh hoạ từ exports.rs; header thật do cbindgen sinh
NimkCore   *nimk_new(const char *data_dir, const char *models_dir, const char *app_version);
void        nimk_free(NimkCore *core);
bool        nimk_shutdown(NimkCore *core);
void        nimk_context(NimkCore *core, const NimkCtx *ctx, NimkOut *out);
void        nimk_key(NimkCore *core, const NimkKey *key, NimkOut *out);
void        nimk_commit_pending(NimkCore *core, double timestamp, NimkOut *out);
bool        nimk_set_model(NimkCore *core, uint8_t kind, double timestamp, NimkOut *out);
uint8_t     nimk_model(const NimkCore *core);
bool        nimk_reset_model(NimkCore *core, double timestamp, NimkOut *out);
uint8_t     nimk_checkpoint_source(const NimkCore *core, uint8_t kind);
uint8_t     nimk_mode(const NimkCore *core);
const char *nimk_policy_id(NimkCore *core);
const char *nimk_take_diagnostics(NimkCore *core);
uint32_t    nimk_set_config(NimkCore *core, const char *config_toml);
double      nimk_idle_commit_secs(const NimkCore *core);
void        nimk_note_navigation(NimkCore *core);
```

`nimk_set_config` nhận văn bản `config.toml` của người dùng (hoặc null), ghép lên mặc định nhúng và trả số giá trị bị bỏ (chi tiết ở `nimk_take_diagnostics`, [11](11-config.md)); `nimk_idle_commit_secs` cho timer rảnh của Swift (0 = tắt, đổi theo nhóm app của focus); `nimk_note_navigation` báo con trỏ đổi do chuột (thoát `Raw`). Hợp đồng `NimkOut` không đổi: `commit` luôn được áp trước, rồi phím tới app khi `consumed = false` ([ADR 0012](adr/0012-unified-boundary-commit.md)).
`NIMK_MODEL_LINEAR` = 0 và `NIMK_MODEL_GRU` = 1 là mã `kind`; `nimk_set_model` trả `false` (và để `out` ở dạng pass-through) khi `kind` lạ hoặc core null.
`nimk_checkpoint_source` trả `NIMK_CHECKPOINT_NONE` (0), `NIMK_CHECKPOINT_BUNDLED` (1) hoặc `NIMK_CHECKPOINT_USER` (2); `nimk_reset_model` xoá checkpoint của người dùng và nạp lại model gốc.

`NimkOut` mirror `EngineOutput`. Hợp đồng đầu ra: Swift áp `commit` (`insertText`), rồi `marked` (`setMarkedText`); từ model kết thúc ở lại `marked` (từ + ranh giới) cho tới khi từ kế tiếp bắt đầu, lúc đó `commit` mang từ pending ([ADR 0011](adr/0011-pending-word-commit.md)).

| Trường `NimkOut` | Nghĩa |
| --- | --- |
| `consumed` | `false` → trả phím nguyên vẹn cho app |
| `commit`, `marked` | Null = không đổi; `marked = ""` xoá preedit; `commit` là từ đang gõ hoặc từ pending cộng ranh giới |
| `mode` | `NIMK_MODE_EN` (0), `NIMK_MODE_VI` (1), `NIMK_MODE_UNCHANGED` (255) |
| `overlay` | `NIMK_OVERLAY_NONE` (0), `NIMK_OVERLAY_AUTO` (1), `NIMK_OVERLAY_MANUAL` (2) |

Quy tắc FFI: mỗi hàm bọc `catch_unwind`, lỗi → `NimkOut::pass_through()` (`consumed = false`); chuỗi trong `NimkOut` sống đến lần gọi kế.
`nimk_new` trả null khi engine không dựng được; mọi hàm khác chấp nhận null và cho phím đi qua. Panic ghi vào diagnostics (tối đa 256 dòng).
Không mở được data dir thì dùng `MemStorage` và báo qua diagnostics.
`data_dir` do Swift lấy từ SDK (không hard-code); `models_dir` là thư mục model đóng gói trong app, null hoặc thiếu thì chạy không có model gốc và báo qua diagnostics ([ADR 0010](adr/0010-sdk-storage-and-bundled-models.md)).

### Phân tầng storage

`nimk_new` dựng `LayeredStorage(PlainFsStorage(data_dir), ReadOnlyDirBlobs(models_dir))`; engine vẫn chỉ cầm `Arc<dyn Storage>`.

```text
 Swift: FileManager                     Swift: Bundle.main.resourceURL
 Application Support/<bundle id>        Contents/Resources/models
        | data_dir                             | models_dir
        v                                      v
+-----------------------------------------------------------+
| LayeredStorage                                            |
|  events    --> PlainFsStorage (user)                      |
|  put/delete--> PlainFsStorage (user)                      |
|  get       --> user, if absent --> ReadOnlyDirBlobs       |
|  list      --> union of both, sorted                      |
+-----------------------------------------------------------+
```

Checkpoint học thêm luôn ghi vào tầng người dùng và che bản gốc; xoá nó đưa model về bản gốc.
