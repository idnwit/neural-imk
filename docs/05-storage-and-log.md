# Storage, event log & replay

> **Trạng thái:** stable · **Cập nhật:** 2026-11-08 · **Code:** `crates/nimk-storage`, `crates/nimk-types/src/event.rs`, `crates/nimk-replay`, `crates/nimk-synth/src/log.rs`

Mọi dữ liệu bền (event log, model snapshot, config) đi qua trait `Storage` và chỉ thấy bytes.
PoC dùng `PlainFsStorage`; mã hoá sau này là một decorator, không sửa code gọi ([ADR 0004](adr/0004-storage-behind-trait.md)).

## Trait

Serialize (serde JSON) nằm ở lớp trên (`nimk-engine::logger`); `nimk-storage` không phụ thuộc crate `nimk-*` nào, kể cả `nimk-types`.

```rust
pub struct SegmentId(pub String);

pub trait EventSink: Send {
    fn append(&mut self, record: &[u8]) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

pub type RecordIter<'a> = Box<dyn Iterator<Item = io::Result<Vec<u8>>> + 'a>;

pub trait EventSource {
    fn segments(&self) -> io::Result<Vec<SegmentId>>;
    fn read_segment(&self, id: &SegmentId) -> io::Result<RecordIter<'_>>;
}

pub trait BlobStore: Send + Sync {
    fn put(&self, key: &str, bytes: &[u8]) -> io::Result<()>;
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>>;
    fn list(&self, prefix: &str) -> io::Result<Vec<String>>;
}

pub trait Storage: Send + Sync {
    fn event_sink(&self, session: &str) -> io::Result<Box<dyn EventSink>>;
    fn event_source(&self) -> io::Result<Box<dyn EventSource + '_>>;
    fn blobs(&self) -> &dyn BlobStore;
}
```

| Hợp đồng | Nội dung |
| --- | --- |
| `event_sink(session)` | Mở writer cho một session; sink được nhúng `session` vào tên segment |
| `SegmentId` | Sink tự đặt tên và xoay segment, ví dụ `2026-11-08_<session>`; thứ tự chuỗi = thứ tự thời gian |
| `append` | Một record đã serialize, không có newline cuối |
| `read_segment` | Trả record theo thứ tự append |
| `put` | Atomic: reader thấy bản cũ hoặc bản mới, không bao giờ lẫn (tmp + rename) |
| `get` | `Ok(None)` khi không có key |
| `list` | Key bắt đầu bằng `prefix`, đã sắp xếp |
| Kiểm tra đầu vào | Chung cho mọi backend: `session` đổi ký tự lạ thành `_` (rỗng → lỗi); `SegmentId` phải là `[A-Za-z0-9_-]+`; record chứa `\n` → lỗi; blob key gồm các đoạn nối bằng `/`, mỗi đoạn không rỗng, không bắt đầu bằng `.`, không chứa `\` hay NUL. Vi phạm → `io::ErrorKind::InvalidInput` |

## Implementation

| Kiểu | Dùng cho | Ghi chú |
| --- | --- | --- |
| `PlainFsStorage::open(root)` | IME thật | Tạo `root/events` và `root/blobs` nếu chưa có |
| `MemStorage::new()` | Unit test, replay | Clone dùng chung dữ liệu |
| `EncryptedStorage<S: Storage>` | Sau PoC | Decorator mã hoá từng record/blob rồi chuyển cho `S` |

Layout của `PlainFsStorage` với root `~/Library/Application Support/NeuralIMK/`:

```text
~/Library/Application Support/NeuralIMK/
  events/
    2026-11-08_<session>.jsonl   # one segment per day and session, one Record per line
    2026-11-09_<session>.jsonl
  blobs/
    model/<kind>/latest     # model snapshot, <kind> = linear_action | gru_action
```

Thêm mã hoá chỉ là bọc thêm một lớp; engine vẫn cầm `Arc<dyn Storage>`.

```text
+-------------+   Arc<dyn Storage>   +-----------------------+   bytes   +----------------+
| nimk-engine | -------------------> | EncryptedStorage<S>   | --------> | PlainFsStorage |
+-------------+                      | encrypt / decrypt     |           | events/, blobs/|
                                     +-----------------------+           +----------------+
```

Decorator thấy đúng một record hoặc một blob mỗi lần gọi, đủ để mã hoá độc lập từng đơn vị.

## Schema sự kiện

Mỗi dòng log là một `Record` JSON; trường chung `v`, `seq`, `t`, `session`, `type`, payload phẳng cạnh nhau (`#[serde(flatten)]`).
`SCHEMA_VERSION = 1`; đổi không tương thích phải tăng số và cập nhật bảng này.

| Trường chung | Kiểu | Nghĩa |
| --- | --- | --- |
| `v` | `u32` | Phiên bản schema |
| `seq` | `u64` | Số thứ tự trong session, từ 0; sắp xếp record theo `(session, seq)` |
| `t` | `f64` | Giây từ lúc boot, lấy từ event kích hoạt (`NSEvent.timestamp`); không duy nhất, reset khi reboot |
| `session` | `String` | Id ngẫu nhiên của lần chạy process IME |
| `type` | tag | Tên biến thể `Event`, `snake_case` |

| `type` | Trường riêng | Ghi khi |
| --- | --- | --- |
| `session_start` | `wall_unix_ms`, `app_version`, `config_json` (`EngineConfig` dạng JSON), `policy_id` | Record đầu tiên của mỗi session |
| `context` | `bundle_id`, `field_id`, `caret` (`{x, y}` hoặc `null`) | `activateServer`, đổi app/ô nhập, phím đầu của từ |
| `key` | `key` (`KeyEvent`), `word_id` (`u64` hoặc `null`) | Mỗi phím IME nhận |
| `decision` | `target`, `at_key` (số phím của từ sau action, 0 cho prior), `policy_id`, `p_vi` (`null` khi `teach` hoặc giá trị không hữu hạn), `rule` (luôn `null` ở log mới; `invalid_vi`/`no_telex` chỉ có trong log cũ), `chosen`, `edit` | Mỗi lần model được hỏi (mỗi phím không phải hotkey, và prior lúc focus) |
| `manual_switch` | `scope` (`in_word`/`after_commit`/`before_typing`), `word_id`, `to` | Hotkey (double Shift hoặc Ctrl+Space) |
| `word_commit` | `word_id`, `raw`, `vi`, `en`, `mode`, `boundary` | Commit |
| `label` | `target`, `label`, `weight`, `source` (`implicit`/`correction`) | Nhãn chốt; sửa ngược muộn ghi thêm nhãn `correction` mới cho cùng `target` |
| `model_update` | `policy_id`, `n_samples`, `loss` | Trainer cập nhật |

`target` là `Target` tag `kind`/`id`: `{"kind":"word","id":7}` hoặc `{"kind":"focus","id":2}`.
`edit` là `EditAction` dạng `snake_case` (`append`, `pop`, `commit`, `pass`); trường có `#[serde(default)]` nên log cũ không có nó vẫn đọc được, và prior lúc focus ghi `null`.
Giá trị enum: `Lang` = `"en"`/`"vi"`; `KeyKind` = `"key_down"`/`"flags_changed"`; `Boundary` = `space`, `return`, `tab`, `punct`, `navigation`, `shortcut`, `external`; `mods` là số bit `NSEvent.ModifierFlags`.

Ví dụ một từ `has` gõ trong VS Code (rút gọn, mỗi record một dòng):

```json
{"v":1,"seq":0,"t":5120.0,"session":"4f1c9a","type":"session_start","wall_unix_ms":1794111700000,"app_version":"0.1.0","config_json":"{...}","policy_id":"gru_action@v1"}
{"v":1,"seq":1,"t":5123.4,"session":"4f1c9a","type":"context","bundle_id":"com.microsoft.VSCode","field_id":91823,"caret":{"x":0.31,"y":0.62}}
{"v":1,"seq":2,"t":5123.4,"session":"4f1c9a","type":"decision","target":{"kind":"focus","id":0},"at_key":0,"policy_id":"gru_action@v1","p_vi":0.41,"rule":null,"chosen":"en","edit":null}
{"v":1,"seq":3,"t":5123.41,"session":"4f1c9a","type":"key","key":{"kind":"key_down","keycode":4,"chars":"h","mods":0,"timestamp":5123.41},"word_id":7}
{"v":1,"seq":6,"t":5123.62,"session":"4f1c9a","type":"decision","target":{"kind":"word","id":7},"at_key":3,"policy_id":"gru_action@v1","p_vi":0.12,"rule":null,"chosen":"en","edit":"append"}
{"v":1,"seq":8,"t":5123.7,"session":"4f1c9a","type":"word_commit","word_id":7,"raw":"has","vi":"há","en":"has","mode":"en","boundary":"space"}
{"v":1,"seq":15,"t":5126.1,"session":"4f1c9a","type":"label","target":{"kind":"word","id":7},"label":"en","weight":0.3,"source":"implicit"}
{"v":1,"seq":16,"t":5126.11,"session":"4f1c9a","type":"model_update","policy_id":"gru_action@v1","n_samples":3,"loss":0.13}
```

Hotkey và một quyết định `Commit` (rendering tại bước commit):

```json
{"v":1,"seq":30,"t":5130.02,"session":"4f1c9a","type":"manual_switch","scope":"in_word","word_id":9,"to":"vi"}
{"v":1,"seq":41,"t":5131.5,"session":"4f1c9a","type":"decision","target":{"kind":"word","id":10},"at_key":6,"policy_id":"gru_action@v1","p_vi":0.03,"rule":null,"chosen":"en","edit":"commit"}
```

Log đủ để dựng lại nguyên phiên gõ: replay dựng engine từ `session_start`, chỉ dùng `context` + `key` làm input và `label` làm đáp án; `decision.edit` của log gốc chỉ dùng để tính `edit_acc`.

## Replay harness

`nimk-replay` cho một hoặc nhiều action model quyết định lại trên log và xuất metric.

```sh
nimk-replay --data ~/Library/Application\ Support/NeuralIMK \
            --model linear_action,gru_action --from 2026-11-01 \
            --init /path/to/pretrained --report report.json
```

| Cờ | Nội dung |
| --- | --- |
| `--data DIR` | Bắt buộc; root `PlainFsStorage` (chứa `events/`) |
| `--model LIST` | Danh sách `linear_action`, `gru_action` (phẩy ngăn cách); mặc định cả hai |
| `--from PREFIX` | Chỉ segment có id từ prefix này trở đi |
| `--report FILE` | Ghi `report.json` |
| `--seed N` | Ghi đè `EngineConfig::seed` của mọi session |
| `--init DIR` | Nạp blob `model/<kind>/latest` của `DIR` (file `DIR/blobs/model/<kind>/latest`) làm model khởi đầu (ví dụ đầu ra của `nimk-synth pretrain`); thiếu thì model bắt đầu chưa train và báo cảnh báo |
| `--frozen` | Không đưa nhãn vào engine, model không học online; chỉ dùng nhãn làm đáp án đo |

| Cơ chế | Nội dung |
| --- | --- |
| Dựng engine | Mỗi session dựng lại từ `config_json` của `session_start` (lỗi parse → mặc định kèm cảnh báo), ghi đè `policy` theo `--model`, `hotkeys = false`, `Threading::Inline` |
| Nạp event | `context` → `on_context`, `key` → `on_key`, `label` → `Engine::inject_label` đúng vị trí, `word_commit` có boundary `external` → `commit_pending` |
| Nhãn | Record `label` trong log gốc là ground truth (nhãn cuối của mỗi target thắng) |
| Restart | Một `MemStorage` cho mỗi model sống xuyên session + `Engine::new` mỗi session, nên nhịp snapshot giống production |
| Prequential | Luôn như vậy: engine quyết định tại commit trước, rồi mới học nhãn; không có cờ `--online` |
| Đọc log | Dòng hỏng hoặc không phải schema v1 bị bỏ qua và đếm; session theo thứ tự xuất hiện đầu tiên, record theo `seq` |
| Output | Bảng trên stdout (một dòng `ALL` mỗi model, rồi một dòng mỗi bundle id) và `report.json` kèm metadata (50 cảnh báo đầu) |

`Engine::inject_label` cũng là cửa nạp nhãn cho log synthetic:

```rust
pub fn inject_label(&mut self, target: Target, label: Lang, weight: f32, source: LabelSource);
```

### Log synthetic

`nimk_synth::write_log` ghi script synthetic thành event log thường để replay benchmark model khi chưa có log thật ([ADR 0006](adr/0006-synthetic-pretraining.md)).

```rust
pub fn write_log(scripts: &[Script], storage: &dyn Storage) -> io::Result<()>;
```

| Record | Nội dung |
| --- | --- |
| `session_start` | Một mỗi script; `session` = `synth-<seed>-<n>`, `policy_id = "synthetic"`, config mặc định với `hotkeys = true` |
| `context` | Một, `bundle_id = "synthetic"` |
| `key` | Toàn bộ phím, gồm ranh giới; không có hotkey |
| `label` | Một mỗi từ ngay sau phím ranh giới kết từ, `source = correction`, `weight = 1.0` |

Phím ranh giới kết từ N mang `word_id = Some(N)` như engine; các phím ranh giới sau đó không có `word_id`.
`write_log` không ghi `decision`, nên `edit_acc` là `-` khi replay log synthetic: metric này cần log IME thật có `decision.edit`.
Vì mọi từ là `correction`, replay trên log synthetic trả chi phí cửa sổ correction cho từng từ; log synthetic phải để ở storage riêng, trộn với log thật sẽ làm sai metric manual switch.

Mặc định `p_typo = 0`. Với `p_typo > 0`, script có Backspace và `write_log` lệch word id so với engine (walk của nó coi Backspace là phím kết từ), nên chỉ dùng `p_typo > 0` cho `pretrain` (qua `Script::targets`), không dùng cho `write_log`.

## Metric

Metric chính là manual switch / 1000 từ; các metric khác giải thích vì sao nó lên hay xuống.
Cột stdout: `model scope words labelled acc amb_acc auto_sw auto_prec manual/1k prior_acc edit_acc`.

| Metric | Định nghĩa (`nimk-replay/src/{metrics,report}.rs`) | Mục tiêu |
| --- | --- | --- |
| `manual/1k` | `(labelled - correct) * 1000 / words`: mỗi từ sai sẽ cần một lần sửa tay | Chính, tiến về 0 |
| `acc` | Từ commit có nhãn mà `mode` tại commit bằng nhãn, chia số từ có nhãn | Tham khảo |
| `amb_acc` | Như `acc` nhưng chỉ từ mập mờ: bản Telex khác `raw` (`vi != raw`), tức là lựa chọn có ý nghĩa | Tham khảo, khó hơn `acc` |
| `auto_sw` | Số quyết định replay làm đổi mode | Tham khảo |
| `auto_prec` | Tỉ lệ `auto_sw` có nhãn mà `chosen` bằng nhãn | > 95% |
| `prior_acc` | Tỉ lệ prior lúc focus (có nhãn) đúng | Tham khảo theo app |
| `edit_acc` | Edit của quyết định replay bằng edit của log gốc cho cùng `(target, at_key)`; cặp lặp được ghép theo thứ tự; bỏ qua quyết định không có `edit` gốc | Tiến về 100% |
| Latency p50/p99 | Thời gian `nimk_key` trong process thật; replay không đo | p99 < 2 ms |

Một tỉ lệ có mẫu số 0 hiển thị `-`. `ambiguous` nghĩa là `raw != vi` (không còn xét `valid_vi`); điều này khác định nghĩa của `pretrain`, nơi từ mập mờ còn phải là âm tiết hợp lệ ([04](04-engine-ml.md)).

## Thiên lệch của replay

Nhãn ngầm "không sửa = đúng" sinh ra dưới model cũ, nên replay chỉ là ước lượng.
Model mới chọn khác ở từ không bị sửa sẽ bị tính sai dù người dùng có thể chấp nhận; kết luận cuối dựa trên metric dùng thật (M5).
`--frozen` cho đường cơ sở "không học online" để tách phần đóng góp của online learning.
