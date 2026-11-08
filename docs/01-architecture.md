# Kiến trúc

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `Cargo.toml`, `arch-lint.toml`, `crates/nimk-engine/src/lib.rs`

NeuralIMK là một process `NeuralIMK.app`: Swift shim mỏng nói chuyện với IMK/AppKit, mọi logic nằm trong Rust core link tĩnh qua C ABI.
Hot path chỉ đọc snapshot policy; trainer và log writer chạy nền nên phím không bao giờ chờ I/O hay training.

## Process và luồng

Một process, ba luồng Rust; chỉ module `trainer` và `logger` được tạo thread.

| Luồng | Chạy gì | Giao tiếp |
| --- | --- | --- |
| Main (IMK, gọi từ Swift) | `Engine::on_key` / `on_context` / `commit_pending`: hotkey → composer → policy | Đọc `Arc<dyn Policy>` hiện tại; gửi record và sample qua channel |
| Trainer | Replay buffer, cửa sổ correction, `Learner::update`, snapshot | Gửi `Arc<dyn Policy>` mới qua mpsc; engine `try_recv` ở ranh giới từ |
| Log writer | Serialize `Record` → `EventSink::append` | Nhận record qua channel |

Với `Threading::Inline` (test, replay, synth), cả ba chạy trên luồng gọi, kết quả tất định.

```text
+------------+  C ABI   +----------------------------+  Record  +--------+  bytes  +---------+
| Swift shim | -------> | main thread: Engine        | -------> | logger | ------> | Storage |
|   (IMK)    | <------- | hotkey > composer > policy |          +--------+         +---------+
+------------+ NimkOut  +----------------------------+                                 ^
                           |                   ^                                       |
                    Sample |                   | Arc<dyn Policy> (mpsc, try_recv)      |
                           v                   |                                       |
                        +-------------------------+     snapshot (Learner::save)       |
                        |         trainer         | -----------------------------------+
                        +-------------------------+
```

Mũi tên duy nhất đi vào main thread từ nền là snapshot policy, và main thread chỉ nhận nó ở ranh giới từ.

## Crate và tầng

Workspace `crates/*`, edition 2024, phụ thuộc chỉ đi một chiều từ trên xuống.

```text
 +--------------+  +--------------+  +--------------+      +-------------------------+
 |  nimk-ffi    |  | nimk-replay  |  | nimk-synth   |      | macos/NeuralIMK (Swift) |
 |  staticlib   |  |  bin         |  |  lib         |      | links libnimk_ffi.a     |
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
        +------------+ +-------------+ |  (true leaf,   |
                   |       |           |  no nimk deps) |
                   v       v           +----------------+
                 +-----------------+
                 |   nimk-types    |
                 +-----------------+
```

Tầng trên được bỏ qua tầng giữa (ví dụ `nimk-replay` dùng thẳng `nimk-policy`), nhưng không bao giờ phụ thuộc ngược lên.

| Crate | Loại | Trách nhiệm | Dependency nội bộ (`Cargo.toml`) |
| --- | --- | --- | --- |
| `nimk-types` | lib | `Lang`, `KeyEvent`, `AppContext`, `CaretRel`, `DecisionCtx`, `Features`, `Sample`, schema sự kiện | — |
| `nimk-storage` | lib | `Storage`, `EventSink`, `EventSource`, `BlobStore`; `PlainFsStorage`, `MemStorage`; chỉ biết bytes | — |
| `nimk-telex` | lib | `transform`, `encode`, `is_valid_syllable` | `nimk-types` |
| `nimk-policy` | lib | `FeatureExtractor`, `Policy`, `Learner`, `Model`; `Baseline`, `Linear`, `TinyMlp`; `HashedExtractor` | `nimk-types` |
| `nimk-engine` | lib | `Engine`: composer, hotkey, labeler, trainer, logger | `types`, `storage`, `telex`, `policy` |
| `nimk-ffi` | staticlib | C ABI cho Swift; crate duy nhất có `extern "C"`/`unsafe` | `types`, `engine`, `storage` |
| `nimk-replay` | bin | Replay log, so sánh policy, xuất metric | `types`, `storage`, `policy`, `engine` |
| `nimk-synth` | lib | Dữ liệu gõ VI/EN synthetic, ghi log hoặc pretrain ([ADR 0006](adr/0006-synthetic-pretraining.md)) | `types`, `telex`, `storage`, `policy`, `engine` |
| `macos/NeuralIMK` | Swift | IMK shim, overlay, status item (M2) | `libnimk_ffi.a` qua header |

Quy tắc tầng được khoá bằng `arch-lint.toml` và chạy trong `make check`; bảng đầy đủ ở [08](08-coding-standard.md#contract-arch-lint).

## Module trong `nimk-engine`

Năm module nội bộ độc lập với nhau; chỉ `engine.rs` nối chúng lại.

```text
                      +-------------------------------------------+
                      |                engine.rs                  |
                      | Engine, EngineOutput, Overlay, SessionInfo |
                      +-------------------------------------------+
                        |        |         |          |         |
                        v        v         v          v         v
                  +--------+ +-------+ +--------+ +--------+ +--------+
                  |composer| |hotkey | |labeler | |trainer | |logger  |
                  +--------+ +-------+ +--------+ +--------+ +--------+
                                                   thread     thread
```

Module con không `use` nhau (`eng-*-independent`); thread chỉ được tạo trong `trainer` và `logger` (`hot-path-no-io`).

| Module | Trách nhiệm |
| --- | --- |
| `composer` | `raw` của từ hiện tại, hai bản VI/EN, ranh giới từ |
| `hotkey` | State machine Shift tap / Ctrl+Space ([02](02-keyboard-io.md)) |
| `labeler` | Commit + manual switch → `Sample` sau cửa sổ sửa ([04](04-engine-ml.md)) |
| `trainer` | Replay buffer, cửa sổ correction; nền hoặc inline |
| `logger` | Serialize `Record` vào `EventSink` |

## API công khai của `nimk-engine`

`Engine` là cửa duy nhất mà FFI, replay và synth dùng; không `Sync`, chỉ gọi từ một luồng.

```rust
impl Engine {
    pub fn new(cfg: EngineConfig, storage: Arc<dyn Storage>, session: SessionInfo) -> Self;
    pub fn with_model(cfg: EngineConfig, storage: Arc<dyn Storage>, session: SessionInfo, model: Model) -> Self;
    pub fn on_context(&mut self, ctx: &AppContext) -> EngineOutput;
    pub fn on_key(&mut self, key: &KeyEvent) -> EngineOutput;
    pub fn commit_pending(&mut self, timestamp: f64) -> EngineOutput;
    pub fn inject_label(&mut self, target: Target, label: Lang, weight: f32, source: LabelSource);
    pub fn mode(&self) -> Lang;
    pub fn policy_id(&self) -> Arc<str>;
    pub fn take_diagnostics(&mut self) -> Vec<String>;
    pub fn shutdown(self) -> Result<(), EngineError>;
}

pub struct SessionInfo {
    pub id: String,
    pub wall_unix_ms: u64,
    pub app_version: String,
}
```

| Hành vi | Nội dung |
| --- | --- |
| `new` không bao giờ lỗi | Snapshot thiếu/hỏng → model mới từ `cfg.policy`; sink lỗi → null sink; cả hai báo qua `take_diagnostics` |
| `new` ghi `session_start` | Record đầu tiên chứa `EngineConfig` dạng JSON để replay dựng lại |
| `on_key` không panic | Lỗi nội bộ → `consumed = false`, phím tới app |
| `shutdown` | Chốt nhãn còn treo, flush log, lưu snapshot; lỗi duy nhất là `EngineError::Storage` |

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

## Hợp đồng C ABI

`nimk-ffi` mới là skeleton; hợp đồng dưới đây là bản thiết kế, phải sửa theo `Engine` trước M2 (minh hoạ).

```c
// minh hoạ — header thật sinh bằng cbindgen ra target/nimk_ffi.h
NimkCore *nimk_new(const char *data_dir);
void      nimk_free(NimkCore *);
void      nimk_context(NimkCore *, const NimkCtx *);
void      nimk_key(NimkCore *, const NimkKey *, NimkOut *out);
void      nimk_commit_pending(NimkCore *, NimkOut *out);
```

| Điểm lệch với `Engine` cần chốt | Lý do |
| --- | --- |
| `NimkCtx` thiếu `field_id`, `timestamp` | `AppContext` có cả hai |
| `nimk_commit_pending` thiếu `timestamp` | `Engine::commit_pending(timestamp: f64)` |
| `NimkOut.show_overlay: bool` | `EngineOutput::overlay` phân biệt `Auto`/`Manual` ([06](06-gui.md)) |
| `NimkOut.mode: uint8_t` luôn có giá trị | `EngineOutput::mode = None` nghĩa là không đổi |
| `nimk_new` chỉ nhận `data_dir` | `Engine::new` cần `SessionInfo` (FFI tự dựng id, wall clock, version) |

Quy tắc FFI giữ nguyên: mỗi hàm bọc `catch_unwind`, lỗi → `consumed = false`; chuỗi trong `NimkOut` sống đến lần gọi kế.
