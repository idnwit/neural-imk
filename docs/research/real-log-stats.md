# Real-log statistics

> **Historical research note** · **Date:** 2026-11-09 · **Still current:** yes, as the calibration source for `MixProfile::default()` · **Code:** `crates/nimk-synth/examples/log_collisions.rs`

Aggregate numbers from one user's event logs (2026-11-08 to 2026-11-09, 4 files, 23 112 lines, **1319 committed words**). The sample is tiny and early, and almost all words are in terminals. This repo contains no typed text; every number below is a count or a ratio.

## Method

- Each `word_commit` gets a final language: the word's `label` if present (1297 implicit, 19 correction), else the `mode` at commit.
- App group comes from the latest `context` bundle id (terminal, IDE, browser, other).
- A run is a streak of consecutive words in the same language within a session.
- A "collision" is an EN word whose raw keys, passed through Telex, give a valid VI syllable different from raw (`has_telex && valid_vi`).

## Results

| Group | Words | % VI | Mean VI run | Mean EN run | Adjacent same-language | raw = VI rendering | Manual switches / 1000 words |
| --- | --- | --- | --- | --- | --- | --- | --- |
| terminal | 1261 | 71.1 | 4.5 | 1.8 | 0.686 | 30.7% | 15.1 |
| IDE | 11 | 45.5 | n too small | n too small | 0.80 | 27.3% | 0 |
| browser | 17 | 11.8 | n too small | n too small | 0.75 | 70.6% | 176 (3 times) |
| other | 29 | 75.9 | n too small | n too small | 0.71 | 44.8% | 0 |
| all | 1319 | 70.2 | 4.43 | 1.90 | 0.688 | 31.5% | 16.7 (22 times) |

Run length distribution (all groups; runs of length 1, 2, 3, 4+):

| Language | 1 | 2 | 3 | 4+ |
| --- | --- | --- | --- | --- |
| VI | 46 | 40 | 20 | 103 |
| EN | 133 | 45 | 10 | 19 |

- Collisions: 79 of 394 EN words (20.1%), which is 6.0% of all words.
- 47 of 925 VI words (5.1%) have raw keys equal to a word in the generator's EN corpus.
- Telex changes 733 of 925 VI words (79%).
- Short function words make up most occurrences in both languages.

## Use for the generator

| Statistic | Real | `MixProfile::legacy` | `MixProfile::default` |
| --- | --- | --- | --- |
| % VI | 70.2 | 60.4 | 68.9 |
| Run VI / EN | 4.4 / 1.9 | 5.0 / 3.3 | 4.5 / 2.0 |
| Adjacent same-language | 0.688 | 0.759 | 0.695 |
| Collisions / EN word | 20.1% | 40.1% | 19.2% |
| Collisions / all words | 6.0% | 15.9% | 6.0% |
| raw = rendering | 31.5% | 22.3% | 27.2% |

The legacy generator doubled the collision rate because `p_ambiguous = 0.4` forced ambiguous EN words. The default profile matches real data on these metrics (see [04](../04-engine-ml.md)).

## Limits

- 1319 words from one person in the first week; non-terminal groups have only 57 words. Rates are good to a few percentage points.
- The log includes words typed to test the IME, so EN and collision rates may be skewed.
- "Terminal" is mostly chat with a CLI in Vietnamese, not shell commands.
- Real logs contain punctuation and symbols that the generator only produces in its terminal/IDE flavours, so synthetic `raw = rendering` is lower.
