# 0010. SDK-defined storage locations and bundled pretrained models

> **Status:** accepted · **Date:** 2026-11-08

## Context

The previous version hard-coded `~/Library/Application Support/NeuralIMK` and `~/Library/Logs/NeuralIMK.log` in Swift, and made users copy the pretrained model by hand.
macOS defines these directories via `FileManager`, and an app can ship read-only resources in its bundle.

## Decision

- Swift gets the data dir from `FileManager` (`.applicationSupportDirectory` plus the bundle identifier), the log from `Library/Logs`, and the bundled models from `Bundle.main.resourceURL/models`. No hard-coded paths. Diagnostics also go through `os.Logger`.
- `nimk-storage` adds `LayeredStorage<U, D>` (writes go to the user layer, reads fall through to the default layer), `ReadOnlyDirBlobs`, `BlobStore::delete` and `BlobStore::origin`.
- `make models` pretrains both models (seed 42, 200 x 500, 2 epochs) into `target/models/bundle`; `scripts/build-app.sh` copies them to `Contents/Resources/models`. Generated files are not committed.
- The engine's load logic is unchanged: `get` falls through to the bundled model. It adds `checkpoint_source` (`User`/`Bundled`/`None`) for the menu and `reset_model`, which deletes the user checkpoint. FFI: `nimk_new(data_dir, models_dir, app_version)`, `nimk_reset_model`, `nimk_checkpoint_source`.

## Consequences

- First run starts from the pretrained model, learns online, and saves improved checkpoints to Application Support. The menu can restore the bundled model.
- The app grows by about 0.8 MB; the first build takes about 100 s longer for GRU pretraining (cached in `target/`).
- Moving `PlainFsStorage` only needs a different `data_dir` string from Swift.
- Exception: `scripts/install.sh` uses `$HOME/Library/Input Methods`, the location Apple defines for input methods.
- The bundled model is fixed per build; a learned model does not update on app upgrade, so users must restore it.
