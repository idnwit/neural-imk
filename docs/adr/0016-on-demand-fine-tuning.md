# 0016. Fine-tuning on the user's own log, on demand

> **Status:** accepted · **Date:** 2026-11-11

## Context

The user felt the model learned little from double-Shift corrections. The log showed why: online learning trains only the render head with a small SGD step ([gru](../models/gru.md)), so one correction barely moves the next decision. Of the live errors, 124 of 171 were on words the user really types in both languages (`sex` 31 EN / 53 VI), which no per-word memory can resolve; a per-word bias or count tried on top of the live model made things worse (204 to 375 errors vs 171). See [finetune-from-log](../research/finetune-from-log.md).

What did help is teaching the model the user's history the way pre-training teaches synthetic scripts: on a chronological split of the real log, one pass over the first 80% raised held-out render accuracy at commit from 92.8% to 96.4%.

## Decision

- **Real logs become scripts.** `nimk-tune` reads an event log back into [`Script`]s (`scripts_from_log`), one per focus run, with the app's bundle id, the engine's own state machine for edit targets, and the word's final language (committed mode, then implicit label, then correction label, then hotkey switch). A word-ending key the log gives no word id (about 3%) is dropped with its word and cuts the run.
- **Fine-tuning is a button, not a schedule.** Nobody yet knows how often it pays off, so the IME menu shows "Tinh chỉnh với dữ liệu mới… (N từ mới, M lần sửa)" and the user decides. Disabled below 200 new words.
- **New data is counted by a watermark.** Blob `tune/state` stores, per session, the next `seq` not yet used. New data is every word label after it. Counting is incremental: the log only grows at the end of its newest segment, so the blob `tune/pending-index` keeps per-segment counts and the byte offset they cover, and a count reads only what was appended since (about 0.25 ms on a 97 MB log; the first count, and the first after a run moves the watermark, read it all, about 0.3 s). The index is only a cache, rebuilt if missing or stale. It still runs in the background and the menu shows the last count. (The first version re-parsed the whole log on every menu opening, which kept a core busy while typing.)
- **The run never touches the live engine.** Sequence: `nimk_tune_prepare` (main thread: save the models so the run reads current weights), `nimk_tune_run` (background queue: teach new scripts plus up to 3000 older words, one epoch, write `tune/<kind>/result`, move the watermark), `nimk_tune_install` (main thread: `Engine::install_tuned` replaces the checkpoints with every result found and reloads). Typing goes on during the run; what the model learns online in that window is replaced by the install.
- **A new crate, `nimk-tune`,** holds scripts, the state machine mirror, log reading, teacher forcing and the run. It sits between `engine` and both `ffi` and `synth`: the FFI must stay thin (`ffi-thin` in `arch-lint.toml`) and must not link the synthetic-data crate.

## Not done

- A bigger model. GRU/LSTM, 1 to 2 layers, hidden 64 to 128, window 24 or 48 tokens: after fine-tuning all scored 96.1 to 96.6% (the noise of a 3143-word test is about 0.3 points), at 2 to 6 times the training time. Reverted; the patch is kept out of the tree because it also changes the snapshot format, which would have discarded every user's learned GRU.
- Mixing synthetic scripts into the run: it lowered the score on the user's log (95.2 to 95.3% vs 96.4%); forgetting of synthetic behavior is not measured.
- A schedule, a low-rank adapter merged back periodically, and a recurrent state carried across words.

## Consequences

- A fine-tune of the real log (15.9k words, 118k keys) takes about 10 s on the GRU.
- The tuned model is installed over the user checkpoint, so "Khôi phục model gốc" still returns to the bundled one.
- The watermark and the result live in blobs next to the model, so they survive restarts and are removed with the data directory.
- The numbers come from one user and one chronological split; labels for uncorrected words are the old model's own choices.
