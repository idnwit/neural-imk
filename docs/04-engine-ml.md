# Engine & ML

> **Trạng thái:** stable · **Cập nhật:** 2026-11-08 · **Code:** `crates/nimk-types/src/action.rs`, `crates/nimk-policy`, `crates/nimk-engine/src/{config,labeler,trainer}.rs`, `crates/nimk-engine/src/engine/flow.rs`, `crates/nimk-synth/src/pretrain.rs`

Một neural action model quyết định mọi thao tác trên từ đang gõ: `Append`, `Pop`, `Commit`, `Pass`, và rendering VI hay EN ([ADR 0007](adr/0007-neural-action-model.md)).
Hotkey là nhãn chắc chắn, im lặng là nhãn yếu; không exploration vì mỗi lần thử sai là một lần gõ sai ([ADR 0005](adr/0005-online-supervised-not-exploratory-rl.md)).

## Hợp đồng action

Engine dựng `Observation` cho mỗi phím, model trả `ActionScores`, engine chọn `Action` và áp cơ học.
Mọi kiểu nằm trong `nimk-types/src/action.rs`; `RECENT_WINDOW = 16`.

```rust
pub enum EditAction { Append, Pop, Commit, Pass }

pub struct Action {
    pub edit: EditAction,
    pub render: Lang,
}

pub struct ActionMask {
    pub append: bool, // key prints a char that can join a word
    pub pop: bool,    // buffer non-empty
    pub commit: bool, // buffer non-empty
}

pub struct Observation {
    pub key: Option<KeyEvent>, // None = mode prior on focus change; never a hotkey
    pub recent: Vec<KeyEvent>, // up to RECENT_WINDOW previous non-hotkey events
    pub raw: String,
    pub vi: String,            // Telex rendering of raw (tool output)
    pub valid_vi: bool,
    pub viable_vi: bool,
    pub shown: Lang,
    pub mode: Lang,            // sticky mode the next word starts in
    pub prev_langs: Vec<Lang>, // up to 3 previous words, most recent first
    pub bundle_id: String,
    pub caret: Option<CaretRel>,
    pub secs_since_switch: f32, // f32::INFINITY if none
    pub mask: ActionMask,
}

pub struct ActionScores {
    pub edit: [f32; 4], // probabilities over EditAction::ALL, before masking
    pub p_vi: f32,
}

pub struct Example {
    pub obs: Observation,
    pub target: Action,
    pub edit_weight: f32,   // 0 disables the edit-head loss
    pub render_weight: f32, // 0 disables the render-head loss
}
```

`ActionMask` chỉ chặn action vô nghĩa về vật lý (xem [02](02-keyboard-io.md)), không phải luật ngôn ngữ; `Pass` luôn hợp lệ.
Telex và các cờ `valid_vi`/`viable_vi` là feature do công cụ cấp; model tự quyết có tin chúng hay không.

### Telex v2 và kiểu gõ trong dữ liệu học

Quy tắc đầy đủ và bảng kiểm thử của Telex nằm ở [10-telex.md](10-telex.md); mục này chỉ nêu phần ảnh hưởng tới model. Telex v2 cho gõ dấu và `dd` muộn (`theme` → thêm, `dungd` → đung), nên cùng một chuỗi phím như `theme` vừa có thể là EN vừa là VI; model phải phân biệt bằng ngữ cảnh.

### Luật Telex liên quan tới `w`

| Gõ | Kết quả | Ghi chú |
| --- | --- | --- |
| `aw` `ow` `uw` `uow` | `ă` `ơ` `ư` `ươ` | Không đổi |
| `w` | `ư` | `w` không tác động lên `a`/`o`/`u` đứng trước và từ chưa có nguyên âm nào (đầu từ, hoặc sau phụ âm đầu): `tw` → `tư`, `nhwng` → `nhưng`, `twowng` → `tương` |
| `W` | `Ư` | Giữ hoa thường theo từng chữ |
| `ww` | `w` | Phím thứ hai huỷ và gõ `w` thật; `w` tắt cho tới hết từ (`www` → `ww`) |
| `dwowngf` | `dường` | `dw` là `d` + `ư`, không phải `đ`; muốn `đường` gõ `ddwowngf` hoặc `dduwowngf` |
| `when` | `ưhen` | `has_telex` đúng nhưng không viable, nên model xem như EN |
| `new`, `show` | `new`, `show` | `w` không áp khi kết quả không còn khả dĩ (`sh` không là onset) hoặc không có nguyên âm nhận móc |

`encode` luôn trả dạng chuẩn (`uw`, thanh ở cuối); `encode_variants` trả tối đa 8 cách gõ thực tế khác: thanh ngay sau nguyên âm (`toasn`), dấu gõ muộn (`theme`, `vietej`, `tuongw`), `w` đứng riêng (`nhw`), `d` muộn (`dungd`) và tổ hợp của chúng.
Synthetic chọn ngẫu nhiên một biến thể với xác suất `p_variant` (thay cho `p_w_variant`), để pretrain thấy nhiều thứ tự gõ.

## Điểm quyết định

Model được hỏi ở hai thời điểm; quyết định "chốt" của một từ là rendering tại bước `Commit`.

| Điểm | Khi nào | `Observation::key` | `target` trong log |
| --- | --- | --- | --- |
| Prior | `on_context` khi app hoặc `field_id` đổi | `None`, mask chỉ cho `Pass` | `Target::Focus(focus_id)`, `at_key = 0`, `edit = null` |
| Theo phím | Mỗi phím không phải hotkey, không phải `FlagsChanged` | Phím hiện tại | `Target::Word(word_id)`, `at_key` = số phím của từ sau action, `edit` = action đã áp |

`word_id` là 0, 1, 2... theo session, mỗi từ commit không rỗng một id; id chỉ phụ thuộc phím ranh giới, không phụ thuộc rendering, nên replay và dữ liệu synthetic khớp id với log.

## Encoding

`nimk_policy::encode::Encoded` biến `Observation` thành dữ liệu cố định, nằm hoàn toàn trên stack, dùng chung cho hai model.

```rust
pub const SEQ_LEN: usize = 24;
pub const SCALARS: usize = 18;
pub const BUNDLE_BUCKETS: usize = 256;
pub const CARET_CELLS: usize = 65;
```

Chuỗi token là `recent ++ [SEP] ++ raw ++ [phím tới]`, lấy 24 token cuối, đệm trái bằng `PAD`; token cũ nhất bị bỏ khi quá dài.

| Token | Id | Ý nghĩa |
| --- | --- | --- |
| ASCII `0x21..=0x7E` | 0 đến 127 (id = mã byte) | Ký tự in được; dải id này luôn chiếm 128 hàng embedding |
| `PAD`, `SEP` | 128, 129 | Đệm trái; đánh dấu đầu buffer `raw` |
| `SPACE`, `RETURN`, `TAB`, `BACKSPACE` | 130 đến 133 | Phím tương ứng |
| `NAV`, `SHORTCUT` | 134, 135 | Mũi tên; phím có ⌘/⌃/⌥ |
| `OTHER`, `FLAGS`, `NONE` | 136, 137, 138 | Escape/không ASCII; modifier-only; không có phím (prior) |

Vocab là 139 hàng embedding. `bundle_id` băm FNV-1a vào 256 bucket; con trỏ vào ô lưới 8x8, id 64 nghĩa là không biết (65 ô).

| Scalar | Vị trí | Nội dung |
| --- | --- | --- |
| `VALID_VI`, `VIABLE_VI`, `TELEX_CHANGED` | 0 đến 2 | `valid_vi`, `viable_vi`, `vi != raw` |
| `SHOWN_VI` | 3 | **Dành riêng, luôn 0** (xem bên dưới) |
| `MODE_VI` | 4 | `mode == Vi` |
| `PREV_LANGS` | 5 đến 7 | +1 VI, -1 EN, 0 chưa có |
| `SWITCH_RECENCY` | 8 | `1 / (1 + secs_since_switch / 10)`, 0 nếu chưa switch |
| `RAW_LEN` | 9 | Độ dài `raw` / 10, tối đa 1 |
| `MASK_APPEND`, `MASK_POP`, `MASK_COMMIT` | 10 đến 12 | Các quyền của mask |
| `SHIFT`, `CMD`, `CTRL`, `OPT` | 13 đến 16 | Modifier của phím tới |
| `HAS_KEY` | 17 | Có phím tới (0 với prior) |

Slot `SHOWN_VI` chỉ giữ chỗ để kích thước snapshot không đổi; lý do dành riêng nó là phát hiện rò nhãn (label leak).
Khi pretrain bằng `Engine::teach`, `shown` là render *đích* của phím trước, nên feature `shown == Vi` lộ nhãn và model học chép nó.
Độ chính xác render trên tập giữ lại chỉ đạt khoảng 0.65; đặt feature về 0 (cùng các sửa ở mục Kết quả đo) đưa nó lên khoảng 0.97. Snapshot cũ phải pretrain lại.

## Model

Hai model cùng implement `ImeLearner`/`ImeModel`; chọn bằng `ModelKind` và `build_action` ([ADR 0009](adr/0009-burn-nn-backend.md) cho backend).

```rust
pub trait ImeModel: Send + Sync {
    fn id(&self) -> &str;
    fn score(&self, obs: &Observation) -> ActionScores;
}

pub trait ImeLearner: Send {
    fn id(&self) -> &str;
    fn update(&mut self, batch: &[Example]) -> f32;
    fn learns_from_confirmations(&self) -> bool { true }
    fn snapshot(&self) -> Arc<dyn ImeModel>;
    fn save(&self) -> Vec<u8>;
    fn load(&mut self, bytes: &[u8]) -> Result<(), PolicyError>;
}

pub fn build_action(kind: ModelKind, seed: u64) -> Box<dyn ImeLearner>;
pub fn decide(scores: &ActionScores, mask: ActionMask) -> Action;
```

| Hợp đồng | Nội dung |
| --- | --- |
| `score` | Không block, không cấp phát lớn; p99 < 1 ms |
| `update` | Một bước online; trả loss có trọng số của batch đo trước khi update (ghi vào `model_update.loss`) |
| `snapshot` | Bản read-only; trainer gửi `Arc<dyn ImeModel>` qua channel, engine `try_recv` ở ranh giới từ nên hot path không lock |
| `save`/`load` | Header `NIMK` + kind + version + payload; hỏng hoặc khác kind/version → `PolicyError::Corrupt`, `Engine::new` dựng model mới và báo qua `take_diagnostics` |
| Neutral init | Model chưa train cho `p_vi` trong 0.5 +- 0.05 và edit gần đều; vì vậy IME phải đi kèm snapshot đã pretrain |
| `decide` | Edit xác suất cao nhất trong số các edit mask cho phép (hoà thì edit đứng trước trong `EditAction::ALL`, `Pass` luôn hợp lệ); `Vi` nếu `p_vi >= 0.5` |

`ModelKind` có hai giá trị, tên `FromStr` lần lượt `linear_action` và `gru_action`; mặc định của engine là `GruAction`: sau Telex v2 nó chính xác hơn (render 0.974 so với 0.965) và phân biệt được ngữ cảnh (`theme` sau "Tôi muốn" ra thêm, sau "change the dark" giữ theme), điều `LinearAction` không làm được; xem devlog.

| | `LinearAction` | `GruAction` |
| --- | --- | --- |
| Cài đặt | Viết tay, không phụ thuộc ngoài | `burn` 0.22.1 backend `flex`, autodiff |
| Kiến trúc | 5 đầu ra (4 edit + 1 render) trên 2^15 hàng băm n-gram/context và 18 scalar, trọng số khởi tạo 0 | Embedding 32 → GRU 64 trên 24 token → ghép [h, scalars, bundle emb 16, caret emb 8] → Linear 64 + ReLU → edit (4) và render (1); 35 053 tham số |
| Optimizer | SGD minibatch, mỗi hàng chạm tới bước `lr * sum / sqrt(hits)`; `LINEAR_LR = 0.1` | Adam `GRU_LR = 3e-3` cho batch có edit |
| Online (render-only) | Bước nhân `LINEAR_ONLINE_SCALE = 0.03` | SGD thuần `GRU_ONLINE_LR = 1e-3`, chỉ cập nhật đầu render |
| `learns_from_confirmations` | `true` | `false` |
| Chính quy hoá | — | Context dropout: 25% ẩn mọi token trừ phím tới, ngược lại 10% token thành `OTHER` |
| Khởi tạo | Zero, bỏ qua `seed` | RNG riêng theo `seed` (RNG thiết bị của burn là global) |
| Snapshot | ~0.5 MB (`linear_action@v1`) | ~4 MB (`gru_action@v1`); không lưu trạng thái Adam |

```text
 tokens [24] -> Embedding(139,32) -> GRU(64) -> last hidden [64] --+
 scalars [18] -----------------------------------------------------+
 bundle  [1]  -> Embedding(256,16) ---------------------------------+-> concat [106]
 caret   [1]  -> Embedding(65,8) -----------------------------------+
        -> Linear(106,64) + ReLU --+--> edit   Linear(64,4)  (logits, softmax)
                                   +--> render Linear(64,1)  (logit of p_vi)
```

Hai đầu dùng chung thân dense; đầu edit train không mask, engine mask lúc suy luận qua `decide`.

Vì sao `LinearAction` dùng SGD chứ không Adam: Adam chuẩn hoá theo toạ độ nên n-gram hiếm bị nhớ đè lên bằng chứng cấu trúc; trên từ lạ (bài toán đồ chơi) Adam đạt 83% render, SGD 94%.
Vì sao `GruAction` cần dropout: không có thì từ lạ chỉ đạt khoảng 0.87 trên bài toán đồ chơi, dù trong từ vựng cao hơn khoảng 0.4 điểm.

## Quy tắc quyết định

Mỗi phím có một quyết định `edit` từ `decide`; rendering qua ba nhánh, hysteresis chỉ làm ổn định hiển thị giữa từ.

| Tình huống | Rendering dùng |
| --- | --- |
| Từ đã bị khoá bởi hotkey giữa từ | Rendering hiện tại, không đổi đến hết từ |
| Action là `Commit` | `Vi` iff `p_vi >= 0.5` của chính model tại bước commit (quyết định mới, không qua hysteresis) |
| Còn lại (giữa từ) | Hysteresis: chỉ rời rendering đang hiện khi `\|p - 0.5\| > switch_margin` (mặc định 0.02) |
| Prior lúc focus | Hysteresis quanh sticky mode |

Hysteresis chỉ áp giữa từ vì thực đo cho thấy giữ rendering cũ lúc commit làm hỏng từ kết thúc ở vùng mập mờ (khoảng 1 điểm).
Hiển thị giữa từ nên bám dự đoán của model theo từng phím, nên margin được giữ rất nhỏ. Đo bằng `nimk-synth eval --model K --init target/models/K --seed 42 --sessions 200 --words 500 --margins ...` trên 20 script giữ lại (snapshot đóng gói). `mid_word_acc` = tỉ lệ phím giữa từ mà rendering đang hiện bằng nhãn cuối của từ; `flips/từ` = số lần rendering hiện đổi giữa các phím của một từ; `render@commit` không phụ thuộc margin vì commit dùng quyết định mới.

| margin | GRU mid_word_acc | GRU flips/từ | Linear mid_word_acc | Linear flips/từ | render@commit (GRU / Linear) |
| --- | --- | --- | --- | --- | --- |
| 0.0 | 0.9062 | 0.274 | 0.8565 | 0.382 | 0.9736 / 0.9652 |
| 0.02 | 0.9063 | 0.259 | 0.8569 | 0.367 | 0.9736 / 0.9652 |
| 0.05 | 0.9061 | 0.239 | 0.8569 | 0.347 | 0.9736 / 0.9652 |
| 0.1 | 0.9049 | 0.214 | 0.8574 | 0.318 | 0.9736 / 0.9652 |
| 0.2 | 0.9004 | 0.187 | 0.8611 | 0.271 | 0.9736 / 0.9652 |

Chọn 0.02 cho model mặc định (GRU): độ chính xác cao nhất, flips/từ 0.26 (dưới ngưỡng 0.3), bỏ dao động quanh đúng 0.5. Chênh lệch giữa các margin nhỏ (dưới 1 điểm); Linear (không phải mặc định) lại nhỉnh hơn với margin lớn.

Sticky mode đổi ở hai nơi: lúc commit (theo rendering của từ) và lúc focus; hotkey đổi ngay. `overlay = Auto` chỉ khi sticky mode đổi do model.
Không exploration (ε = 0).

## Nhãn và cửa sổ sửa

Mỗi từ commit giữ lại các bước model đã đi (`Step { obs, edit }`); nhãn chốt sinh một `Example` cho mỗi bước.

```rust
// minh hoạ: Example sinh từ một bước của từ có nhãn `label`
Example {
    obs: step.obs,
    target: Action { edit: step.edit, render: label },
    edit_weight: cfg.edit_online_weight, // 0.0 mặc định
    render_weight: weight,
}
```

Nhãn chỉ chốt khi hết cửa sổ sửa: `correction_words` (2) từ kế tiếp hoặc `label_timeout_secs` (10 s), cái nào đến trước.

| Trường hợp | `label` | `weight` | `LabelSource` |
| --- | --- | --- | --- |
| Hết cửa sổ, không bị sửa | rendering đã commit | `implicit_weight` = 0.3 | `implicit` |
| Hotkey giữa từ (`in_word`) | rendering mới | 1.0 | `correction` |
| Hotkey sửa ngược từ vừa commit (`after_commit`) | rendering mới | 1.0 | `correction` |
| Hotkey `before_typing` (prior) | sticky mode mới | 1.0 | `correction` |

Labeler giữ steps của 4 từ cuối đã chốt ngầm; sửa ngược muộn vẫn ghi đè nhãn ngầm đó bằng một nhãn `correction` mới cùng `Example`.
Mỗi lần bật tắt tiếp theo của cùng từ phát một nhãn mới. Nhãn `FinalLabel::corrected` chỉ đúng khi nhãn mâu thuẫn rendering đã commit.

```text
 time -->
          w1 commit        w2 commit        w3 commit
              |                |                |
 -------------+----------------+----------------+----------------------->
              |<---- w1 window: until w3 commit or t(w1) + 10 s ---->|
                               |<---- w2 window ------------------------->
                     ^
                     | double Shift (after_commit, only boundary typed)
                     | => w1 replaced in the other rendering
                     | => w1 label = new rendering, weight 1.0, source = correction
              ^
              | window closes with no hotkey => label = rendering used, weight 0.3, implicit
```

Sửa ngược trong cửa sổ thay nhãn ngầm bằng nhãn sửa; ngoài cửa sổ nhưng còn trong cache 4 từ, nó vẫn ghi đè.

## Chọn model

Người dùng chọn `LinearAction` hoặc `GruAction` trên menu IME ([06](06-gui.md)); mỗi model giữ checkpoint riêng.

| Mục | Nội dung |
| --- | --- |
| Checkpoint | `model/<kind>/latest`, trên đĩa `<data>/blobs/model/<kind>/latest`; không bao giờ dùng chung giữa hai model |
| Lựa chọn | Blob `settings/model`, JSON `{"model":"linear_action"}`; `Engine::new` đọc nó trước, hợp lệ thì ghi đè `cfg.policy`, hỏng thì bỏ qua và báo diagnostic |
| `switch_model` | Lưu checkpoint model cũ, thay trainer, nạp checkpoint model mới, publish snapshot; checkpoint thiếu hoặc hỏng thì dùng model mới chưa train kèm diagnostic |
| Replay, pretrain | Không đọc `settings/model`: replay chỉ chép blob `model/<kind>/latest` của model đang đánh giá, pretrain dùng `with_model` |

Model chưa pretrain là trung tính ([07](07-build-and-install.md#pretrain-model)); menu ghi "(chưa pre-train)" khi chưa có checkpoint.

## Online learning

Trainer chạy nền (thread chỉ nằm trong `trainer` và `logger`), cập nhật model rồi publish snapshot; hot path chỉ đọc snapshot.
Các ví dụ của một từ được gửi cùng nhau thành một lần `update`.

| Bước | Khi nào | Tham số (`EngineConfig`) |
| --- | --- | --- |
| Update thường | Mỗi nhãn chốt, toàn bộ ví dụ của từ trong một batch | — |
| Correction | Chỉ khi nhãn mâu thuẫn rendering đã commit: lấy W ví dụ gần nhất, chạy N bước gradient | `correction_window` = 20, `correction_steps` = 5 |
| Chống quên | Mỗi 50 ví dụ mới: 1 minibatch ngẫu nhiên từ replay buffer | `replay_every` = 50, `replay_batch` = 32, `replay_capacity` = 10 000 |
| Publish snapshot | Mỗi 20 lần update và sau mỗi correction | — |
| Lưu snapshot | Mỗi `snapshot_every_secs` (theo giờ event) và khi `shutdown` | `snapshot_every_secs` = 300 |

Quy tắc học online:

1. Chỉ đầu render học online. `edit_online_weight = 0.0` nên đầu edit chỉ học lúc pretrain hoặc `Engine::teach`; chưa có tín hiệu sửa lỗi edit đáng tin.
2. Correction luôn được học. Nhãn xác nhận (implicit, hoặc nhãn trùng rendering đã commit) chỉ được học nếu `learns_from_confirmations()`.
3. `GruAction` trả `false`: trainer bỏ mọi batch render-only không phải correction. Cập nhật render-only trôi thân GRU; mỗi bước Adam trên đầu render tốn 1 đến 2 điểm, và học từ mọi xác nhận làm model thiên về ngôn ngữ của từ cuối.
4. Batch có `edit_weight == 0` chỉ cập nhật đầu render: `GruAction` dùng SGD `GRU_ONLINE_LR`, `LinearAction` nhân bước với `LINEAR_ONLINE_SCALE`.
5. Với `hotkeys = false` (replay, pretrain) không sinh nhãn ngầm; nhãn đến từ `Engine::inject_label`.

## Pretrain synthetic

Model khởi đầu được pretrain bằng behaviour cloning trên dữ liệu gõ synthetic, để không bắt đầu từ con số 0 ([ADR 0006](adr/0006-synthetic-pretraining.md), [ADR 0007](adr/0007-neural-action-model.md)).

```rust
pub fn generate(cfg: &SynthConfig, corpus: &Corpus) -> Vec<Script>;
pub fn pretrain(
    scripts: &[Script],
    cfg: EngineConfig,
    opts: PretrainOptions,
    storage: Arc<dyn Storage>,
) -> io::Result<PretrainReport>;
```

| Bước | Nội dung |
| --- | --- |
| Corpus | `Corpus::builtin()`: 3090 từ VI, 1672 từ EN, 245 từ EN mập mờ (`has`, `for`, `user`); ba danh sách rời nhau |
| Trộn | `MixProfile` mặc định: `p_vi_sentence` 0.7, `p_en_in_vi` 0.15, `p_vi_in_en` 0.05, `p_ambiguous` 0.4, `p_typo` 0, `p_shortcut` 0, `p_variant` 0.5 |
| Phím và đích | Mỗi `Script` có `keys` và `targets` cùng chỉ số: chữ → `Append`, phím kết từ → `Commit`, ranh giới khi buffer rỗng và ⌘+phím → `Pass`, Backspace sau gõ nhầm → `Pop`; `render` là ngôn ngữ của từ (hoặc từ trước với `Pass`) |
| Train | Một engine inline, `hotkeys = false`; mỗi phím qua `Engine::teach` với `edit_weight = render_weight = 1.0`; mỗi script bắt đầu bằng đổi focus sang field mới |
| Tham số | `PretrainOptions { epochs: 2, eval_frac: 0.1, batch: 128 }` mặc định; trainer replay tắt, snapshot chỉ lưu một lần lúc kết thúc |
| Đầu ra | Chỉ blob `model/<kind>/latest` được giữ, event bị bỏ; `PretrainReport { epochs, train_scripts, train_keys, eval }` |
| Đánh giá | Engine mới nạp snapshot đã lưu, chạy `on_key` trên script giữ lại (mô hình tự quyết, không học), đọc `Decision` đã log. Phím không gọi model (ranh giới nối vào từ pending, Return, phím tắt; ADR 0011) không có `Decision`, nên quyết định được ghép với phím qua bản ghi `key`; text commit mang cả ranh giới (`tiếng `) nên được cắt trước khi so |

`SynthConfig` mặc định: `seed` 42, 100 session x 500 từ, `key_interval_secs` 0.12.
`EvalReport` gồm độ chính xác edit, render tại commit (kể cả quyết định mới trước hysteresis), riêng từ mập mờ và từ EN mập mờ, histogram `p_vi` tại commit, độ chính xác theo vị trí phím, và tỉ lệ text khớp.
Từ "mập mờ" là từ có `vi != raw` và `vi` là âm tiết hợp lệ. Feature app và vị trí không học được từ synthetic; chúng chỉ học online.
CLI và quy trình cài snapshot: [07](07-build-and-install.md).

## Kết quả đo

Số liệu từ lần tuning 2026-11-08, `seed` 42, bản release; replay dùng log synthetic seed 7, 20 session x 500 từ, nhiễu khoảng +-0.3 điểm.
Pretrain: render@commit là độ chính xác rendering tại phím commit, amb là trên từ mập mờ, match là tỉ lệ từ commit khớp text kỳ vọng.

| Model, cấu hình | render@commit | amb | amb EN | match | Replay online | Replay `--frozen` |
| --- | --- | --- | --- | --- | --- | --- |
| `LinearAction`, trước tuning | 0.757 | 0.711 | — | 0.794 | 94.4%, manual/1k 56 | — |
| `LinearAction`, 100 session x 500 x 2 epoch | 0.967 | 0.964 | 0.926 | 0.974 | 97.0% | 96.6% |
| `LinearAction`, 200 x 500 x 2 | 0.975 | 0.971 | 0.944 | 0.979 | 97.5% | — |
| `GruAction`, trước tuning (100 x 500 x 1, 414 s) | 0.673 | 0.671 | — | 0.733 | 55.3%, chỉ 5746 trên 10 000 từ được commit | — |
| `GruAction`, 200 x 500 x 2 (102 s) | 0.975 | 0.969 | 0.909 | 0.978 | 97.3%, đủ 10 000 từ | 97.4% |
| `LinearAction`, 200 x 500 x 2, sau `w` → `ư` (2026-11-08) | 0.968 | 0.964 | 0.984 | 0.974 | — | — |
| `GruAction`, như trên | 0.968 | 0.962 | 0.967 | 0.973 | — | — |

Hai dòng đó: dữ liệu có thêm kiểu gõ `w` đứng riêng nên khó hơn (thêm từ mập mờ kiểu `was`, `war`, `walk`).

Sau Telex v2 và pending-word commit (ADR 0011), cùng cấu hình 200 x 500 x 2, `p_variant` 0.5, từ `did`, `wrong`, `dad` chuyển sang `en_ambiguous`, `away` sang `en`:

| Model | render@commit | amb | amb EN | match | lát "gõ biến thể" (`p_variant` 1.0): render / amb |
| --- | --- | --- | --- | --- | --- |
| `LinearAction` | 0.965 | 0.961 | 0.880 | 0.972 | 0.958 / 0.952 |
| `GruAction` | 0.974 | 0.968 | 0.940 | 0.977 | 0.969 / 0.961 |

Lát "gõ biến thể" là 10 session x 300 từ chưa thấy, mọi từ VI gõ bằng biến thể (CLI in `variant_*`).
Thăm dò `theme` sau ngữ cảnh (CLI in `probe ...`; `p_vi` là đầu ra trước hysteresis):

| Ngữ cảnh | `LinearAction` | `GruAction` |
| --- | --- | --- |
| "Tôi muốn" + `theme` (kỳ vọng thêm) | p_vi 0.347, ra EN | p_vi 0.671, ra VI |
| "change the dark" + `theme` (kỳ vọng EN) | p_vi 0.500, ra VI (sai) | p_vi 0.057, ra EN |
| "Tôi muốn" + `theem` (kỳ vọng thêm) | 0.986, VI | 0.790, VI |

`LinearAction` chưa tách được `theme` theo ngữ cảnh (đặc trưng tuyến tính ít dùng từ trước); `GruAction` tách đúng. Đây là lý do GRU vẫn được giữ để so sánh.

Độ chính xác edit đạt 1.000 ở mọi cấu hình đã tuning; các lỗi còn lại là phím giống hệt nhau ở hai ngôn ngữ (`in`, `ten`, `an`, `hang`, `me`, `no`).
Online learning hầu như không cải thiện trên log synthetic (97.3% so với 97.4% frozen cho GRU) vì nhãn synthetic cùng phân phối với pretrain; lợi ích thật chỉ đo được trên log thật.

Nguyên nhân của số "trước tuning" và cách sửa:

| # | Phát hiện | Kết luận |
| --- | --- | --- |
| 1 | Rò nhãn qua `SHOWN_VI` (xem [Encoding](#encoding)) | Sửa: scalar dành riêng, luôn 0 |
| 2 | `LinearAction` lr 2.0 cập nhật từng ví dụ làm nổ; GRU chậm vì cập nhật từng phím | Sửa: lr 0.1 với bước `lr * sum / sqrt(hits)`; pretrain batch 128 (GRU 414 s xuống 102 s cho 2 epoch lớn hơn) |
| 3 | Commit dùng rendering giữ bởi hysteresis (H1) | Xác nhận nhỏ (+1 điểm): commit dùng quyết định mới |
| 4 | Trọng số theo vị trí phím cho render (H2) | Bác bỏ, làm kém đi |
| 5 | Cập nhật render-only làm trôi thân GRU (H3) | Xác nhận: `learns_from_confirmations = false` cho GRU |
| 6 | Batching khi pretrain (H4) | Xác nhận, xem dòng 2 |

## Độ trễ

Đo trên M4 Pro, bản release; ngân sách cho `nimk_key` là p99 < 2 ms gồm suy luận.

| Thao tác | `LinearAction` | `GruAction` |
| --- | --- | --- |
| `score` | p99 0.5 µs | trung bình 166 µs, p99 170 đến 186 µs |
| `update`, batch 32 | 17 µs | 3.6 ms (p99 4.3 ms) |
| `snapshot` | 8 µs | 4 µs |

`update` và `snapshot` chạy trên trainer thread nên không nằm trên đường phím; chỉ `score` được đo trong ngân sách.

## `EngineConfig` mặc định

Giá trị lấy từ `impl Default for EngineConfig`; đổi mặc định phải cập nhật bảng này cùng commit.
`EngineConfig` derive serde với `#[serde(default)]` và được ghi vào record `session_start`; log cũ có `policy` là `tiny_mlp` không còn parse được.

| Trường | Kiểu | Mặc định | Ý nghĩa |
| --- | --- | --- | --- |
| `policy` | `ModelKind` | `GruAction` | Model dựng khi chưa có snapshot; `settings/model` (lựa chọn trên GUI) ghi đè khi `Engine::new` |
| `seed` | `u64` | `0x5eed` | Seed khởi tạo model |
| `switch_margin` | `f32` | `0.02` | Hysteresis giữa từ |
| `hotkeys` | `bool` | `true` | Nhận diện hotkey; replay và pretrain đặt `false` |
| `shift_tap_secs` | `f64` | `0.3` | Giữ Shift tối đa để tính là một nháy |
| `double_shift` | `bool` | `true` | Bật hotkey nháy Shift hai lần |
| `double_shift_secs` | `f64` | `0.4` | Khoảng tối đa giữa hai lần nhả Shift |
| `ctrl_space` | `bool` | `true` | Bật hotkey dự phòng Ctrl+Space |
| `correction_words` | `usize` | `2` | Số từ sau commit mà nhãn còn chờ sửa |
| `label_timeout_secs` | `f64` | `10.0` | Giây sau commit trước khi chốt nhãn ngầm |
| `implicit_weight` | `f32` | `0.3` | Weight nhãn ngầm |
| `edit_online_weight` | `f32` | `0.0` | Weight loss edit trong ví dụ online; 0 = đầu edit chỉ học bằng pretrain/`teach` |
| `teach_batch` | `usize` | `1` | Số ví dụ `teach` mỗi lần update; phần dư train lúc `shutdown` (`nimk-synth` dùng 128) |
| `correction_window` | `usize` | `20` | Số ví dụ train lại quanh một correction |
| `correction_steps` | `usize` | `5` | Số bước gradient trên cửa sổ correction |
| `replay_capacity` | `usize` | `10_000` | Dung lượng replay buffer |
| `replay_every` | `usize` | `50` | Train 1 minibatch mỗi N ví dụ mới |
| `replay_batch` | `usize` | `32` | Cỡ minibatch replay |
| `snapshot_every_secs` | `f64` | `300.0` | Chu kỳ lưu snapshot |
| `threading` | `Threading` | `Background` | `Inline` cho test, replay, pretrain (tất định) |
