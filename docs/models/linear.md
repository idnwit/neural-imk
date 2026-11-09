# Linear model (`linear_action`)

> **Status:** stable · **Updated:** 2026-11-09 · **Code:** `crates/nimk-policy/src/{linear_action.rs,encode.rs}`

`LinearAction` is a hand-written linear model over hashed n-gram features. It is tiny, scores in under a microsecond, and learns online with plain SGD. Use it when you want the lowest latency and CPU cost. It is the "fast" choice in the menu ("Linear (nhanh)", `nhanh` = fast).

## What it decides

After ADR 0012 the edit (append, pop, commit) is structural and comes from the state machine. The model only picks the rendering: `p_vi >= 0.5` means Vietnamese (Telex), otherwise English. The 4-way edit head exists and is trained during pretraining, but the engine does not use it.

## Architecture

Input encoding (shared with the GRU, `encode.rs`):

| Part | Content |
| --- | --- |
| Tokens | Last 24 tokens of `recent keys ++ SEP ++ raw chars ++ incoming key`, left-padded |
| Vocab | 139 ids: 128 ASCII bytes plus specials (PAD, SEP, SPACE, RETURN, TAB, BACKSPACE, NAV, SHORTCUT, OTHER, FLAGS, NONE) |
| Scalars | 18 dense features in [-1, 1]: Telex validity flags, mode, previous languages, switch recency, raw length, mask bits, modifiers, has-key (one slot is reserved and always 0) |
| Context | Hashed `bundle_id` (256 buckets) and caret cell (8x8 grid plus unknown = 65) |

The Linear model does not use embeddings. It turns the tokens into hashed sparse ids (FNV-1a, 2^15 buckets):

- positional unigrams of the 4 newest tokens;
- 1 to 4-grams of the current word framed as `^raw$`;
- key x mask, last char x key, key x word-length interactions;
- bag of bigrams of the history before the word;
- app context: bundle, caret, bundle x caret.

```text
 tokens + scalars + bundle + caret
              |
              v
   hashed sparse ids (<= 192)  +  18 dense scalars  +  bias
              |
              v
   weight table: (32768 + 18 + 1) rows x 5 outputs
              | sum of the rows touched
              v
   z[0..4] --softmax--> edit probs (unused)
   z[4]    --sigmoid--> p_vi
```

The model is one weight table, summed over the active rows.

| Property | Value |
| --- | --- |
| Parameters | 163,935 (32,787 rows x 5) |
| Initial weights | All zero, so an untrained model is neutral (`p_vi = 0.5`) |
| Optimizer | Plain SGD; each touched row moves by `lr * grad_sum / sqrt(hits)` |
| Checkpoint size | About 640 KB (f32 weights) |

## Training

**Pretraining** (`nimk-synth pretrain --model linear_action`): behaviour cloning on synthetic typing scripts. Both heads are trained, with step size `LINEAR_LR = 0.1`.

**Online learning:** the model learns from every label the engine finalizes: corrections (double-Shift) and implicit confirmations (uncorrected words, weight `implicit_weight = 0.3`). Online examples train the render head only (`edit_online_weight = 0`) and the step is scaled by `LINEAR_ONLINE_SCALE = 0.03`. The model reports `learns_from_confirmations() = true`.

Trainer settings come from `crates/nimk-engine/config/default.toml`: `correction_window = 20`, `correction_steps = 5`, `replay_capacity = 10000`, `replay_every = 50`, `replay_batch = 32`. A correction re-trains the last 20 samples for 5 steps. Shadow training (`models.linear_action.online_training`, ADR 0013) lets it learn even when GRU is active.

## Checkpoints

| Item | Location |
| --- | --- |
| Bundled | `NeuralIMK.app/Contents/Resources/models/model/linear_action/latest` |
| User (learned) | Blob key `model/linear_action/latest` in the app data folder |
| Per-model stats | Blob key `stats/linear_action` |

The user blob overrides the bundled one. Menu item "Khôi phục model gốc" (restore the original model) drops it. Rebuild the bundled checkpoint with `make models FORCE=1`, then `make install`.

## Results

Measured 2026-11-09 at commit `8f00cfe` on a Mac (release build). Same seed, scripts and held-out split as the bundled model recipe in the `Makefile`.

| Metric | Value |
| --- | --- |
| Edit accuracy | 1.0000 |
| Word accuracy (render accuracy at commit, 10,000 words; 1 - wrong-word rate) | 0.9772 |
| Ambiguous-word accuracy (6,813 words) | 0.9768 |
| Ambiguous English accuracy (560 words) | 0.8768 |
| Display accuracy (mid-word shown rendering vs final label) | 0.8631 |
| Flips per word (visible rendering changes, margin 0.02) | 0.4145 |
| Manual switches per 1000 words, frozen | 21.4 |
| Manual switches per 1000 words, online | 17.0 |
| `score()` latency p50 / p99 | 0.3 us / 1.0 us |
| `update(32)` latency p50 | 16.6 us |

Metric definitions: [12-metrics.md](../12-metrics.md). Reproduce:

```sh
S=/tmp/nimk-eval
# 1. pretrain (same flags as `make models`) and evaluate on the held-out split
cargo run --release -p nimk-synth -- pretrain --model linear_action --out $S/linear_action \
  --seed 42 --sessions 200 --words 500 --epochs 2
cargo run --release -p nimk-synth -- eval --model linear_action --init $S/linear_action \
  --seed 42 --sessions 200 --words 500 --margins 0,0.05,0.2
# 2. manual switches: replay a fresh synthetic log (different seed)
cargo run --release -p nimk-synth -- gen --out $S/log --seed 7 --sessions 20 --words 500
cargo run --release -p nimk-replay -- --data $S/log --init $S/linear_action --model linear_action --frozen
cargo run --release -p nimk-replay -- --data $S/log --init $S/linear_action --model linear_action
# 3. latency (untrained snapshot; the cost does not depend on the weights)
cargo test --release -p nimk-policy --test action_latency -- --ignored --nocapture
```

Notes: manual switches per 1000 = wrong words at commit x 1000 / words in the replay (each wrong word would need one double-Shift). "Frozen" means no online learning; "online" learns while replaying. The eval and replay numbers are synthetic; results on real typing are not measured here.

## Limitations

- Mid-word rendering is the weakest part (0.863): short prefixes carry little evidence, so the shown text flips about 0.41 times per word at margin 0.02.
- Ambiguous English words inside Vietnamese runs (such as `has`, `for`, `user`) are right 87.7% of the time.
- Hashed n-grams memorize words, so unseen words depend on the dense scalars (Telex validity) more than on the text.
- Online learning gives a small gain only at the low step scale; larger steps make it chase the last few words.
