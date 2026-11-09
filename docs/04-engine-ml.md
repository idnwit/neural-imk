# Engine & ML

> **Status:** stable · **Updated:** 2026-11-09 · **Code:** `crates/nimk-types/src/action.rs`, `crates/nimk-policy`, `crates/nimk-engine/src/{config,labeler,trainer}.rs`, `crates/nimk-engine/src/engine/`, `crates/nimk-synth/src/pretrain.rs`

A neural action model scores every key of the word being typed.
Since [ADR 0012](adr/0012-unified-boundary-commit.md) the model only picks the rendering (Vietnamese or English); the edit is structural.
Hotkeys are strong labels, silence is a weak label, and there is no exploration ([ADR 0005](adr/0005-online-supervised-not-exploratory-rl.md), [ADR 0007](adr/0007-neural-action-model.md)).

Per-model architecture, training and measured results: [models/linear.md](models/linear.md), [models/gru.md](models/gru.md), and the comparison table in the [README](README.md).

## Contract

The engine builds an `Observation` for each key, the model returns `ActionScores`, and the engine applies the result.
All types live in `nimk-types/src/action.rs`; `RECENT_WINDOW = 16`.

```rust
pub enum EditAction { Append, Pop, Commit, Pass }
pub struct Action { pub edit: EditAction, pub render: Lang }
pub struct ActionMask { pub append: bool, pub pop: bool, pub commit: bool }

pub struct Observation {
    pub key: Option<KeyEvent>, // None = mode prior on focus change
    pub recent: Vec<KeyEvent>, // up to RECENT_WINDOW previous non-hotkey events
    pub raw: String,
    pub vi: String,            // Telex rendering of raw
    pub valid_vi: bool,
    pub viable_vi: bool,
    pub shown: Lang,
    pub mode: Lang,            // sticky mode the next word starts in
    pub prev_langs: Vec<Lang>, // up to 3 previous words, newest first
    pub bundle_id: String,
    pub caret: Option<CaretRel>,
    pub secs_since_switch: f32, // f32::INFINITY if none
    pub mask: ActionMask,
}

pub struct ActionScores { pub edit: [f32; 4], pub p_vi: f32 }
pub struct Example { pub obs: Observation, pub target: Action,
                     pub edit_weight: f32, pub render_weight: f32 }
```

- The mask comes from the state machine ([02](02-keyboard-io.md)): `Append`, `Pop` or `Commit` is allowed only when the key physically means that. `Pass` is always legal.
- When exactly one structural edit is legal, the engine uses it and ignores the edit scores. `p_vi >= 0.5` means Vietnamese.
- `Example::edit_weight` is 0 online (`edit_online_weight = 0`), so only the render head learns from users.
- Telex output and the `valid_vi` / `viable_vi` flags are features. The model decides whether to trust them ([10-telex](10-telex.md)).

## Decision points

| Point | When | `Observation::key` | Logged `target` |
| --- | --- | --- | --- |
| Prior | `on_context`, when the app or field changes | `None`, mask allows only `Pass` | `Focus(id)`, `at_key = 0`, `edit = null` |
| Per key | Every non-hotkey key that reaches the model | The current key | `Word(id)`, `at_key` = keys in the word after the action |

The decision that counts for a word is the rendering at the `Commit` step.
`word_id` depends only on boundary keys, so replay and synthetic data line up with the log.

## Shared encoding

`nimk_policy::encode::Encoded` turns an `Observation` into fixed-size stack data used by both models.
Per-model use of these inputs is in the model docs.

| Input | Size | Content |
| --- | --- | --- |
| Token sequence | `SEQ_LEN = 24` | `recent ++ [SEP] ++ raw ++ [next key]`, last 24 tokens, left-padded with `PAD` |
| Vocabulary | 139 ids | 128 ASCII ids (id = byte), then `PAD`, `SEP`, `SPACE`, `RETURN`, `TAB`, `BACKSPACE`, `NAV`, `SHORTCUT`, `OTHER`, `FLAGS`, `NONE` |
| Scalars | `SCALARS = 18` | Telex flags, mode, previous languages, switch recency, `raw` length, mask, modifiers, has-key |
| App bucket | `BUNDLE_BUCKETS = 256` | FNV-1a hash of `bundle_id` |
| Caret cell | `CARET_CELLS = 65` | 8x8 grid cell, id 64 = unknown |

Scalar slot 3 (`SHOWN_VI`) is reserved and always 0.
During `Engine::teach`, `shown` is the target rendering of the previous key, so the feature leaked the label.
Changing the encoding invalidates old snapshots.

## How the engine uses the model

```text
 key --> Composer/machine --> Observation --> model.score (snapshot, no lock)
                                                  |
                          mask + structural edit  |  p_vi
                                                  v
                      hysteresis / commit rule --> rendering --> marked text
                                                  |
                          Step{obs, edit} kept    v
                          per word        --> Labeler --> Examples --> Trainer thread
```

The hot path only reads a published snapshot; all learning happens on the trainer thread.

**Rendering rule**

| Situation | Rendering |
| --- | --- |
| Word locked by a mid-word hotkey | The shown rendering, unchanged to the end of the word |
| `Commit` step | `Vi` iff the model's fresh `p_vi >= 0.5` at that step (no hysteresis) |
| Mid-word | Hysteresis: leave the shown rendering only if `abs(p - 0.5) > switch_margin` (default 0.02) |
| Focus prior | Hysteresis around the sticky mode |

Sticky mode changes at commit (to the word's rendering), on focus, and immediately on a hotkey.

**Labels and the correction window**

Each committed word keeps its model steps; a final label turns each step into an `Example` with `render_weight = weight`.
A label becomes final when `correction_words` (2) more words are committed or `label_timeout_secs` (10) pass.

| Case | Label | Weight | Source |
| --- | --- | --- | --- |
| Window ends, no correction | The committed rendering | `implicit_weight` (0.3) | `implicit` |
| Hotkey mid-word (`in_word`) | New rendering | 1.0 | `correction` |
| Hotkey right after commit (`after_commit`) | New rendering | 1.0 | `correction` |
| Hotkey `before_typing` | New sticky mode | 1.0 | `correction` |

The labeler keeps the last 4 finalized words, so a late correction still overrides an implicit label.

```text
 time -->
          w1 commit        w2 commit        w3 commit
              |                |                |
 -------------+----------------+----------------+----------------------->
              |<---- w1 window: until w3 commit or t(w1) + 10 s ---->|
                     ^
                     | double Shift (after_commit): w1 is re-rendered,
                     | label = new rendering, weight 1.0, source = correction
              ^
              | window closes with no hotkey => implicit label, weight 0.3
```

A hotkey inside the window replaces the implicit label with a correction.

**Trainer thread**

The trainer runs in the background (`threading = background`) and publishes snapshots to the engine.
The engine picks up a new snapshot with `try_recv` at a word boundary.

| Step | When | Config |
| --- | --- | --- |
| Normal update | Each final label; all examples of the word in one batch | none |
| Correction replay | A label contradicting the committed rendering: last W examples, N gradient steps | `correction_window` 20, `correction_steps` 5 |
| Replay buffer | Every `replay_every` new examples, one random minibatch | 50, `replay_batch` 32, `replay_capacity` 10000 |
| Publish snapshot | Every 20 updates and after each correction | `PUBLISH_EVERY = 20` |
| Save checkpoint | Every `snapshot_every_secs` (event time) and on shutdown | 300 |

- A confirmation (implicit label, or label equal to the committed rendering) is learned only if the model's `learns_from_confirmations()` is true. Corrections are always learned.
- With `hotkeys = false` (replay, pretrain) no implicit labels are produced; labels arrive through `Engine::inject_label`.

## Model selection and checkpoints

The user picks the model in the IME menu ([06](06-gui.md)); `policy` in `default.toml` is the default (`gru_action`).

| Item | Detail |
| --- | --- |
| Checkpoint | Blob `model/<kind>/latest`, never shared between kinds; bundled defaults are read-only fallbacks ([05](05-storage-and-log.md)) |
| Choice | Blob `settings/model`, `{"model":"linear_action"}`; read first in `Engine::new`, overrides `policy` |
| `switch_model` | Saves the old checkpoint, replaces the trainer, loads the new checkpoint, publishes a snapshot; a missing or corrupt checkpoint gives an untrained model plus a diagnostic |
| Replay, pretrain | Ignore `settings/model` |

Header `NIMK` + kind + version guards each checkpoint; a mismatch is `PolicyError::Corrupt`.
An untrained model outputs `p_vi` near 0.5, so the app bundles pretrained snapshots ([07](07-build-and-install.md#pretrain-model)).

## Shadow training and model stats

Defined in [ADR 0013](adr/0013-shadow-training-and-model-stats.md).
The trainer holds a core for every model kind. Each label's examples go to every core whose `online_training` is on; only the active core publishes snapshots.

- A frozen model (`online_training = false`) is not updated, saved or shadow-scored. The active model is always measured.
- Toggles come from `[models.<kind>]` in `config.toml`; the menu choice is saved in the blob `settings/models` and wins at the next start.
- Shadow scoring runs on the trainer side from Observations the engine pushes per key. The hot path never calls a shadow model.

Counted per committed word with a final label:

| Stat | Definition |
| --- | --- |
| Corrected | Active: the user corrected the word. Shadow: its own fresh decision at the commit step differs from the final label |
| Windows | Last 500 words (`WINDOW`) and lifetime; lifetime corrected-by-category (terminal, ide, browser, editor, other) |
| Display accuracy | Keys whose display decision equals the final label, with each model applying `switch_margin` to its own simulated display |
| Flips | Display changes between consecutive keys of a word |

Stats persist in the blob `stats/<kind>` and reset with the model.
API: `Engine::model_stats(kind)`; FFI `nimk_model_stats`, `nimk_set_online_training`, `nimk_online_training`. Stats are not logged as events.

## Pretraining

The shipped models start from behaviour cloning on synthetic typing ([ADR 0006](adr/0006-synthetic-pretraining.md)).

```rust
pub fn generate(cfg: &SynthConfig, corpus: &Corpus) -> Vec<Script>;
pub fn pretrain(scripts: &[Script], cfg: EngineConfig, opts: PretrainOptions,
                storage: Arc<dyn Storage>) -> io::Result<PretrainReport>;
```

| Step | Detail |
| --- | --- |
| Generate | `Corpus::builtin()` word lists mixed by `MixProfile::default()` (realistic profile): VI/EN runs, fixed phrases, Zipf frequency, per-app flavours (chat, terminal, IDE) |
| Targets | Each `Script` has `keys` and same-index `targets`; the render target is the language of the word |
| Train | One inline engine with `hotkeys = false`; each key goes through `Engine::teach` with weights 1.0 |
| Options | `PretrainOptions { epochs: 2, eval_frac: 0.1, batch: 128 }`; the replay buffer is off and one snapshot is saved at the end |
| Output | Only the blob `model/<kind>/latest` is kept; `PretrainReport` carries counts and `eval` |
| Evaluate | A fresh engine loads the snapshot and runs `on_key` on held-out scripts without learning |

- `SynthConfig` defaults: `seed` 42, 100 sessions x 500 words, `key_interval_secs` 0.12.
- App and position features cannot be learned from synthetic data; they are learned online.
- Run it with `make models`; CLI and install steps are in [07](07-build-and-install.md). Typing variants and `w` rules for the generator are in [10-telex](10-telex.md).
- Real-log calibration of the profile: [research/real-log-stats.md](research/real-log-stats.md).
