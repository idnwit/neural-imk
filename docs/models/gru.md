# GRU model (`gru_action`)

> **Status:** stable · **Updated:** 2026-11-09 · **Code:** `crates/nimk-policy/src/{gru_action.rs,gru_net.rs,encode.rs}`

`GruAction` is a small character-level GRU built on the `burn` CPU backend. It reads the typed characters in order, so it handles mid-word decisions better than the Linear model. It is the default (`policy = "gru_action"`). Use it unless you need the lowest CPU cost.

## What it decides

Same as the Linear model: the edit is structural (ADR 0012) and the model only picks the rendering through `p_vi`. The edit head exists, is trained during pretraining, and is not used by the engine.

## Architecture

The input encoding is shared with the Linear model: 24 tokens, a 139-id vocab, 18 scalars, a hashed bundle id (256 buckets) and a caret cell (65). See [linear.md](linear.md) for the encoding table.

```text
tokens  [B,24] -> Embedding(139, 32) -> GRU(64) -> last hidden [B,64] --+
scalars [B,18] -------------------------------------------------------+
bundle  [B,1]  -> Embedding(256, 16) ----------------------------------+-> concat [B,106]
caret   [B,1]  -> Embedding(65, 8) ------------------------------------+
  -> Linear(106, 64) + ReLU -+-> edit:   Linear(64, 4)  (softmax; unused)
                             +-> render: Linear(64, 1)  (sigmoid = p_vi)
```

Both heads share one hidden layer (the trunk).

| Property | Value |
| --- | --- |
| Parameters | 35,053 |
| Initial weights | Seeded (splitmix64): embeddings U(-0.5, 0.5), Xavier matrices, zero biases and zero heads, so `p_vi = 0.5` |
| Optimizer | Adam, `GRU_LR = 3e-3` for pretraining; plain SGD, `GRU_ONLINE_LR = 1e-3` for online batches |
| Regularizer | Context dropout while training: 1/4 of examples keep only the incoming key, otherwise each context token becomes `OTHER` with probability 1/10 |
| Threading | Runs on the calling thread (no burn thread pool) |

## Training

**Pretraining** (`nimk-synth pretrain --model gru_action`): behaviour cloning on synthetic scripts, both heads, Adam. About 100 s for the bundled recipe.

**Online learning:** the model learns from corrections only (double-Shift). Labels that merely confirm what was committed are dropped (`learns_from_confirmations() = false`), because on the shared dense layer they act as a bias toward the last word's language. Online batches have `edit_weight = 0`: only the render head gets a gradient, the trunk is frozen for them, and they use SGD. Corrections re-train the last `correction_window = 20` samples for `correction_steps = 5` steps; the replay buffer holds 10,000 samples and trains a batch of 32 every 50 samples (`default.toml`). Shadow training (`models.gru_action.online_training`, ADR 0013) lets it learn while Linear is active.

Saved checkpoints hold weights only; Adam state is not saved.

## Checkpoints

| Item | Location |
| --- | --- |
| Bundled | `NeuralIMK.app/Contents/Resources/models/model/gru_action/latest` |
| User (learned) | Blob key `model/gru_action/latest` in the app data folder |
| Per-model stats | Blob key `stats/gru_action` |

The user blob overrides the bundled one. "Khôi phục model gốc" (restore the original model) in the menu drops it. Rebuild with `make models FORCE=1`, then `make install`.

## Results

Measured 2026-11-09 at commit `8f00cfe` on a Mac (release build), same recipe and held-out split as the Linear model.

| Metric | Value |
| --- | --- |
| Edit accuracy | 1.0000 |
| Word accuracy (render accuracy at commit, 10,000 words; 1 - wrong-word rate) | 0.9807 |
| Ambiguous-word accuracy (6,813 words) | 0.9774 |
| Ambiguous English accuracy (560 words) | 0.8339 |
| Display accuracy (mid-word shown rendering vs final label) | 0.9179 |
| Flips per word (visible rendering changes, margin 0.02) | 0.2390 |
| Manual switches per 1000 words, frozen | 16.3 |
| Manual switches per 1000 words, online | 17.9 |
| `score()` latency p50 / p99 | 161 us / 183 us |
| `update(32)` latency p50 | 3.5 ms |

Metric definitions: [12-metrics.md](../12-metrics.md). Reproduce: same as [linear.md](linear.md#results) with `--model gru_action` and `$S/gru_action` (pretraining takes about 100 s).

```sh
cargo run --release -p nimk-synth -- pretrain --model gru_action --out $S/gru_action \
  --seed 42 --sessions 200 --words 500 --epochs 2
cargo run --release -p nimk-synth -- eval --model gru_action --init $S/gru_action \
  --seed 42 --sessions 200 --words 500 --margins 0,0.05,0.2
cargo run --release -p nimk-replay -- --data $S/log --init $S/gru_action --model gru_action --frozen
cargo run --release -p nimk-replay -- --data $S/log --init $S/gru_action --model gru_action
```

Notes: numbers are synthetic; real typing is not measured here. Latency is measured on an untrained snapshot, which costs the same as a trained one.

## Limitations

- Ambiguous English words inside Vietnamese runs are right only 83.4% of the time, lower than Linear (87.7%).
- In the replay, online learning did not help (16.3 frozen, 17.9 online manual switches per 1000): learning from corrections only is conservative and the synthetic log has few real corrections.
- Scoring costs about 160 us, roughly 500 times Linear, still well under the 1 ms budget.
- Online updates cost about 3.5 ms per batch of 32, done on the trainer thread.
