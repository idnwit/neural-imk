# Storage, event log & replay

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `crates/nimk-storage`, `crates/nimk-types/src/event.rs`, `crates/nimk-replay`

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
    <key>                   # model snapshot, config, replay buffer
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
| `decision` | `target`, `at_key` (số phím trong từ, 0 cho prior), `policy_id`, `p_vi` (`null` nếu luật cứng), `rule` (`null`/`invalid_vi`/`no_telex`), `chosen` | Mỗi lần policy hoặc luật cứng được hỏi |
| `manual_switch` | `scope` (`in_word`/`after_commit`/`before_typing`), `word_id`, `to` | Hotkey |
| `word_commit` | `word_id`, `raw`, `vi`, `en`, `mode`, `boundary` | Commit |
| `label` | `target`, `label`, `weight`, `source` (`implicit`/`correction`) | Nhãn chốt |
| `model_update` | `policy_id`, `n_samples`, `loss` | Trainer cập nhật |

`target` là `Target` tag `kind`/`id`: `{"kind":"word","id":7}` hoặc `{"kind":"focus","id":2}`.
Giá trị enum: `Lang` = `"en"`/`"vi"`; `KeyKind` = `"key_down"`/`"flags_changed"`; `Boundary` = `space`, `return`, `tab`, `punct`, `navigation`, `shortcut`, `external`; `mods` là số bit `NSEvent.ModifierFlags`.

Ví dụ một từ `has` gõ trong VS Code (rút gọn, mỗi record một dòng):

```json
{"v":1,"seq":0,"t":5120.0,"session":"4f1c9a","type":"session_start","wall_unix_ms":1794111700000,"app_version":"0.1.0","config_json":"{...}","policy_id":"tiny_mlp@v3"}
{"v":1,"seq":1,"t":5123.4,"session":"4f1c9a","type":"context","bundle_id":"com.microsoft.VSCode","field_id":91823,"caret":{"x":0.31,"y":0.62}}
{"v":1,"seq":2,"t":5123.4,"session":"4f1c9a","type":"decision","target":{"kind":"focus","id":0},"at_key":0,"policy_id":"tiny_mlp@v3","p_vi":0.41,"rule":null,"chosen":"en"}
{"v":1,"seq":3,"t":5123.41,"session":"4f1c9a","type":"key","key":{"kind":"key_down","keycode":4,"chars":"h","mods":0,"timestamp":5123.41},"word_id":7}
{"v":1,"seq":6,"t":5123.62,"session":"4f1c9a","type":"decision","target":{"kind":"word","id":7},"at_key":3,"policy_id":"tiny_mlp@v3","p_vi":0.12,"rule":null,"chosen":"en"}
{"v":1,"seq":8,"t":5123.7,"session":"4f1c9a","type":"word_commit","word_id":7,"raw":"has","vi":"há","en":"has","mode":"en","boundary":"space"}
{"v":1,"seq":15,"t":5126.1,"session":"4f1c9a","type":"label","target":{"kind":"word","id":7},"label":"en","weight":0.3,"source":"implicit"}
{"v":1,"seq":16,"t":5126.11,"session":"4f1c9a","type":"model_update","policy_id":"tiny_mlp@v4","n_samples":1,"loss":0.13}
```

Hotkey và quyết định của luật cứng:

```json
{"v":1,"seq":30,"t":5130.02,"session":"4f1c9a","type":"manual_switch","scope":"in_word","word_id":9,"to":"vi"}
{"v":1,"seq":41,"t":5131.5,"session":"4f1c9a","type":"decision","target":{"kind":"word","id":10},"at_key":6,"policy_id":"tiny_mlp@v4","p_vi":null,"rule":"invalid_vi","chosen":"en"}
```

Log đủ để dựng lại nguyên phiên gõ: replay dựng engine từ `session_start`, không dùng `decision` của model cũ, chỉ dùng `context` + `key` làm input và `label` làm đáp án.

## Replay harness

`nimk-replay` cho một hoặc nhiều policy quyết định lại trên log thật và xuất metric (dự kiến, `main` còn là skeleton).

```sh
nimk-replay --data ~/Library/Application\ Support/NeuralIMK \
            --policy baseline,linear,tiny_mlp --from 2026-11-01 --online
```

| Cơ chế | Nội dung |
| --- | --- |
| Dựng engine | Mỗi session dựng lại từ `config_json` của `session_start`, với `hotkeys = false` |
| Nhãn | Record `label` trong log nạp qua `Engine::inject_label` làm ground truth |
| Restart | `MemStorage` sống xuyên session + `Engine::new` mỗi session, nên nhịp snapshot giống production |
| `--online` | Prequential: predict trước, rồi mới update bằng nhãn, đúng như chạy thật |
| Output | Bảng so sánh trên stdout và `report.json` cho dashboard ([06](06-gui.md)) |

`Engine::inject_label` cũng là cửa nạp nhãn cho dữ liệu synthetic của `nimk-synth`:

```rust
pub fn inject_label(&mut self, target: Target, label: Lang, weight: f32, source: LabelSource);
```

### Log synthetic

`nimk_synth::write_log` ghi script synthetic thành event log thường để replay benchmark policy khi chưa có log thật ([ADR 0006](adr/0006-synthetic-pretraining.md)).

```rust
pub fn write_log(scripts: &[Script], storage: &dyn Storage) -> io::Result<()>;
```

| Record | Nội dung |
| --- | --- |
| `session_start` | Một mỗi script; `session` = `synth-<seed>-<n>` |
| `context` | Một, `bundle_id = "synthetic"` |
| `key` | Toàn bộ phím, gồm ranh giới; không có hotkey |
| `label` | Một mỗi từ, `source = correction`, `weight = 1.0` |

Log synthetic phải để ở storage riêng; trộn với log thật sẽ làm sai metric manual switch.

## Metric

Metric chính là manual switch / 1000 từ; các metric khác giải thích vì sao nó lên hay xuống.

| Metric | Định nghĩa | Mục tiêu |
| --- | --- | --- |
| Manual switch / 1000 từ | Số `manual_switch` (thật) hoặc số từ sai nhãn (replay) trên 1000 từ commit | Chính, tiến về 0 |
| Word accuracy | Tỉ lệ từ commit có `mode == label`, chỉ tính từ mập mờ (không tính từ luật cứng đã quyết) | Tham khảo |
| Auto switch | Số lần policy tự đổi mode | Tham khảo |
| Auto switch precision | Tỉ lệ auto switch không bị sửa trong cửa sổ | > 95% |
| Prior accuracy | Tỉ lệ prior lúc focus đúng | Tham khảo theo app |
| Latency p50/p99 | Thời gian `nimk_key`, đo trong process thật | p99 < 2 ms |

## Thiên lệch của replay

Nhãn ngầm "không sửa = đúng" sinh ra dưới policy cũ, nên replay chỉ là ước lượng.
Policy mới chọn khác ở từ không bị sửa sẽ bị tính sai dù người dùng có thể chấp nhận; kết luận cuối dựa trên metric dùng thật (M5).
