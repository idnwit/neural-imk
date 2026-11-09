# 12. Metrics and ground truth

> **Status:** stable · **Updated:** 2026-11-09 · **Code:** `crates/nimk-types/src/truth.rs`, `crates/nimk-engine/src/trainer/stats.rs`, `crates/nimk-replay/src/metrics.rs`, `crates/nimk-synth/src/eval_report.rs`

One definition of "correct" for every accuracy number, online and offline.

## Ground truth

A suggestion that the user does not correct is correct. Not corrected means taken as is.

```
committed rendering --+-- no correction -----------> truth = committed rendering
                      |
                      +-- user switched language --> truth = the language switched to
```

`nimk_types::ground_truth(committed, correction)` is the only place that decides this, and `nimk_types::word_correct(rendering, truth)` is the only comparison.

What counts as a correction:

- a mid-word switch (double Shift while typing the word);
- a switch while the word is still pending (marked text);
- a late correction of a word that was already finalized (`Labeler::refinalize`).

What counts as not corrected: implicit finalization, i.e. `correction_words` more words committed, or `label_timeout_secs` passed.

## Metrics

| Metric | Correct when | Where |
| --- | --- | --- |
| Word accuracy, active model | committed rendering == truth. Equals `1 - corrected share` | `ModelStats::word_accuracy_*`; the menu shows the corrected share |
| Word accuracy, shadow model | its own decision at the commit step == truth | same, per model |
| Display accuracy | keystrokes whose display decision == truth | `keys_match_*` / `keys_*` |
| Flips | display changes between consecutive keystrokes of a word | `flips_*` |
| Offline word accuracy | decision at commit == labelled language (replay: `correct_words`; synth: `render_correct`) | `nimk-replay`, `nimk-synth` |

Online numbers are kept for the lifetime and for the last 500 words, per model, in the blob `hooks/model_stats/<kind>` (version 2; older blobs start empty; formerly `stats/<kind>`, migrated once). The shadow models are scored on the same words as the active one, so the three are directly comparable. Offline, the labelled language of the replay or synthetic data plays the correction, so the same names and semantics apply.

## Excluded

Not counted by any metric, because there is no language decision to judge:

- raw / command-mode text;
- focus priors (`Target::Focus`);
- words that never got a final label (the labeler flushes pending words at shutdown, so this is only a word lost to overflow);
- for a shadow model: words where it saw no keystroke (it was off).

## Late corrections

A word that was implicitly finalized (counted as correct) can still be corrected afterwards. The correction amends the earlier record, it does not count the word again:

```
record(word)         words +1, correct (truth = committed)
refinalize(word, En) amend: truth = En
                     active: correct -> corrected; each shadow re-evaluated
                     lifetime counters and the window entry (if still in it) adjusted
                     words unchanged
```

The last few records are kept in memory for this; an unknown or forgotten word is ignored. The current engine only re-finalizes focus priors this way (words are corrected while pending), so the path is a safeguard that is covered by unit tests.
