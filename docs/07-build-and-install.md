# Build and install

> **Status:** draft · **Updated:** 2026-11-09 · **Code:** `Makefile`, `scripts/`, `Cargo.toml`, `crates/nimk-ffi`, `crates/nimk-synth/src/bin/nimk-synth.rs`

There is no Xcode project. `cargo` builds `libnimk_ffi.a`, `cbindgen` writes the C header, and `swiftc` links `NeuralIMK.app`.
`make check` must pass before every commit.

## Makefile targets

| Target | Runs | Notes |
| --- | --- | --- |
| `check` | `fmt lint test arch` | Pre-commit gate |
| `fmt` | `cargo fmt --all -- --check` | |
| `lint` | `cargo clippy --workspace --all-targets -- -D warnings` | See [08](08-coding-standard.md) |
| `test` | `cargo test --workspace` | |
| `arch` | `cargo arch-lint check`, then `--rules no-panic-in-lib` | Contract in `arch-lint.toml` |
| `swift-test` | `swiftc` + run the tests in `macos/tests/` | `Debouncer`, `IdleCommit`, `StatsText`; not part of `check` |
| `header` | `cbindgen ... --output target/nimk_ffi.h` | C header for Swift |
| `models` | Pretrain both bundled models | See below |
| `app` | `scripts/build-app.sh` | Builds `build/NeuralIMK.app` |
| `install` | `app`, then `scripts/install.sh` | Copies into `~/Library/Input Methods/` |

## Build pipeline

```text
+-------------------+  cargo build   +------------------+
| crates/nimk-ffi   | -------------> | libnimk_ffi.a    | ---+
+-------------------+  --release     +------------------+    |
        |                                                    |  link
        | cbindgen                                           v
        v                            +------------------+  +---------------------------+
+-------------------+  import        | macos/NeuralIMK/ |  | NeuralIMK.app/Contents/   |
| target/nimk_ffi.h | -------------> | *.swift (swiftc) |->|   MacOS/NeuralIMK         |
+-------------------+  objc-header   +------------------+  |   Info.plist, Resources/  |
                                                           +---------------------------+
                                                                  | codesign -s -
                                                                  v
                                                     ~/Library/Input Methods/
```

The generated header is the only contract between Rust and Swift. If you change `nimk-ffi`, rebuild the header or the Swift build fails.

`scripts/build-app.sh` runs these steps in order:

1. Generate the header.
2. Run `make models` if the bundled models are missing.
3. `cargo build --release -p nimk-ffi`, then `swiftc -O` with `Cocoa`, `InputMethodKit` and `CoreGraphics`.
4. Copy the models, a reference `default-config.toml`, and `Info.plist` into the bundle, and render the icon.
5. Sign ad hoc with `codesign -s -`.

Set `ARCHS="arm64 x86_64"` for a universal binary (joined with `lipo`). The default is the host architecture. Minimum macOS is 13.0.

## Build profile

`burn` is about 40x slower without optimization. The root `Cargo.toml` sets `opt-level = 3` for dependencies in dev builds.
Workspace code stays at debug level.

`nimk-policy` pins `burn = "=0.22.1"` with default features off ([ADR 0009](adr/0009-burn-nn-backend.md)). That means no rayon and no GPU backend: kernels run on one thread, which suits an IME process.
The GRU learning test is slow in debug builds. It runs under `cargo test --release -p nimk-policy`, or with `--include-ignored` in debug. The latency test needs `--release --test action_latency -- --ignored`.

## Bundled models

The app ships models pretrained on synthetic data. A fresh install starts from them and keeps learning online ([ADR 0010](adr/0010-sdk-storage-and-bundled-models.md)).

1. `make models` pretrains `linear_action` and `gru_action` (seed 42, 200 sessions x 500 words, 2 epochs) into `target/models/<kind>/`.
2. It copies each checkpoint to `target/models/bundle/model/<kind>/latest`.
3. A kind that is already in `bundle/` is skipped. `make models FORCE=1` retrains it (GRU takes about 100 s).
4. `build-app.sh` copies `bundle/` to `NeuralIMK.app/Contents/Resources/models/`, so the app holds `Contents/Resources/models/model/<kind>/latest`.
5. Output stays under `target/` and is not committed.

At runtime the bundled checkpoint is read-only. Learned checkpoints go to `<Application Support>/<bundle id>/blobs/model/<kind>/latest` and shadow the bundled one.
The input menu item `Khôi phục model gốc` ("Restore original model") deletes the learned checkpoint of the active model and reloads the bundled one.

To pretrain by hand (`--out` is required and never defaults to the data directory):

```sh
cargo run --release -p nimk-synth --bin nimk-synth -- pretrain \
    --model gru_action --out target/models/gru_action --sessions 200 --words 500 --epochs 2
```

The command prints a report (`edit_accuracy`, `render_accuracy`, `ambiguous_acc`, `text_match_rate`); see [04](04-engine-ml.md).

| Subcommand | Purpose | Flags |
| --- | --- | --- |
| `gen` | Write a synthetic event log | `--out --sessions --words --seed` |
| `pretrain` | Train and save a model | the above, plus `--model --epochs --eval-frac --batch` |
| `eval` | Score a saved model on held-out scripts | `--model --init --margins --eval-frac` |

Defaults: 100 sessions, 500 words, seed 42, 2 epochs, `eval-frac` 0.1, batch 128. Retrain whenever the Telex engine or the feature encoding changes.

## Storage locations

The macOS SDK supplies every location; none is hard-coded in Swift.

| What | Where |
| --- | --- |
| Data (checkpoints, event log) | `<Application Support>/<bundle id>` |
| Diagnostics log | `<Library>/Logs/<bundle id>.log` |
| Bundled models | `Bundle.main.resourceURL/models` |
| Installed app | `~/Library/Input Methods/NeuralIMK.app` (set by Apple, used by `install.sh`) |

## `Info.plist`

If a key below is missing or wrong, macOS does not list the IME in Input Sources.

| Key | Value | Note |
| --- | --- | --- |
| `CFBundleIdentifier` | `com.giangndm.inputmethod.NeuralIMK` | Must contain `.inputmethod.` |
| `InputMethodConnectionName` | `com.giangndm.inputmethod.NeuralIMK_Connection` | Passed to `IMKServer` in `main.swift` |
| `InputMethodServerControllerClass` | `InputController` | Swift declares `@objc(InputController)` |
| `LSBackgroundOnly` | `true` | No Dock icon |
| `TISIntendedLanguage` | `vi` | |
| `tsInputMethodCharacterRepertoireKey` | `["Latn"]` | |

## Install

1. `make install` builds, bundles the models (pretraining them if needed), stops the old process, and copies the app.
2. First time only: System Settings, Keyboard, Input Sources, add NeuralIMK.
3. Keep the ABC input source enabled as a fallback.
4. After an update, select NeuralIMK again in the input menu.

## Debug and recovery

| Situation | Action |
| --- | --- |
| Read the debug log | Menu `Show diagnostics log`, or `log stream --predicate 'subsystem == "com.giangndm.inputmethod.NeuralIMK"'` |
| Inspect data and event log | Menu `Open data folder` |
| IME hangs or crashes | Switch to ABC, then `killall NeuralIMK` |
| IME missing from Input Sources | Check `Info.plist`; log out and back in |
| New build has no effect | `killall NeuralIMK`, then select the input source again |
| Model learned badly | Menu `Khôi phục model gốc`; a corrupt checkpoint makes the engine build a fresh model and log it |
| Uninstall | Delete `~/Library/Input Methods/NeuralIMK.app` and remove it from Input Sources |

An error on the key path never loses a key. The engine returns `consumed = false` and the key goes to the app.
