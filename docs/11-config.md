# Cấu hình TOML

> **Trạng thái:** stable · **Cập nhật:** 2026-11-09 · **Code:** `crates/nimk-engine/config/default.toml`, `crates/nimk-engine/src/config.rs`, `macos/NeuralIMK/Core.swift`

Mọi tham số điều chỉnh được nằm trong một file TOML; code chỉ giữ logic chung ([ADR 0012](adr/0012-unified-boundary-commit.md)).
Mặc định ở `crates/nimk-engine/config/default.toml`, nhúng vào engine bằng `include_str!`; `EngineConfig::default()` chính là kết quả parse của file đó.

## Nạp cấu hình

```text
+------------------+  text   +-----------------+  nimk_set_config  +---------------------+
| config.toml      | ------> | Core.swift      | ----------------> | EngineConfig::      |
| (Application     |         | reloadConfig()  |                   | from_toml_layers    |
|  Support/<id>/)  |         +-----------------+                   +---------------------+
+------------------+                                                         |
   optional file             menu "Reload config" or start-up       defaults + override
```

Engine không đọc file (arch-lint cấm `std::fs`): Swift đọc `<Application Support>/<bundle id>/config.toml` (không có file cũng được) và đưa văn bản qua FFI.
`EngineConfig::from_toml_layers(&[&str])` ghép từng lớp lên mặc định nhúng, từng khoá một; mảng thay nguyên khối, `[[app_category]]` ghép theo `name` và `params` ghép từng khoá.

- Giá trị sai kiểu, ngoài khoảng, hay khoá lạ: bỏ giá trị đó, ghi một dòng `config: <khoá>: <lý do>` vào log chẩn đoán, giữ giá trị trước (mặc định). Không bao giờ làm hỏng khởi tạo; TOML không parse được thì bỏ cả lớp.
- Nạp lại bằng mục menu `Reload config` (`nimk_set_config` → `Engine::reconfigure`): áp ngay các khoá "live" (cột Live bên dưới); khoá khác (model, seed, trainer) có hiệu lực ở lần chạy sau.
- Nhóm app được chọn theo `bundle_id` mỗi lần focus đổi (`on_context`); `Engine::idle_commit_secs()` và `nimk_idle_commit_secs` trả giá trị đang hiệu lực.

## Khoá

Test `every_field_is_in_default_toml` bảo đảm mọi trường của `EngineConfig` có trong `default.toml` và ngược lại.

| Khoá | Kiểu | Khoảng | Mặc định | Live | Ý nghĩa |
| --- | --- | --- | --- | --- | --- |
| `policy` | `"gru_action"`, `"linear_action"` | enum | `gru_action` | không | Model dựng khi chưa có snapshot |
| `seed` | số nguyên | `u64` | 24301 | không | Seed khởi tạo model |
| `switch_margin` | số thực | 0 đến 0.5 | 0.02 | có | Hysteresis giữa từ: chỉ đổi rendering khi `|p - 0.5| >` margin |
| `hotkeys` | bool | | true | không | Nhận hotkey; replay đặt false |
| `implicit_weight` | số thực | 0 đến 1 | 0.3 | không | Trọng số nhãn ngầm |
| `edit_online_weight` | số thực | 0 đến 1 | 0.0 | không | Trọng số loss đầu edit khi học online |
| `correction_words` | số nguyên | 0 đến 20 | 2 | không | Số từ sau commit mà hotkey còn sửa được |
| `label_timeout_secs` | số thực | 0.5 đến 600 | 10 | không | Giây trước khi nhãn ngầm chốt |
| `teach_batch` | số nguyên | 1 đến 4096 | 1 | không | Số ví dụ teacher-forced mỗi lần cập nhật |
| `correction_window` | số nguyên | 1 đến 1000 | 20 | không | Mẫu học lại quanh một correction |
| `correction_steps` | số nguyên | 0 đến 100 | 5 | không | Số bước gradient trên cửa sổ đó |
| `replay_capacity` | số nguyên | 16 đến 1000000 | 10000 | không | Sức chứa replay buffer |
| `replay_every` | số nguyên | 0 đến 100000 | 50 | không | Cứ N mẫu mới train một minibatch (0 = tắt) |
| `replay_batch` | số nguyên | 1 đến 4096 | 32 | không | Cỡ minibatch |
| `snapshot_every_secs` | số thực | 10 đến 86400 | 300 | không | Chu kỳ lưu snapshot |
| `threading` | `"background"`, `"inline"` | enum | `background` | không | Trainer và logger chạy nền hay cùng thread |
| `double_shift` | bool | | true | có | Nhận nháy Shift hai lần |
| `double_shift_secs` | số thực | 0.05 đến 2 | 0.4 | có | Tối đa giữa hai lần nhả Shift |
| `shift_tap_secs` | số thực | 0.05 đến 1 | 0.3 | có | Tối đa giữa nhấn và nhả Shift để tính là nháy |
| `ctrl_space` | bool | | true | có | Nhận thêm Ctrl+Space |
| `idle_commit_secs` | số thực | 0, hoặc 0.3 đến 10 | 1.5 | có | Giây không phím thì Swift commit marked text; 0 = tắt |
| `extra_letters` | chuỗi | không điều khiển, không khoảng trắng | `""` | có | Ký tự coi là chữ dù không phải chữ cái (ví dụ `"_"`) |
| `extra_boundaries` | chuỗi | không điều khiển | `""` | có | Ký tự coi là ranh giới dù là chữ cái |
| `commit_keycodes` | mảng số | mỗi số 0 đến 127 | Return, Enter số, Tab, Escape, Home/End/PageUp/PageDown, mũi tên, Delete tới, F1 đến F20 | có | Phím commit từ rồi tới app (phím tắt ⌘/⌃/⌥ luôn commit) |
| `command_triggers` | mảng chuỗi | mỗi chuỗi đúng 1 ký tự, không phải chữ, số, khoảng trắng | `["/", "@", "!"]` | có | Ký tự từ `Idle` vào `Raw` |
| `command_end_chars` | chuỗi | | `" "` | có | Ký tự kết thúc `Raw` (vẫn tới app) |
| `command_end_keycodes` | mảng số | mỗi số 0 đến 127 | Return, Enter số, Tab, Escape, Home/End/PageUp/PageDown, mũi tên, Delete tới | có | Phím kết thúc `Raw` (phím tắt cũng kết thúc) |

### Per-model options (`[models.<kind>]`)

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `models.gru_action.online_training` | bool | true | Train this model online, also while it is not active (shadow training, [ADR 0013](adr/0013-shadow-training-and-model-stats.md)); false freezes it |
| `models.linear_action.online_training` | bool | true | Same for the linear model |

Layers merge per model and per key; an unknown model or key, or a value of the wrong type, is dropped with a diagnostic. The IME menu toggle is saved in the blob `settings/models` and overrides these values at start.

## Ghi đè theo nhóm app

`[[app_category]]` gồm `name`, `bundle_ids` (mục kết thúc bằng `*` khớp theo tiền tố) và bảng `[app_category.params]`.
Nhóm khớp đầu tiên thắng; `params` ghi đè các khoá cùng tên ở cột Live (`switch_margin`, `idle_commit_secs`, `double_shift`, `double_shift_secs`, `shift_tap_secs`, `ctrl_space`, `extra_letters`, `extra_boundaries`, `commit_keycodes`, `command_triggers`, `command_end_chars`, `command_end_keycodes`), cùng khoảng giá trị.
Mỗi khoá sai trong `params` bị bỏ riêng (báo `app_category.<name>.params.<khoá>`), phần còn lại của nhóm vẫn có hiệu lực.

| Nhóm | `bundle_ids` | `idle_commit_secs` |
| --- | --- | --- |
| `terminal` | `com.github.wez.wezterm`, `com.googlecode.iterm2`, `com.apple.Terminal`, `com.mitchellh.ghostty`, `net.kovidgoyal.kitty`, `org.alacritty`, `dev.warp.Warp-Stable` | 1.0 |
| `ide` | `com.microsoft.VSCode`, `com.todesktop.230313mzl4w4u92` (Cursor), `dev.zed.Zed`, `com.apple.dt.Xcode`, `com.jetbrains.*` | 1.0 |
| `browser` | `com.google.Chrome`, `com.microsoft.edgemac`, `com.apple.Safari`, `org.mozilla.firefox` | 0.8 |
| `editor` | `com.apple.Notes`, `com.apple.TextEdit` | 2.0 |

Ví dụ `config.toml` của người dùng:

```toml
switch_margin = 0.05
command_triggers = ["/", "@", "!", ":"]

[[app_category]]
name = "terminal"
[app_category.params]
idle_commit_secs = 0.6

[[app_category]]
name = "chat"
bundle_ids = ["com.tinyspeck.slackmacgap"]
[app_category.params]
command_end_chars = " ,"
```

Bản sao `default.toml` được chép vào `Contents/Resources/default-config.toml` khi đóng gói, chỉ để tham khảo; mặc định thật lấy từ bản nhúng trong engine.
