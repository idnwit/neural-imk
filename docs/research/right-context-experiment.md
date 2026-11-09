# Experiment: value of right context (the next word)

> **Historical research note** · **Date:** 2026-11-09 · **Still current:** yes; no two-word window was built, and the state machine of ADR 0012 keeps one pending word · **Code:** `crates/nimk-synth/examples/right_context.rs`

On synthetic data, adding the next word's keys does not improve the VI/EN decision for the pending word. GRU left+right scored ambiguous 0.975 against 0.976 for left-only, and the Bayes upper bound gains 0.0 on ambiguous and +0.3 on colliding keys. Recommendation: **do not** build a two-word window UX based on this.

## Question

If two words stayed pending, the model would see the next word's keys when settling the previous one. How much better is the pending word's decision than with left context only? The user's bar was at least +1.5 to 2 points on ambiguous words.

## Method

- Data: `generate` with seed 42, 200 sessions x 500 words, default `MixProfile`; the last 20 scripts held out (10 000 words), as in `nimk-synth pretrain`.
- Variants, all with the same GRU (embedding 32, GRU 64, MLP 64, one render head; 3 epochs, Adam 3e-3, batch 128; 2 seeds):

| Variant | Input |
| --- | --- |
| `word` | the word only |
| `left` (today) | last 24 tokens of history + word + boundary |
| `left+right` | `left` + the keys up to the end of the next word (<= 16 tokens) |
| `right` | the word + right part |

- The Bayes upper bound is computed exactly from the generator's probabilities.
- Sets: overall; ambiguous (`text != raw` and valid syllable, 6968 words); collision (keys the generator can emit for both languages, 98 strings, 623 words).

## Results

GRU numbers are 2-seed means; seeds differ by 4-5 points on collision and 0.1-0.5 on ambiguous.

| Method | overall | ambiguous | collision |
| --- | --- | --- | --- |
| Bayes, word only | 0.9862 | 0.9836 | 0.7785 |
| Bayes, left | 0.9887 | 0.9849 | 0.8186 |
| Bayes, left + 1 right word | 0.9889 | 0.9849 | 0.8218 |
| GRU `word` | 0.9820 | 0.9784 | 0.756 |
| GRU `left` | 0.9810 | 0.9762 | 0.791 |
| GRU `left+right` | 0.9792 | 0.9746 | 0.795 |
| GRU `right` | 0.9791 | 0.9744 | 0.778 |

Right context only moves errors from VI-to-EN to EN-to-VI; the total barely changes.

## Why it does not help here

The generator picks a main language per sentence and flips each word independently. A few left words already reveal the main language, and an inserted EN word is always sandwiched between VI words.

```text
generator:  VI VI [EN] VI VI      <- words inserted independently, neighbors say nothing
real use:   VI VI [EN EN EN] VI   <- code-switching in clusters, next word confirms
```

Other synthetic biases: no collocations, uniform word sampling (frequent function words are under-sampled), independent sentences, and a closed vocabulary.

## What if code-switching came in clusters

A 2-state Markov chain (61% VI) gave Bayes accuracy on collision words. `q = 1` means independent words; small `q` means long clusters.

| `q` | Same-language adjacent | left | left + 1 right word |
| --- | --- | --- | --- |
| 1 | 0.523 | 0.786 | 0.786 |
| 0.6 | 0.714 | 0.798 | 0.816 |
| 0.3 | 0.858 | 0.849 | 0.923 |
| 0.15 | 0.929 | 0.914 | 0.964 |

With clusters, right context adds 5-7 points on collisions, and one word captures nearly all of it. On the ambiguous set that is only about +0.4 to 0.6 points, below the bar, unless collisions are 25-30% of ambiguous words in real text.

## Latency

Rescoring the pending word costs one extra `score()` per word: a 40-token row measured median 261 us, p99 280 us (M4 Pro, release), against about 166 us today. The real cost would be UX: two-word marked text, app compatibility ([09](../09-app-compat.md)) and delayed display.

## Recommendation

1. Do not build a two-word window now.
2. For real text the question is open; synthetic data cannot answer it. Re-run with the real collision rate and cluster length from logs.
3. The cheaper alternative, flipping the pending word with double Shift, covers the few remaining errors ([0008](../adr/0008-double-shift-retro-correction.md), [0011](../adr/0011-pending-word-commit.md)).

Rerun: `cargo run --release -p nimk-synth --example right_context -- --sessions 200 --words 500 --epochs 3 --runs 2` (about 6 minutes; `--runs 0` gives only the Bayes bound).

## Rerun on the realistic profile (2026-11-09)

With `MixProfile::default()` (runs, clusters, Zipf; chat sessions only), ambiguous scores were `word` 0.981/0.984, `left` 0.978/0.979, `left+right` 0.980/0.977, `right` 0.979/0.977; collision about 0.88 for all. Right context still did not help (differences within seed noise), because a 3-epoch GRU does not exploit clusters as well as the Bayes bound. The conclusion is unchanged.
