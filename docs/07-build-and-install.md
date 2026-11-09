# Build and install

> **Status:** draft · **Updated:** 2026-11-09 · **Code:** `Makefile`, `scripts/`, `macos/App`, `Cargo.toml`, `crates/nimk-ffi`, `crates/nimk-synth/src/bin/nimk-synth.rs`

There is no Xcode project. `cargo` builds `libnimk_ffi.a`, `cbindgen` writes the C header, and `swiftc` links the input method and the companion app.
`build/NeuralIMK.app` is the companion app (`com.giangndm.NeuralIMK`, SwiftUI) with the input method nested inside it.
`make check` must pass before every commit.

## Makefile targets

| Target | Runs | Notes |
| --- | --- | --- |
| `check` | `fmt lint test arch` | Pre-commit gate |
| `fmt` | `cargo fmt --all -- --check` | |
| `lint` | `cargo clippy --workspace --all-targets -- -D warnings` | See [08](08-coding-standard.md) |
| `test` | `cargo test --workspace` | |
| `arch` | `cargo arch-lint check`, then `--rules no-panic-in-lib` | Contract in `arch-lint.toml` |
| `swift-test` | `swiftc` + run the tests in `macos/tests/` | `Debouncer`, `IdleCommit`, `StatsText`, and the app's pure logic in `macos/Shared/`; not part of `check` |
| `header` | `cbindgen ... --output target/nimk_ffi.h` | C header for Swift |
| `models` | Pretrain both bundled models | See below |
| `app` | `scripts/build-app.sh` | Builds `build/NeuralIMK.app` (companion app with the IME nested) |
| `dmg` | `scripts/package-dmg.sh` | Universal (arm64 + x86_64) `build/NeuralIMK-<version>.dmg`; see Release |
| `install` | `app`, then `scripts/install.sh` | Copies the nested IME into `~/Library/Input Methods/`; `INSTALL_APP=1` also copies the app to `/Applications` |

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
                                                                  | nested at Contents/Library/InputMethods/
                                                                  v
                                                     +---------------------------+  codesign   +-------------+
                                                     | build/NeuralIMK.app       | ----------> | .dmg        |
                                                     | (companion app wraps IME) |  (inside-out)| make dmg   |
                                                     +---------------------------+             +-------------+
```

The generated header is the only contract between Rust and Swift. If you change `nimk-ffi`, rebuild the header or the Swift build fails.

`scripts/build-app.sh` runs these steps in order:

1. Generate the header.
2. Run `make models` if the bundled models are missing.
3. IME: `cargo build --release -p nimk-ffi`, then `swiftc -O` with `Cocoa`, `InputMethodKit` and `CoreGraphics` (plus `macos/Shared/CorrectionHint.swift`).
4. Copy the models, a reference `default-config.toml`, and `Info.plist` into `build/ime/NeuralIMK.app`, and render the menu icon.
5. App: `swiftc -O -parse-as-library` over `macos/App/*.swift` and `macos/Shared/*.swift` with `SwiftUI`, `Charts`, `Carbon` and `AppKit`.
6. Assemble `build/NeuralIMK.app`: the app binary, `macos/App/Info.plist`, `Resources/AppIcon.icns` (rendered by `scripts/make-app-icon.swift`, converted with `iconutil`), and a copy of the IME at `Contents/Library/InputMethods/NeuralIMK.app`.
7. Sign inside-out: the nested IME first, then the app (no `--deep`), then `codesign --verify --strict`. Ad hoc (`codesign -s -`) by default; with `NIMK_SIGN_IDENTITY` set, signs with that Developer ID identity, hardened runtime and timestamp.

`CFBundleVersion` of both bundles is `git rev-list --count HEAD`, a build number that only grows. Release builds need a full (non-shallow) clone, or the count is wrong. The installer compares it to decide whether to update; `CFBundleShortVersionString` stays semantic (`0.1.0`).

```text
build/NeuralIMK.app                              com.giangndm.NeuralIMK
  Contents/MacOS/NeuralIMK                       companion app (SwiftUI)
  Contents/Resources/AppIcon.icns
  Contents/Library/InputMethods/NeuralIMK.app    the IME, com.giangndm.inputmethod.NeuralIMK
```

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
| Installed IME | `~/Library/Input Methods/NeuralIMK.app` (set by Apple; written by the companion app and `install.sh`) |
| Companion app log | `<Library>/Logs/com.giangndm.NeuralIMK.log` (install errors) |

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

| Path | For | How |
| --- | --- | --- |
| `make install` | Developers | Builds and copies the IME to `~/Library/Input Methods/` |
| DMG | Users | Drag `NeuralIMK.app` to Applications, open it |

Companion app (end users): open `NeuralIMK.app`. On every launch it compares the nested IME's `CFBundleVersion` with the one in `~/Library/Input Methods`. If the IME is missing or older, the app:

1. copies the nested IME to a temporary bundle in `~/Library/Input Methods` (same volume, so the swap is a rename);
2. asks the running IME to quit, then sends SIGTERM (the IME flushes its engine) and waits up to 2 s;
3. replaces the installed bundle;
4. registers and enables the input source (`TISRegisterInputSource`, `TISEnableInputSource`).

Errors appear on the Overview screen with a "Thử lại" (Try again) button and go to the app's log.

Developer flow (`make install`):

1. `make install` builds, bundles the models (pretraining them if needed), stops the old process, and copies the nested IME. `INSTALL_APP=1 make install` also copies the app to `/Applications`.
2. First time only: System Settings, Keyboard, Input Sources, add NeuralIMK.
3. Keep the ABC input source enabled as a fallback.
4. After an update, select NeuralIMK again in the input menu.

## Release

`make dmg` runs `scripts/package-dmg.sh`: a universal build (`ARCHS="arm64 x86_64"`; needs `rustup target add x86_64-apple-darwin`), staged with an `Applications` symlink, packed into `build/NeuralIMK-<CFBundleShortVersionString>.dmg`. With `create-dmg` installed (`brew install create-dmg`) the window gets a background and icon layout (`scripts/make-dmg-background.swift`); otherwise the script falls back to a plain `hdiutil` DMG. Use a full (non-shallow) clone so the build number is right.

Requirements for a distributable build:

- Apple Developer Program membership and a Developer ID Application certificate in the keychain.
- A notarytool profile: `xcrun notarytool store-credentials <profile>`.

```sh
# Ad hoc (local testing): no env needed
make dmg

# Signed and notarized
NIMK_SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" \
NIMK_NOTARY_PROFILE=<profile> make dmg
```

`NIMK_SIGN_IDENTITY` signs the apps (hardened runtime, timestamp) and the DMG. `NIMK_NOTARY_PROFILE` submits the DMG to Apple and staples the ticket; without it, notarization is skipped.

An ad hoc build is not notarized. Gatekeeper blocks it on other Macs until the user opens System Settings, Privacy & Security, and clicks Open Anyway.

### Manual release checklist

Use a fresh macOS user account.

1. Open the DMG and drag `NeuralIMK.app` to Applications.
2. Open the app and go through onboarding steps 1 to 4.
3. Type in TextEdit and confirm Telex works and the mode switches.
4. Re-drag a newer build over the old one and open it once: the IME updates (Overview shows the new version).
5. Check the Ctrl+Space warning appears only when the macOS shortcut for the previous input source is on.
6. Uninstall from the app's settings and confirm the IME is gone from `~/Library/Input Methods` and Input Sources.

## Debug and recovery

| Situation | Action |
| --- | --- |
| Read the debug log | Menu `Show diagnostics log`, or `log stream --predicate 'subsystem == "com.giangndm.inputmethod.NeuralIMK"'` |
| Inspect data and event log | Menu `Open data folder` |
| IME hangs or crashes | Switch to ABC, then `pkill -f "Input Methods/NeuralIMK.app/Contents/MacOS/NeuralIMK"` (not `killall`: the companion app shares the executable name) |
| IME missing from Input Sources | Check `Info.plist`; log out and back in |
| New build has no effect | `pkill -f "Input Methods/NeuralIMK.app/Contents/MacOS/NeuralIMK"` (not `killall`: the companion app shares the executable name), then select the input source again |
| Model learned badly | Menu `Khôi phục model gốc`; a corrupt checkpoint makes the engine build a fresh model and log it |
| Uninstall | Delete `~/Library/Input Methods/NeuralIMK.app` (and `/Applications/NeuralIMK.app`) and remove it from Input Sources |

An error on the key path never loses a key. The engine returns `consumed = false` and the key goes to the app.
