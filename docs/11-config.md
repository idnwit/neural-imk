# Configuration (TOML)

> **Status:** stable · **Updated:** 2026-11-09 · **Code:** `crates/nimk-engine/config/default.toml`, `crates/nimk-engine/src/config.rs`, `macos/NeuralIMK/Core.swift`

Every tunable lives in one TOML file; code keeps only the shared logic ([ADR 0012](adr/0012-unified-boundary-commit.md)).
Defaults are in `crates/nimk-engine/config/default.toml`, embedded with `include_str!`; `EngineConfig::default()` is that file parsed.

## Loading

```text
+------------------+  text   +-----------------+  nimk_set_config  +---------------------+
| config.toml      | ------> | Core.swift      | ----------------> | EngineConfig::      |
| (Application     |         | reloadConfig()  |                   | from_toml_layers    |
|  Support/<id>/)  |         +-----------------+                   +---------------------+
+------------------+                                                         |
   optional file             menu "Reload config" or start-up       defaults + override
```

Swift reads the optional file and passes the text over FFI; the engine never touches the filesystem (arch-lint forbids `std::fs`).

- `EngineConfig::from_toml_layers(&[&str])` merges each layer onto the embedded defaults, key by key. Arrays replace wholesale; `[[app_category]]` merges by `name` and `params` merges per key.
- A bad type, out-of-range value or unknown key is dropped with one diagnostic line `config: <key>: <reason>`; the previous value stays. Invalid TOML drops the whole layer. Init never fails.
- Menu item `Reload config` calls `nimk_set_config` and `Engine::reconfigure`. It keeps `policy`, `seed`, `threading` and `models`; other keys are replaced.
- The app category is chosen by `bundle_id` on each focus change; `Engine::idle_commit_secs()` and `nimk_idle_commit_secs` return the effective value.

## Keys

Test `every_field_is_in_default_toml` checks that every `EngineConfig` field is in `default.toml` and the reverse.
"Live" keys apply on `Reload config`; the rest apply at the next start.

| Key | Type | Range | Default | Live | Meaning |
| --- | --- | --- | --- | --- | --- |
| `policy` | `"gru_action"`, `"linear_action"` | enum | `gru_action` | no | Model built when no snapshot exists |
| `seed` | integer | `u64` | 24301 | no | Model init seed |
| `switch_margin` | float | 0 to 0.5 | 0.02 | yes | Mid-word hysteresis: change rendering only if `abs(p - 0.5)` exceeds it |
| `hotkeys` | bool | | true | no | Accept hotkeys and produce implicit labels; replay sets false |
| `implicit_weight` | float | 0 to 1 | 0.3 | no | Weight of implicit labels |
| `edit_online_weight` | float | 0 to 1 | 0.0 | no | Edit-head loss weight online (unused; edit is structural) |
| `correction_words` | integer | 0 to 20 | 2 | no | Words after a commit in which a hotkey still corrects it |
| `label_timeout_secs` | float | 0.5 to 600 | 10 | no | Seconds before an implicit label is final |
| `teach_batch` | integer | 1 to 4096 | 1 | yes | Teacher-forced examples per update |
| `correction_window` | integer | 1 to 1000 | 20 | no | Examples re-trained around a correction |
| `correction_steps` | integer | 0 to 100 | 5 | no | Gradient steps over that window |
| `replay_capacity` | integer | 16 to 1000000 | 10000 | no | Replay buffer size |
| `replay_every` | integer | 0 to 100000 | 50 | no | Train one minibatch per N new examples (0 = off) |
| `replay_batch` | integer | 1 to 4096 | 32 | no | Minibatch size |
| `snapshot_every_secs` | float | 10 to 86400 | 300 | yes | Checkpoint interval |
| `threading` | `"background"`, `"inline"` | enum | `background` | no | Trainer and hooks on background threads or inline |
| `double_shift` | bool | | true | yes | Accept a double Shift tap |
| `double_shift_secs` | float | 0.05 to 2 | 0.4 | yes | Max time between the two releases |
| `shift_tap_secs` | float | 0.05 to 1 | 0.3 | yes | Max Shift down-to-up time for a tap |
| `ctrl_space` | bool | | true | yes | Also accept Ctrl+Space |
| `idle_commit_secs` | float | 0, or 0.3 to 10 | 1.5 | yes | Idle seconds before Swift commits the marked text; 0 = off |
| `extra_letters` | string | no control or whitespace chars | `""` | yes | Characters treated as letters (e.g. `"_"`) |
| `extra_boundaries` | string | no control chars | `""` | yes | Characters treated as boundaries although alphabetic |
| `commit_keycodes` | int array | each 0 to 127 | Return, keypad Enter, Tab, Escape, Home/End/PageUp/PageDown, arrows, forward Delete, F1 to F20 | yes | Keys that commit the word and then reach the app (chords with Cmd/Ctrl/Opt always commit) |
| `command_triggers` | string array | each exactly 1 char, not letter, digit or space | `["/", "@", "!"]` | yes | Characters that move `Idle` to `Raw` |
| `command_end_chars` | string | | `" "` | yes | Characters that end `Raw` (they still reach the app) |
| `command_end_keycodes` | int array | each 0 to 127 | Return, keypad Enter, Tab, Escape, Home/End/PageUp/PageDown, arrows, forward Delete | yes | Keys that end `Raw` (chords also end it) |

### Per-model options (`[models.<kind>]`)

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `models.gru_action.online_training` | bool | true | Train this model online, also while inactive (shadow training, [ADR 0013](adr/0013-shadow-training-and-model-stats.md)); false freezes it |
| `models.linear_action.online_training` | bool | true | Same for the linear model |

Layers merge per model and per key; an unknown model or key, or a wrong type, is dropped with a diagnostic.
The menu toggle is saved in the blob `settings/models` and overrides these values at start.

## Per-app overrides

`[[app_category]]` has `name`, `bundle_ids` (an entry ending in `*` matches by prefix) and a `[app_category.params]` table.
The first matching category wins. `params` may override only the "Live" keys above: `switch_margin`, `idle_commit_secs`, `double_shift`, `double_shift_secs`, `shift_tap_secs`, `ctrl_space`, `extra_letters`, `extra_boundaries`, `commit_keycodes`, `command_triggers`, `command_end_chars`, `command_end_keycodes`.
A bad key in `params` is dropped on its own (`app_category.<name>.params.<key>`); the rest of the category stays.

| Category | `bundle_ids` | `idle_commit_secs` |
| --- | --- | --- |
| `terminal` | `com.github.wez.wezterm`, `com.googlecode.iterm2`, `com.apple.Terminal`, `com.mitchellh.ghostty`, `net.kovidgoyal.kitty`, `org.alacritty`, `dev.warp.Warp-Stable` | 1.0 |
| `ide` | `com.microsoft.VSCode`, `com.todesktop.230313mzl4w4u92` (Cursor), `dev.zed.Zed`, `com.apple.dt.Xcode`, `com.jetbrains.*` | 1.0 |
| `browser` | `com.google.Chrome`, `com.microsoft.edgemac`, `com.apple.Safari`, `org.mozilla.firefox` | 0.8 |
| `editor` | `com.apple.Notes`, `com.apple.TextEdit` | 2.0 |

Example user `config.toml`:

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

The packaging step copies `default.toml` to `Contents/Resources/default-config.toml` for reference only; the real defaults come from the embedded copy.
