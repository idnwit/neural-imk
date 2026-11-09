# Storage, event log & replay

> **Status:** stable · **Updated:** 2026-11-09 · **Code:** `crates/nimk-storage`, `crates/nimk-types/src/event.rs`, `crates/nimk-replay`, `crates/nimk-synth/src/log.rs`

All durable data (event log, model snapshots, settings) goes through the `Storage` trait and is seen only as bytes.
Encryption can later be added as a decorator without changing callers ([ADR 0004](adr/0004-storage-behind-trait.md)).

## Trait

Serialization (serde JSON) lives in the layer above (the `event_log` hook in `nimk-hook`, run by `nimk-engine::hookbus`); `nimk-storage` depends on no `nimk-*` crate.

```rust
pub struct SegmentId(pub String);

pub trait EventSink: Send {
    fn append(&mut self, record: &[u8]) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}
pub trait EventSource {
    fn segments(&self) -> io::Result<Vec<SegmentId>>;
    fn read_segment(&self, id: &SegmentId) -> io::Result<RecordIter<'_>>;
}
pub trait BlobStore: Send + Sync {
    fn put(&self, key: &str, bytes: &[u8]) -> io::Result<()>;
    fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>>;
    fn list(&self, prefix: &str) -> io::Result<Vec<String>>;
    fn delete(&self, key: &str) -> io::Result<bool>;
    fn origin(&self, key: &str) -> io::Result<Option<BlobOrigin>>; // User | Default
}
pub trait Storage: Send + Sync {
    fn event_sink(&self, session: &str) -> io::Result<Box<dyn EventSink>>;
    fn event_source(&self) -> io::Result<Box<dyn EventSource + '_>>;
    fn blobs(&self) -> &dyn BlobStore;
}
```

| Call | Contract |
| --- | --- |
| `event_sink(session)` | Writer for one session; the sink names and rotates segments, e.g. `2026-11-08_<session>` (string order = time order) |
| `append` | One serialized record, no trailing newline |
| `put` | Atomic (tmp + rename): readers see the old or the new blob |
| `get` | `Ok(None)` if the key is missing |
| `list` | Sorted keys starting with `prefix` |
| `delete` | `Ok(true)` if deleted, `Ok(false)` if absent; read-only stores return `Unsupported` |
| `origin` | Layer that `get` reads: `User` or `Default` (bundled) |

Validation is shared by all backends and fails with `InvalidInput`:

- `session` has odd characters replaced by `_`; an empty one is an error.
- `SegmentId` must match `[A-Za-z0-9_-]+`.
- A record must not contain `\n`.
- A blob key is `/`-joined segments; none may be empty, start with `.`, or contain `\` or NUL.

## Implementations

| Type | Used for | Notes |
| --- | --- | --- |
| `PlainFsStorage::open(root)` | The real IME | Creates `root/events` and `root/blobs` |
| `MemStorage::new()` | Unit tests, replay | Clones share data |
| `ReadOnlyDirBlobs::open(dir)` | Models bundled in the app | Read-only `BlobStore`; `put`/`delete` return `Unsupported` |
| `LayeredStorage::new(user, defaults)` | The real IME | Events go to `user`; `get` reads `user` then `defaults`; writes touch only `user`; `list` is the sorted union |
| `EncryptedStorage<S>` | Not implemented yet | Planned decorator that encrypts each record or blob |

The data dir is `<Application Support>/<bundle id>`, taken from `FileManager` and `Bundle.main.bundleIdentifier`; no path is hard-coded.

```text
<Application Support>/<bundle id>/
  config.toml                     # optional user override (see 11-config.md)
  events/
    2026-11-08_<session>.jsonl    # one segment per day and session, one Record per line
  blobs/
    model/<kind>/latest           # learned snapshot, <kind> = linear_action | gru_action
    hooks/model_stats/<kind>      # per-model stats, model_stats hook (see 04-engine-ml.md)
    hooks/usage/<YYYY-MM-DD>      # per-day usage counts, usage hook
    stats/<kind>                  # legacy per-model stats; migrated once, then unused
    settings/model                # {"model":"linear_action"}, chosen in the menu
    settings/models               # {"online_training":{"gru_action":false}}, menu toggles

NeuralIMK.app/Contents/Resources/models/
  model/<kind>/latest             # pretrained on synthetic data (make models), read-only
```

The diagnostic log is `<Library>/Logs/<bundle id>.log`, also sent to `os.Logger(subsystem: <bundle id>)`.
Online learning writes `model/<kind>/latest` in the user layer; deleting it (`Engine::reset_model`) restores the bundled model.
Per-model stats live in `hooks/model_stats/<kind>` (`StatsTracker` JSON, version 2). When that key is absent and the legacy `stats/<kind>` exists, the hook reads it once at start and writes it to the new key; `reset_model` deletes the legacy blob.

The `usage` hook keeps one blob per local day, `hooks/usage/<YYYY-MM-DD>`: `{"version":1,"date":"2026-11-09","apps":{"<bundle id>":{"vi":n,"en":n,"corrected":n}},"total":{"vi":n,"en":n,"corrected":n}}`. It counts committed words by app and language, and corrections; no text is stored. The day comes from the session's `wall_unix_ms` plus the `t` elapsed since the session's first event after `session_start` (`session_start` itself is logged at `t = 0`, so the uptime clock is anchored on the next record), shifted by `utc_offset_min`. Dates follow the uptime clock, so they can drift if the Mac sleeps during a long session. `nimk_hook_summary(core, "usage")` returns today's blob.

## Event schema

Each log line is one JSON `Record` with common fields `v`, `seq`, `t`, `session`, `type` and a flat payload (`#[serde(flatten)]`).
`SCHEMA_VERSION = 1`; an incompatible change must bump it and update this table.
Logs hold keystrokes, so they are local files only; docs and tools report aggregates.

| Field | Type | Meaning |
| --- | --- | --- |
| `v` | `u32` | Schema version |
| `seq` | `u64` | Sequence in the session, from 0; sort by `(session, seq)` |
| `t` | `f64` | Seconds since boot from the triggering event (`NSEvent.timestamp`); not unique, resets on reboot |
| `session` | `String` | Random id of one IME process run |
| `type` | tag | `Event` variant name, `snake_case` |

| `type` | Own fields | Written when |
| --- | --- | --- |
| `session_start` | `wall_unix_ms`, `app_version`, `config_json`, `policy_id`, `utc_offset_min` (minutes east of UTC at start, host clock; default 0) | First record of a session |
| `context` | `bundle_id`, `field_id`, `caret` (`{x, y}` or `null`) | App or field change, first key of a word |
| `key` | `key` (`KeyEvent`), `word_id` (`u64` or `null`) | Every key the IME receives |
| `decision` | `target`, `at_key`, `policy_id`, `p_vi` (`null` for teacher-forced), `rule` (always `null`), `chosen`, `edit` | Each time the model is asked, plus the focus prior |
| `manual_switch` | `scope` (`in_word`, `after_commit`, `before_typing`), `word_id`, `to` | Switch hotkey |
| `word_commit` | `word_id`, `raw`, `vi`, `en`, `mode`, `boundary` | A word is really inserted into the app |
| `label` | `target`, `label`, `weight`, `source` (`implicit`, `correction`) | A final label |
| `model_update` | `policy_id`, `n_samples`, `loss` | Trainer update |
| `model_switch` | `from`, `to` (e.g. `linear_action@v3`) | The user changes model; replay ignores it |
| `model_score` | `kind`, `word_id`, `category`, `correct`, `display_hits`, `display_total`, `flips`, `amend`, `served` | One per measured model per labelled word (`amend: true` re-scores a late-corrected word); counted by the `model_stats` hook; replay ignores it |
| `model_reset` | `kind` | The user restores a model's bundled default; the `model_stats` hook zeroes its counters; replay ignores it |

- `target` is `{"kind":"word","id":7}` or `{"kind":"focus","id":2}`.
- `edit` is `append`, `pop`, `commit` or `pass` (`null` for the focus prior); `#[serde(default)]` keeps old logs readable.
- `Lang` is `"en"` or `"vi"`. `Boundary` is `space`, `return`, `tab`, `punct`, `navigation`, `shortcut`, `external`. `mods` is the `NSEvent.ModifierFlags` bit set.
- Keys that never reach the model (non-letter with empty buffer, a boundary joining a pending word) produce no `decision`.

A shortened session fragment (placeholder values; one record per line):

```json
{"v":1,"seq":0,"t":5120.0,"session":"4f1c9a","type":"session_start","wall_unix_ms":1794111700000,"app_version":"0.1.0","config_json":"{...}","policy_id":"gru_action@v1"}
{"v":1,"seq":2,"t":5123.4,"session":"4f1c9a","type":"decision","target":{"kind":"focus","id":0},"at_key":0,"policy_id":"gru_action@v1","p_vi":0.41,"rule":null,"chosen":"en","edit":null}
{"v":1,"seq":6,"t":5123.62,"session":"4f1c9a","type":"decision","target":{"kind":"word","id":7},"at_key":3,"policy_id":"gru_action@v1","p_vi":0.12,"rule":null,"chosen":"en","edit":"append"}
{"v":1,"seq":8,"t":5123.7,"session":"4f1c9a","type":"word_commit","word_id":7,"raw":"has","vi":"há","en":"has","mode":"en","boundary":"space"}
{"v":1,"seq":15,"t":5126.1,"session":"4f1c9a","type":"label","target":{"kind":"word","id":7},"label":"en","weight":0.3,"source":"implicit"}
{"v":1,"seq":16,"t":5126.11,"session":"4f1c9a","type":"model_update","policy_id":"gru_action@v1","n_samples":3,"loss":0.13}
```

The log is enough to rebuild a session: replay builds the engine from `session_start`, feeds `context` and `key` as input, and uses `label` as the answer.

## Replay harness

`nimk-replay` re-decides a log with one or more action models and reports metrics.

```sh
nimk-replay --data ~/Library/Application\ Support/NeuralIMK \
            --model linear_action,gru_action --from 2026-11-01 \
            --init /path/to/pretrained --report report.json
```

| Flag | Meaning |
| --- | --- |
| `--data DIR` | Required; `PlainFsStorage` root (holds `events/`) |
| `--model LIST` | Comma list of `linear_action`, `gru_action`; default both |
| `--from PREFIX` | Only segments with an id at or after this prefix |
| `--report FILE` | Write `report.json` |
| `--seed N` | Override `EngineConfig::seed` of every session |
| `--init DIR` | Start from `DIR/blobs/model/<kind>/latest` (e.g. from `nimk-synth pretrain`); without it the model starts untrained, with a warning |
| `--frozen` | Do not feed labels to the engine; labels are only the scoring truth |

| Mechanism | Detail |
| --- | --- |
| Engine | Rebuilt per session from `config_json` (parse error: defaults plus warning); `policy` set by `--model`, `hotkeys = false`, `Threading::Inline` |
| Input | `context` to `on_context`, `key` to `on_key`, `label` to `Engine::inject_label` at its position, `word_commit` with boundary `external` to `commit_pending` |
| Ground truth | The logged `label` records (the last label per target wins) |
| Restart | One `MemStorage` per model lives across sessions; a new `Engine::new` per session mimics production snapshots |
| Prequential | Always: decide at commit first, then learn the label |
| Bad lines | Corrupt or non-v1 lines are skipped and counted |
| Output | stdout table (an `ALL` row per model, then one row per bundle id) and `report.json` with metadata (first 50 warnings) |

### Synthetic logs

`nimk_synth::write_log` writes scripts as a normal event log so models can be benchmarked without real logs ([ADR 0006](adr/0006-synthetic-pretraining.md)).

| Record | Content |
| --- | --- |
| `session_start` | One per script; `session = synth-<seed>-<n>`, `policy_id = "synthetic"`, default config with `hotkeys = true` |
| `context` | One, `bundle_id = "synthetic"` |
| `key` | All keys including boundaries; no hotkeys |
| `label` | One per word, after the key that ends it, `source = correction`, `weight = 1.0` |

- There are no `decision` records, so `edit_acc` is `-` on synthetic logs.
- Every label is a `correction`, so replay pays the correction-window cost per word. Keep synthetic logs in their own storage; mixing them with real logs corrupts the manual-switch metric.
- With `p_typo > 0` scripts contain Backspace and word ids drift from the engine. Use `p_typo > 0` only for `pretrain`, never for `write_log`.

## Metrics

The main metric is manual switches per 1000 words; the others explain why it moves.
Stdout columns: `model scope words labelled acc amb_acc auto_sw auto_prec manual/1k prior_acc edit_acc`.

| Metric | Definition (`nimk-replay/src/{metrics,report}.rs`) | Goal |
| --- | --- | --- |
| `manual/1k` | `(labelled - correct) * 1000 / words`; each wrong word needs one manual fix | Main, toward 0 |
| `acc` | Labelled committed words whose `mode` equals the label | Reference |
| `amb_acc` | Like `acc`, only for ambiguous words (`vi != raw`) | Reference, harder |
| `auto_sw` | Decisions that changed the mode | Reference |
| `auto_prec` | Share of labelled `auto_sw` whose `chosen` equals the label | > 95% |
| `prior_acc` | Share of labelled focus priors that were right | Per-app reference |
| `edit_acc` | Replay edit equals the logged edit for the same `(target, at_key)` | Near 100% (edit is structural) |
| Latency p50/p99 | `nimk_key` time in the real process; replay does not measure it | p99 < 2 ms |

A ratio with a zero denominator prints `-`.
Here "ambiguous" is `raw != vi`; pretrain evaluation also requires `vi` to be a valid syllable ([04](04-engine-ml.md)).

## Replay bias

The implicit label "not corrected means right" was produced under the old model, so replay is only an estimate.
A new model that chooses differently on an uncorrected word is scored wrong even if the user might accept it.
`--frozen` gives a no-online-learning baseline to isolate the effect of online learning.
