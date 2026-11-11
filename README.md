# NeuralIMK

NeuralIMK is a macOS input method that switches between Vietnamese (Telex) and English by itself. A thin Swift InputMethodKit shim forwards keys to a Rust core. A small neural model decides, for each word, whether to render Vietnamese or English. It learns online from your corrections.

## Quick start

For users: download the DMG from [Releases](https://github.com/idnwit/neural-imk/releases), drag NeuralIMK to Applications, open it and follow the steps. Ctrl+Space is off by default.

For developers:

```sh
make check            # fmt, clippy, tests, architecture lint
make models           # pretrain the bundled Linear and GRU models (GRU takes ~100 s)
make install          # build NeuralIMK.app and copy it to ~/Library/Input Methods
```

Then open System Settings > Keyboard > Input Sources > Edit, click "+", add NeuralIMK, and select it from the input menu. Details: [docs/07-build-and-install.md](docs/07-build-and-install.md).

## How to use

- Type Telex as usual. The model picks Vietnamese or English for each word.
- Wrong rendering? Tap Shift twice. The current or last word flips, and the model learns from the correction. Ctrl+Space is optional and off by default; set `ctrl_space = true` in `config.toml` ([docs/11-config.md](docs/11-config.md)) to enable it.
- The menu bar item shows the mode. Its menu lets you pick the model, toggle online learning per model ("Học online" = learn online), restore the original model ("Khôi phục model gốc"), reload the config, and open the data folder.
- Keys after `/`, `@` or `!` at the start of a word go straight to the app (raw command mode).

## Integration

### Terminals: don't bind Shift+Enter (Claude Code, Codex)

Terminal key bindings run before the input method. If you bind Shift+Enter, NeuralIMK never sees the key, and the word you are typing stays uncommitted.

Use the kitty keyboard protocol instead. The terminal then sends Shift+Enter (`\x1b[13;2u`) to Claude Code and Codex itself, after NeuralIMK has committed the word.

WezTerm (`~/.wezterm.lua`):

```lua
config.enable_kitty_keyboard = true
-- No config.keys entry for Shift+Enter: a SendString binding bypasses the IME.
```

Verified in WezTerm on 2026-11-09. In other terminals, remove any custom Shift+Enter mapping and turn on their kitty or CSI-u keyboard mode. See [docs/09-app-compat.md](docs/09-app-compat.md).

## Models

| | [Linear](docs/models/linear.md) | [GRU](docs/models/gru.md) |
| --- | --- | --- |
| Idea | Hashed n-gram features, linear heads | Char-level GRU with context embeddings |
| Parameters | 163,935 | 35,053 |
| Learns online from | Corrections and confirmations | Corrections only |
| Word accuracy (held-out synthetic) | 97.72% | 98.07% |
| Ambiguous-word accuracy | 97.68% | 97.74% |
| Ambiguous English accuracy | 87.68% | 83.39% |
| Display accuracy (mid-word) | 86.31% | 91.79% |
| Flips per word | 0.41 | 0.24 |
| Manual switches per 1000 words (frozen / online) | 21.4 / 17.0 | 16.3 / 17.9 |
| `score()` latency p50 / p99 | 0.3 us / 1.0 us | 161 us / 183 us |
| Default | no | yes |

Measured on 2026-11-09 at commit `8f00cfe` on synthetic data only; commands are in each model doc. Metric definitions: [docs/12-metrics.md](docs/12-metrics.md). GRU is the default because it is steadier mid-word; Linear is the cheapest and better on short English words inside Vietnamese text.

## Documentation

See [docs/README.md](docs/README.md) for the docs index, writing standard and ADRs.
