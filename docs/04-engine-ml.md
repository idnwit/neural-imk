# Engine & ML

> **Trạng thái:** draft · **Cập nhật:** 2026-11-08 · **Code:** `crates/nimk-policy`, `crates/nimk-engine/src/{config,labeler,trainer}.rs`, `crates/nimk-types/src/{ctx,features}.rs`

Bài toán là phân loại nhị phân `p(VI | context)` học online từ nhãn ngầm.
Hotkey là nhãn chắc chắn, im lặng là nhãn yếu; không exploration vì mỗi lần thử sai là một lần gõ sai ([ADR 0005](adr/0005-online-supervised-not-exploratory-rl.md)).

## Điểm quyết định

Policy được hỏi ở hai thời điểm; quyết định "chốt" của một từ là mode lúc commit.

| Điểm | Khi nào | `DecisionCtx::raw` | `target` trong log |
| --- | --- | --- | --- |
| Prior | `on_context` khi app hoặc `field_id` đổi | `""` | `Target::Focus(focus_id)`, `at_key = 0` |
| Theo phím | Sau mỗi phím trong từ | `raw` hiện tại | `Target::Word(word_id)`, `at_key` = số phím |

`word_id` là 0, 1, 2... theo session, mỗi từ commit không rỗng một id; id chỉ phụ thuộc phím ranh giới, không phụ thuộc mode, nên replay và dữ liệu synthetic khớp id với log.

## Luật cứng

Luật cứng chạy trước model; khi luật quyết định, `p_vi = null` và `rule` ghi tên luật.

| `DecisionRule` | Điều kiện | Kết quả |
| --- | --- | --- |
| `invalid_vi` | `Transformed::valid_vi == false` (ví dụ `string`, `class`) | EN, bỏ qua model |
| `no_telex` | `Transformed::has_telex == false` | Hai bản trùng nhau, không cần quyết định |

Model chỉ xử lý vùng mập mờ: `has`→`há`, `for`→`fỏ`, `user`→`uẻ`, `mod`, `test`.

## Features

`HashedExtractor` biến `DecisionCtx` thành `Features`: sparse hash vào `SPARSE_BUCKETS = 1 << 16`, dense dài `DENSE_LEN = 8`.

```rust
pub struct DecisionCtx<'a> {
    pub bundle_id: &'a str,
    pub caret: Option<CaretRel>,
    pub raw: &'a str,
    pub valid_vi: bool,
    pub has_telex: bool,
    pub prev_langs: &'a [Lang],
    pub current: Lang,
    pub secs_since_switch: f32,
}

pub struct Features {
    pub app: u32,
    pub current: Lang,
    pub sparse: Vec<u32>,
    pub dense: Vec<f32>,
}
```

| Loại | Feature | Nguồn trong `DecisionCtx` | Mã hoá |
| --- | --- | --- | --- |
| sparse | n-gram ký tự 1–4 của `^raw$` | `raw` | hash → bucket |
| sparse | `bundle_id` | `bundle_id` | hash → bucket |
| sparse | `bundle_id × caret cell` | `bundle_id`, `caret` | hash → bucket |
| sparse | ô lưới 8×8 hoặc "unknown" | `caret` | hash → bucket |
| dense[1] | VI hợp lệ | `valid_vi` | 0/1 |
| dense[1] | Có phím Telex | `has_telex` | 0/1 |
| dense[1] | Độ dài | `raw` | số thực |
| dense[3] | Mode 3 từ trước trong cùng ô nhập, mới nhất trước | `prev_langs` | một slot mỗi từ |
| dense[1] | Mode hiện tại | `current` | 0/1 |
| dense[1] | Thời gian từ lần switch cuối, nén | `secs_since_switch` (`INFINITY` nếu chưa có) | hàm nén về `[0, 1]` |
| named | Hash `bundle_id` | `bundle_id` | `Features::app` |
| named | Mode hiện tại | `current` | `Features::current` |

`app` và `current` để dạng trường có tên cho policy không-neural (`Baseline`) đọc mà không giải mã `sparse`.
Prior lúc focus dùng cùng extractor với `raw = ""`, nên nhóm n-gram rỗng.

## Model theo lộ trình

Mọi model implement cùng `Learner`/`Policy`; chọn bằng `PolicyKind` và `nimk_policy::build`.

```rust
pub struct Model {
    pub extractor: Arc<dyn FeatureExtractor>,
    pub learner: Box<dyn Learner>,
}

pub fn build(kind: PolicyKind, seed: u64) -> Model;
```

`Model` ghép learner với extractor có layout feature mà nó cần; hiện mọi `PolicyKind` dùng `HashedExtractor`.

| `PolicyKind` | Tên (`FromStr`) | Mô tả | Mục tiêu predict |
| --- | --- | --- | --- |
| `Baseline` | `baseline` | Luật cứng + nhớ nhãn cuối theo `Features::app`, fallback `Features::current`; bắt buộc có để đo | ~0 |
| `Linear` | `linear` | Logistic regression FTRL-Proximal trên hashed sparse + dense | vài µs |
| `TinyMlp` | `tiny_mlp` | Embedding bag dim 16 → concat dense → 64 ReLU → sigmoid; forward/backward viết tay, Adam | < 100 µs |
| (sau PoC) | — | char-GRU hoặc transformer nhỏ qua `candle`/`burn` | — |

## Quy tắc quyết định

Hysteresis: chỉ rời mode hiện tại khi model đủ chắc, `|p − 0.5| > switch_margin` (mặc định 0.2).

| Mode hiện tại | Đổi khi | Giữ khi |
| --- | --- | --- |
| EN | `p_vi > 0.7` | `p_vi ≤ 0.7` |
| VI | `p_vi < 0.3` | `p_vi ≥ 0.3` |

Không exploration (ε = 0). Policy tự đổi mode thì trả `overlay = Some(Overlay::Auto)`.

## Nhãn và cửa sổ sửa

Mỗi từ commit tạo một `Sample`; nhãn chỉ chốt khi hết cửa sổ sửa: `correction_words` (2) từ kế tiếp hoặc `label_timeout_secs` (10 s), cái nào đến trước.

```rust
pub struct Sample {
    pub features: Features,
    pub label: Lang,
    pub weight: f32,
}
```

| Trường hợp | `label` | `weight` | `LabelSource` |
| --- | --- | --- | --- |
| Hết cửa sổ, không bị sửa | mode đã dùng | `implicit_weight` = 0.3 | `implicit` |
| Hotkey `in_word` | mode mới | 1.0 | `correction` |
| Hotkey `after_commit`, từ vừa commit | mode mới | 1.0 | `correction` |
| Hotkey `after_commit`, từ trước đó | mode mới | 0.5 | `correction` |
| Hotkey `before_typing` (prior) | mode mới | 1.0 | `correction` |

```text
 time -->
          w1 commit        w2 commit        w3 commit
              |                |                |
 -------------+----------------+----------------+----------------------->
              |<---- w1 window: until w3 commit or t(w1) + 10 s ---->|
                               |<---- w2 window ------------------------->
                     ^
                     | hotkey (after_commit, nothing typed yet)
                     | => w1 label = new mode, weight 1.0, source = correction
                     | => w2 is typed in the new mode
              ^
              | window closes with no hotkey => label = mode used, weight 0.3, implicit
```

Hotkey trong cửa sổ của một từ thay nhãn ngầm bằng nhãn sửa; ngoài cửa sổ, từ đó đã chốt và không đổi nữa.

## Online learning

Trainer chạy nền (thread chỉ nằm trong `trainer` và `logger`), cập nhật model rồi publish snapshot; hot path chỉ đọc snapshot.

| Bước | Khi nào | Tham số (`EngineConfig`) |
| --- | --- | --- |
| Update thường | Mỗi nhãn chốt | — |
| Correction | Mỗi nhãn `correction`: lấy W sample gần nhất, gán lại nhãn, chạy N bước gradient | `correction_window` = 20, `correction_steps` = 5 |
| Chống quên | Mỗi 50 sample mới: 1 minibatch ngẫu nhiên từ replay buffer | `replay_every` = 50, `replay_batch` = 32, `replay_capacity` = 10 000 |
| Snapshot | Mỗi 300 s và khi `deactivateServer`/`shutdown` | `snapshot_every_secs` = 300 |

## Pretrain synthetic

Model khởi đầu được pretrain trên dữ liệu gõ synthetic để không bắt đầu từ con số 0 ([ADR 0006](adr/0006-synthetic-pretraining.md)).

```rust
pub fn generate(cfg: &SynthConfig, corpus: &Corpus) -> Vec<Script>;
pub fn pretrain(
    scripts: &[Script],
    cfg: EngineConfig,
    model: Model,
    storage: Arc<dyn Storage>,
) -> io::Result<PretrainReport>;
```

| Bước | Nội dung |
| --- | --- |
| Corpus | `Corpus::builtin()`: ≥ 1500 từ VI, ≥ 1500 từ EN, ≥ 150 từ EN mập mờ (`has`, `for`, `user`) |
| Trộn | `MixProfile` mặc định: `p_vi_sentence` 0.7, `p_en_in_vi` 0.15, `p_vi_in_en` 0.05, `p_ambiguous` 0.4 |
| Phím | `nimk_telex::encode(word)` sinh `raw` (ví dụ `tiếng` → `tieengs`); khoảng cách phím trung bình 0.12 s |
| Context | Một app giả `SYNTHETIC_BUNDLE_ID = "synthetic"`, không có vị trí con trỏ thật |
| Train | Engine inline với `hotkeys = false`; nhãn mỗi từ nạp qua `Engine::inject_label` ngay sau phím ranh giới |
| Kết quả | Model lưu làm snapshot trong `storage`, IME khởi động từ đó; `PretrainReport { words, correct, tail_accuracy }` đo prequential |

`SynthConfig` mặc định: `seed` 42, 100 session × 500 từ.
Feature app và vị trí không học được từ synthetic; chúng chỉ học online.

## Trait

`Policy` (đọc, hot path) tách khỏi `Learner` (ghi, trainer thread) để hot path không bao giờ chờ lock.

```rust
pub trait FeatureExtractor: Send + Sync {
    fn extract(&self, ctx: &DecisionCtx<'_>, out: &mut Features);
}

pub trait Policy: Send + Sync {
    fn id(&self) -> &str;
    fn predict(&self, features: &Features) -> f32;
}

pub trait Learner: Send {
    fn id(&self) -> &str;
    fn update(&mut self, batch: &[Sample]) -> f32;
    fn snapshot(&self) -> Arc<dyn Policy>;
    fn save(&self) -> Vec<u8>;
    fn load(&mut self, bytes: &[u8]) -> Result<(), PolicyError>;
}
```

| Hợp đồng | Nội dung |
| --- | --- |
| `extract` | Tự xoá `out` trước khi điền; không cấp phát sau khi `out` đã ấm |
| `predict` | Trả `p(VI)`; không cấp phát, không block |
| `update` | Một bước online; trả mean weighted log-loss của batch đo trước khi update (ghi vào `model_update.loss`) |
| `snapshot` | Bản read-only; trainer gửi `Arc<dyn Policy>` qua channel, engine `try_recv` ở ranh giới từ nên hot path không lock |
| `save` | Bytes mở đầu bằng header ghi policy kind và format version |
| `load` | Bytes hỏng hoặc khác kind/version → `PolicyError::Corrupt`; `Engine::new` khi đó dựng model mới và báo qua `take_diagnostics` |

## `EngineConfig` mặc định

Giá trị lấy từ `impl Default for EngineConfig`; đổi mặc định phải cập nhật bảng này cùng commit.
`EngineConfig` derive serde và được ghi vào record `session_start` để replay dựng lại đúng engine.

| Trường | Kiểu | Mặc định | Ý nghĩa |
| --- | --- | --- | --- |
| `policy` | `PolicyKind` | `TinyMlp` | Model dựng khi chưa có snapshot |
| `seed` | `u64` | `0x5eed` | Seed khởi tạo model |
| `hotkeys` | `bool` | `true` | Nhận diện hotkey từ phím; replay đặt `false` và dùng `Engine::inject_label` |
| `switch_margin` | `f32` | `0.2` | Hysteresis |
| `shift_tap_secs` | `f64` | `0.3` | Thời gian giữ Shift tối đa để tính là tap |
| `ctrl_space` | `bool` | `true` | Bật hotkey dự phòng Ctrl+Space |
| `correction_words` | `usize` | `2` | Số từ sau commit mà hotkey còn sửa được |
| `label_timeout_secs` | `f64` | `10.0` | Giây sau commit trước khi chốt nhãn ngầm |
| `implicit_weight` | `f32` | `0.3` | Weight nhãn ngầm |
| `correction_window` | `usize` | `20` | Số sample train lại quanh một correction |
| `correction_steps` | `usize` | `5` | Số bước gradient trên cửa sổ correction |
| `replay_capacity` | `usize` | `10_000` | Dung lượng replay buffer |
| `replay_every` | `usize` | `50` | Train 1 minibatch mỗi N sample mới |
| `replay_batch` | `usize` | `32` | Cỡ minibatch replay |
| `snapshot_every_secs` | `f64` | `300.0` | Chu kỳ lưu snapshot |
| `threading` | `Threading` | `Background` | `Inline` cho test và replay (tất định) |
