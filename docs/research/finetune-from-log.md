# Fine-tuning from the user's log

> **Research note** · **Date:** 2026-11-11 · **Decision:** [ADR 0016](../adr/0016-on-demand-fine-tuning.md) · **Code:** `crates/nimk-tune`, `nimk-synth logeval`

One user's log (2026-11-08 to 2026-11-11, 16 sessions, 15 947 words after reading it back, 190 corrections). Everything below is a count or a ratio; no typed text is stored here.

## Why the model seemed not to learn

Counting words the model later got wrong again after a correction (46 words always corrected the same way):

| n-th typing after the first correction | wrong again |
| --- | --- |
| 1 to 2 | 24% |
| 3 to 5 | 28% |
| 6 to 10 | 21% |
| 11 to 30 | 16% |
| 31 and later | 4.5% |

One correction changed almost nothing; dozens of repetitions were needed. Online learning updates the render head only, with plain SGD at `1e-3`.

## What does not help: learning per word on top of the model

A prequential simulation over the log, with the live model's own `p_vi` per word as the base and an adapter learned online on top. Live model: 171 errors in 15 438 words.

| Adapter | Errors |
| --- | --- |
| per-word bias, SGD | 359 |
| word + character n-grams | 370 |
| word + previous language | 344 |
| margin-based updates (word) | 204 |
| calibrated counts `ln((nVI+a)/(nEN+a))` | 247 |
| hindsight word-majority (lower bound for any word-only rule) | 375 |

124 of the 171 live errors are on words whose minority language is at least 20% of their uses (`sex` 31 EN / 53 VI, `casi` 25 / 18, `teen` 10 / 9, `this` 9 / 8, `deps` 5 / 5). Context decides these, not the word.

## What helps: teaching the log like pre-training teaches scripts

`nimk-synth logeval`: chronological split by words (first 80% to train, last 20% held out, 3 143 words), bundled GRU as the start, one epoch.

| Run | render@commit | mid-word acc | flips/word |
| --- | --- | --- | --- |
| bundled model | 92.75% | 83.1% | 0.457 |
| fine-tuned, 1 epoch | **96.40%** | 89.6% | 0.218 |
| fine-tuned, 4 epochs | 96.12% | 90.4% | 0.225 |
| 2 epochs + 600 synthetic scripts | 95.16% | 88.8% | 0.281 |
| 4 epochs + 1200 synthetic scripts | 95.26% | 88.2% | 0.319 |

The standard error of one number is about 0.33 points (3 143 words).

## Bigger models do not add after fine-tuning

Each model pre-trained from scratch on the same synthetic data (seed 42, 200 sessions, 2 epochs), then fine-tuned 1 epoch on the log. `window` = tokens given to the model.

| Model | window | base | fine-tuned | pre-train time |
| --- | --- | --- | --- | --- |
| GRU 1x64 (shipped) | 24 | 92.75% | 96.40% | 118 s |
| GRU 2x64 | 24 | 93.19% | 96.34% | 243 s |
| GRU 1x128 | 24 | 94.08% | 96.53% | 255 s |
| GRU 2x128 | 24 | 92.43% | 96.12% | 571 s |
| LSTM 2x128 | 24 | 94.18% | 96.60% | 750 s |
| GRU 1x64 | 48 | 94.46% | 96.28% | 235 s |
| GRU 2x64 | 48 | 93.32% | 96.28% | 512 s |
| LSTM 1x128 | 48 | 94.05% | 96.53% | 744 s |

A larger model or a longer window helps the pre-trained model (up to 94.5%), but after fine-tuning every row is within noise of the shipped one. A GRU re-reads the window on every call (the model is stateless per observation), so a longer window also costs more per keystroke (the budget is p99 < 1 ms).

## Limits

- One user; 3 143 test words; one split.
- A word that was not corrected counts as correct (ground truth, [12](../12-metrics.md)), so labels carry the old model's own choices.
- Online replay of the real log (`nimk-replay`) does not reproduce live behavior (68% word accuracy vs about 99% live, 22 000 `inject_label` warnings), so it was not used as a yardstick here.
- Open: a recurrent state carried across words would give long memory at constant cost per key; not tried.
