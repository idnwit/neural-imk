# GUI

> **Status:** draft · **Updated:** 2026-11-09 · **Code:** `macos/NeuralIMK/{Overlay,InputController,StatsText}.swift`, `crates/nimk-engine/src/engine.rs`

The app has three UI surfaces, all in the Swift shim because they are short, plain AppKit.
Rust only says when to show and why (`EngineOutput::overlay`, `EngineOutput::mode`); Swift draws.

## Surfaces

| Surface | Implementation | Trigger |
| --- | --- | --- |
| Caret overlay | Borderless `NSPanel`, `.nonactivatingPanel`, level `.statusBar`, `ignoresMouseEvents = true`; placed under the caret rect; shows "VI" or "EN" for about 350 ms, then fades over 600 ms | `overlay = Some(_)`, i.e. the sticky mode changed |
| Menu bar status item | `NSStatusItem` showing "VI" or "EN", with a dot when the model chose the mode (`Overlay::Auto`); tooltip shows the model, e.g. `Model: GRU` | Every `mode` change |
| IME menu | `IMKInputController.menu()` in the system input menu | The user opens the menu |

## Overlay

The overlay must never take focus or block the mouse, because it appears while the user types.

```rust
pub enum Overlay {
    Auto,
    Manual,
}
```

| `Overlay` | Meaning | Color |
| --- | --- | --- |
| `Auto` | The model changed the sticky mode at word end. A focus prior changes only the mode and status item | Blue |
| `Manual` | The user pressed double Shift or Ctrl+Space | Orange |

`NimkOut::overlay` carries `uint8_t` 0 none, 1 auto, 2 manual.

The overlay reads the caret when shown. If the rect is empty it retries once after 80 ms, then falls back to the mouse position.
It reuses the Cocoa caret rect read for the context ([03](03-context-io.md)); no Quartz conversion is needed because `NSPanel` also uses Cocoa coordinates.

## Status item

The `NSStatusItem` is created inside the IME process. This assumes a `LSBackgroundOnly` input method can show one; that is unverified.
If it cannot, the fallback is a separate `LSUIElement` agent app that receives the mode through `DistributedNotificationCenter`.

## IME menu

The menu is built in `InputController.menu()`. Info items are disabled.
IMK calls actions with a `sender` dictionary holding the `NSMenuItem` under `kIMKCommandMenuItemName`, so every action takes `(_ sender: Any?)`. Each selection logs one `menu: ...` line.
Some labels are Vietnamese in the app; their English meaning is given in parentheses.

| Item | Source |
| --- | --- |
| `Mode: Tiếng Việt (VI)` (Vietnamese) / `English (EN)` | `nimk_mode` |
| `Policy: <id>`, e.g. `gru_action@v1` | `nimk_policy_id` |
| `Bản build: 0.1.0 (34) · a1b2c3d · 2026-11-11T10:05` | "Build": version, build number, commit (`+dirty` for uncommitted changes) and local build time, stamped into `Info.plist` (`NIMKGitCommit`, `NIMKBuildTime`) by `scripts/build-app.sh` |
| `Model: <name> ✓ — sửa 1,2% (500 từ gần nhất) · 2,0% tổng (3 450 từ)` | One line per model, at top level (IMK may not deliver submenu actions). `✓` marks the active model (`nimk_model`). "sửa ... (500 từ gần nhất)" means "corrected 1.2% of the last 500 words"; "tổng" means total. Counters come from `nimk_model_stats` and are rebuilt each time the menu opens; text is in `StatsText.swift` |
| Role tag on non-active models | `(chạy ngầm)` = running in the background (shadow training on); `(đóng băng)` = frozen (training off) |
| `(chưa pre-train)` suffix | "not pretrained": no checkpoint exists (`nimk_checkpoint_source` is none) |
| `Học online: GRU ✓` | "Online learning". One toggle per model; `nimk_set_online_training`, saved by the engine in `settings/models` ([ADR 0013](adr/0013-shadow-training-and-model-stats.md)) |
| `Khôi phục model gốc` | "Restore original model": `nimk_reset_model` deletes the active model's user checkpoint and reloads the bundled one; applies `NimkOut`, shows an overlay |
| `Tinh chỉnh với dữ liệu mới… (1 234 từ mới, 12 lần sửa)` | "Fine-tune with new data": the button of [ADR 0016](adr/0016-on-demand-fine-tuning.md). The counts are words with a final label and hotkey corrections since the last run (`nimk_tune_pending`, counted in the background, so the menu shows the previous count and refreshes it for the next opening; "(đang đếm)" = "counting" until the first count ends). Disabled below 200 new words or while a run is going ("Đang tinh chỉnh model…"). Click: `Tuner.start` saves the models (`nimk_tune_prepare`, applies `NimkOut`), fine-tunes the active kind on a utility queue (`nimk_tune_run`, seconds to minutes, typing goes on), then installs the result (`nimk_tune_install`) and shows an overlay (`Đã tinh chỉnh với N từ mới` / `Chưa đủ dữ liệu mới` / `Tinh chỉnh thất bại`). What the model learned online during the run is replaced. Text in `TuneText.swift` |
| `Lần tinh chỉnh gần nhất: 3 giờ trước (2 lần)` | "Last fine-tune: 3 hours ago (2 runs)"; `chưa có` = "none yet" |
| `Reload config` | `Core.reloadConfig()` rereads `<Application Support>/<bundle id>/config.toml` (a missing file is fine), calls `nimk_set_config`, and shows an overlay: `Đã nạp config` ("config loaded") or `Config: N giá trị bị bỏ` ("N values ignored") ([11](11-config.md)) |
| `Mở NeuralIMK…` | "Open NeuralIMK": launches the companion app (`com.giangndm.NeuralIMK`) |
| `Open data folder` | Opens the data dir `<Application Support>/<bundle id>` in Finder |
| `Show diagnostics log` | Opens `<Library>/Logs/<bundle id>.log` |

Choosing a model calls `nimk_set_model`, applies `NimkOut` (committing the word being typed), updates the status item, and shows an overlay with the model name.
Models are described in [linear](models/linear.md) and [gru](models/gru.md). Stats hold counters only, never typed text ([05](05-storage-and-log.md)).

## Companion app notifications

The IME observes `com.giangndm.inputmethod.NeuralIMK.reloadConfig` (distributed) and calls `Core.reloadConfig()` when the companion app rewrites `config.toml`.
It posts `com.giangndm.inputmethod.NeuralIMK.corrected` on every hotkey flip (manual overlay).
For the first three flips (`UserDefaults` key `correctionHintCount`, `CorrectionHint`), the overlay reads `→ <word> · Double-Shift lần nữa để hoàn tác` ("press Double-Shift again to undo") instead of the VI/EN badge; the panel is sized to the text.

## Dashboard (after M4)

A metrics dashboard in `egui` will read the `report.json` from `nimk-replay`. Until then, read metrics from CLI output ([05](05-storage-and-log.md)).
