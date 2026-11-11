//! Runtime model selection: per-model checkpoints, statistics and the
//! persisted choices.
//!
//! Each [`ModelKind`] keeps its own snapshot at `model/<kind>/latest`; its
//! counters belong to the `model_stats` hook (`hooks/model_stats/<kind>`,
//! formerly `stats/<kind>`). The chosen kind is remembered in the
//! blob [`SETTINGS_KEY`] and the per-model online-training toggles of the
//! IME menu in [`MODELS_KEY`]; [`Engine::new`] reads both back.

use nimk_policy::ModelKind;
use nimk_storage::{BlobOrigin, Storage};
use nimk_types::{Boundary, Event};

use super::{
    Engine, EngineOutput, ModelStats, Started, TrainerOut, load_all, snapshot_key, start_trainer,
};
use crate::config::ModelsConfig;
use crate::trainer::Trainer;

/// Blob where a fine-tuning run leaves the tuned snapshot of `kind` for
/// [`Engine::install_tuned`].
#[must_use]
pub fn tuned_key(kind: ModelKind) -> String {
    format!("tune/{}/result", kind.name())
}

/// Blob holding the selected model, `{ "model": "linear_action" }`.
const SETTINGS_KEY: &str = "settings/model";

/// Blob holding the menu's online-training toggles,
/// `{ "online_training": { "gru_action": false } }`. Only models the user
/// toggled appear; the others follow `config.toml` / the defaults.
const MODELS_KEY: &str = "settings/models";

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct ModelsSetting {
    #[serde(default)]
    online_training: std::collections::BTreeMap<String, bool>,
}

fn read_models_blob(storage: &dyn Storage, diagnostics: &mut Vec<String>) -> ModelsSetting {
    match storage.blobs().get(MODELS_KEY) {
        Ok(None) => ModelsSetting::default(),
        Ok(Some(bytes)) => match serde_json::from_slice(&bytes) {
            Ok(saved) => saved,
            Err(e) => {
                diagnostics.push(format!("{MODELS_KEY} invalid ({e}); using config"));
                ModelsSetting::default()
            }
        },
        Err(e) => {
            diagnostics.push(format!("cannot read {MODELS_KEY} ({e}); using config"));
            ModelsSetting::default()
        }
    }
}

/// Applies the persisted toggles over `models` (the menu wins over the
/// config file).
pub(super) fn read_models_setting(
    storage: &dyn Storage,
    models: &mut ModelsConfig,
    diagnostics: &mut Vec<String>,
) {
    let saved = read_models_blob(storage, diagnostics);
    for kind in ModelKind::ALL {
        if let Some(on) = saved.online_training.get(kind.name()) {
            models.set_online_training(kind, *on);
        }
    }
}

/// Where the checkpoint of a model kind would be loaded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointSource {
    /// Saved by this installation (learned online or kept from a switch).
    User,
    /// Shipped read-only with the app (pretrained on synthetic data).
    Bundled,
    /// No checkpoint: the model starts from random initial weights.
    None,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ModelSetting {
    model: ModelKind,
}

/// Reads the persisted model choice. Absent is silent; unreadable or
/// invalid is ignored with a note in `diagnostics`.
pub(super) fn read_setting(
    storage: &dyn Storage,
    diagnostics: &mut Vec<String>,
) -> Option<ModelKind> {
    match storage.blobs().get(SETTINGS_KEY) {
        Ok(None) => None,
        Ok(Some(bytes)) => match serde_json::from_slice::<ModelSetting>(&bytes) {
            Ok(s) => Some(s.model),
            Err(e) => {
                diagnostics.push(format!("{SETTINGS_KEY} invalid ({e}); using default model"));
                None
            }
        },
        Err(e) => {
            diagnostics.push(format!(
                "cannot read {SETTINGS_KEY} ({e}); using default model"
            ));
            None
        }
    }
}

impl Engine {
    /// Kind of the model currently serving predictions.
    #[must_use]
    pub fn model_kind(&self) -> ModelKind {
        self.cfg.policy
    }

    /// Whether any snapshot (user or bundled) exists for `kind`.
    #[must_use]
    pub fn has_checkpoint(&self, kind: ModelKind) -> bool {
        self.checkpoint_source(kind) != CheckpointSource::None
    }

    /// Layer the snapshot `model/<kind>/latest` would be loaded from.
    /// An unreadable store reads as [`CheckpointSource::None`].
    #[must_use]
    pub fn checkpoint_source(&self, kind: ModelKind) -> CheckpointSource {
        match self.storage.blobs().origin(&snapshot_key(kind)) {
            Ok(Some(BlobOrigin::User)) => CheckpointSource::User,
            Ok(Some(BlobOrigin::Default)) => CheckpointSource::Bundled,
            Ok(None) | Err(_) => CheckpointSource::None,
        }
    }

    /// Discards what the active model learned: commits a word being composed
    /// (the returned output carries it), deletes the user checkpoint of the
    /// active kind and reloads the model from the bundled default, or fresh
    /// when there is none. A diagnostic records the outcome.
    pub fn reset_model(&mut self, timestamp: f64) -> EngineOutput {
        let mut out = EngineOutput::default();
        self.tick(timestamp);
        self.commit_word(Boundary::External, timestamp, &mut out);
        self.flush_taught();
        let mut finals = Vec::new();
        self.labeler.flush(&mut finals);
        self.emit_labels(timestamp, finals);

        let kind = self.cfg.policy;
        // The old trainer saves on shutdown, which must happen before the
        // delete. A throwaway trainer over scratch storage holds the slot.
        let scratch = Trainer::new(Vec::new(), false);
        let old = std::mem::replace(&mut self.trainer, scratch);
        self.finish_trainer(old, timestamp);

        // Its counters restart with it: the `model_stats` hook zeroes them
        // on this record. The legacy blob goes too, so it is never migrated
        // again.
        self.hooks.log(
            timestamp,
            Event::ModelReset {
                kind: kind.name().into(),
            },
        );
        let legacy = format!("stats/{}", kind.name());
        if let Err(e) = self.storage.blobs().delete(&legacy) {
            self.diagnostics
                .push(format!("reset {kind:?}: cannot delete {legacy} ({e})"));
        }
        let key = snapshot_key(kind);
        let note = match self.storage.blobs().delete(&key) {
            Ok(true) => format!("reset {key}: user checkpoint deleted"),
            Ok(false) => format!("reset {key}: no user checkpoint"),
            Err(e) => format!("reset {key}: cannot delete user checkpoint ({e})"),
        };
        self.diagnostics.push(note);

        self.restart_trainer(kind);
        self.last_snapshot_t = Some(timestamp);
        // Zero the cached counters now. Inline, the hook's summary (drained
        // below) already agrees. In background mode the hook thread may not
        // have seen the reset yet, so a summary it published before the
        // reset can briefly restore the old counters; its next summary
        // (after the `model_reset` record) brings them back to zero.
        if let Some(s) = self.stats.get_mut(kind.index()) {
            *s = ModelStats::default();
        }
        self.drain_hooks();
        out
    }

    /// Settles pending state and stops the trainer, which saves its models.
    /// A throwaway trainer holds the slot until [`Engine::restart_trainer`].
    fn settle_and_stop(&mut self, timestamp: f64, out: &mut EngineOutput) {
        self.tick(timestamp);
        self.commit_word(Boundary::External, timestamp, out);
        self.flush_taught();
        let mut finals = Vec::new();
        self.labeler.flush(&mut finals);
        self.emit_labels(timestamp, finals);
        let scratch = Trainer::new(Vec::new(), false);
        let old = std::mem::replace(&mut self.trainer, scratch);
        self.finish_trainer(old, timestamp);
    }

    /// Saves what the models learned so far to their checkpoints now, so a
    /// separate fine-tuning run reads the current weights. Commits a word
    /// being composed (the returned output carries it).
    pub fn checkpoint_models(&mut self, timestamp: f64) -> EngineOutput {
        let mut out = EngineOutput::default();
        self.settle_and_stop(timestamp, &mut out);
        let kind = self.cfg.policy;
        self.restart_trainer(kind);
        self.last_snapshot_t = Some(timestamp);
        self.drain_hooks();
        out
    }

    /// Installs every tuned snapshot a fine-tuning run left in
    /// [`tuned_key`] (whatever model is active now): each replaces its
    /// kind's checkpoint, then the models reload. What a model learned
    /// online since the run read its checkpoint is replaced. Returns `false`
    /// (nothing changes) when no kind has a result. A diagnostic records
    /// every outcome.
    pub fn install_tuned(&mut self, timestamp: f64) -> (bool, EngineOutput) {
        let mut out = EngineOutput::default();
        let mut found = Vec::new();
        for kind in ModelKind::ALL {
            let tuned = tuned_key(kind);
            match self.storage.blobs().get(&tuned) {
                Ok(Some(bytes)) => found.push((kind, tuned, bytes)),
                Ok(None) => {}
                Err(e) => self
                    .diagnostics
                    .push(format!("install {tuned}: cannot read ({e})")),
            }
        }
        if found.is_empty() {
            return (false, out);
        }
        self.settle_and_stop(timestamp, &mut out);
        let mut installed = false;
        for (kind, tuned, bytes) in found {
            let key = snapshot_key(kind);
            let note = match self.storage.blobs().put(&key, &bytes) {
                Ok(()) => {
                    installed = true;
                    if let Err(e) = self.storage.blobs().delete(&tuned) {
                        self.diagnostics
                            .push(format!("install {tuned}: cannot delete ({e})"));
                    }
                    format!("install {key}: tuned snapshot installed")
                }
                Err(e) => format!("install {key}: cannot write ({e}); old checkpoint kept"),
            };
            self.diagnostics.push(note);
        }
        let active = self.cfg.policy;
        self.restart_trainer(active);
        self.last_snapshot_t = Some(timestamp);
        self.drain_hooks();
        (installed, out)
    }

    /// Reloads every model from storage into a new trainer with `kind`
    /// serving; the previous trainer must already be shut down.
    fn restart_trainer(&mut self, kind: ModelKind) {
        let learners = load_all(self.storage.as_ref(), self.cfg.seed, &mut self.diagnostics);
        self.cfg.policy = kind;
        let started = start_trainer(&self.cfg, kind, &self.storage, learners);
        let Started {
            trainer,
            model,
            loaded,
        } = started;
        self.trainer = trainer;
        self.loaded = loaded;
        self.model_id = std::sync::Arc::from(model.id());
        self.model = model;
    }

    /// Counters of `kind` from the `model_stats` hook summary as of the
    /// last word boundary (all zero for a model the engine does not hold).
    /// `words_total` counts words with a final label; see
    /// docs/04-engine-ml.md.
    #[must_use]
    pub fn model_stats(&self, kind: ModelKind) -> ModelStats {
        let i = kind.index();
        match (self.loaded.get(i), self.stats.get(i)) {
            (Some(true), Some(s)) => *s,
            _ => ModelStats::default(),
        }
    }

    /// Whether online training of `kind` is on.
    #[must_use]
    pub fn online_training(&self, kind: ModelKind) -> bool {
        self.cfg.models.online_training(kind)
    }

    /// Turns online training of `kind` on or off now (a frozen model keeps
    /// its weights and is neither shadow-scored nor saved) and remembers the
    /// choice in the blob `settings/models` (it wins over `config.toml` at
    /// the next start).
    pub fn set_online_training(&mut self, kind: ModelKind, on: bool) {
        self.cfg.models.set_online_training(kind, on);
        self.trainer.set_learn(kind, on);
        let mut saved = read_models_blob(self.storage.as_ref(), &mut self.diagnostics);
        saved.online_training.insert(kind.name().to_owned(), on);
        match serde_json::to_vec(&saved) {
            Ok(bytes) => {
                if let Err(e) = self.storage.blobs().put(MODELS_KEY, &bytes) {
                    self.diagnostics.push(format!(
                        "cannot save {MODELS_KEY} ({e}); choice not remembered"
                    ));
                }
            }
            Err(e) => self
                .diagnostics
                .push(format!("cannot encode {MODELS_KEY}: {e}")),
        }
    }

    /// Shuts `old` down, logging its final updates and noting failures.
    fn finish_trainer(&mut self, old: crate::trainer::Trainer, timestamp: f64) {
        let mut outs = Vec::new();
        let saved = old.shutdown(&mut outs);
        for o in outs {
            // A final snapshot is dropped: the old model is being replaced.
            if let Some(TrainerOut::Diagnostic(d)) = self.log_trainer_out(timestamp, o) {
                self.diagnostics.push(d);
            }
        }
        if let Err(e) = saved {
            self.diagnostics
                .push(format!("saving the previous model failed: {e}"));
        }
    }

    /// Switches to the `kind` model. Commits a word being composed (the
    /// returned output carries it, as in [`Engine::commit_pending`]),
    /// finalizes pending labels, saves the current model to its own
    /// checkpoint, loads `kind`'s checkpoint (a fresh model plus a
    /// diagnostic when it is unreadable or corrupt), persists the
    /// choice and logs `Event::ModelSwitch`. Switching to the active kind
    /// only commits the pending word.
    pub fn switch_model(&mut self, kind: ModelKind, timestamp: f64) -> EngineOutput {
        let mut out = EngineOutput::default();
        self.tick(timestamp);
        self.commit_word(Boundary::External, timestamp, &mut out);
        if kind == self.cfg.policy {
            self.drain_trainer(timestamp);
            return out;
        }

        self.flush_taught();
        let mut finals = Vec::new();
        self.labeler.flush(&mut finals);
        self.emit_labels(timestamp, finals);
        // A correction can no longer reach the old model's pending words.

        // The old trainer saves every model it trained before the reload.
        let old = std::mem::replace(&mut self.trainer, Trainer::new(Vec::new(), false));
        self.finish_trainer(old, timestamp);
        let from = self.model_id.to_string();
        self.restart_trainer(kind);
        self.last_snapshot_t = Some(timestamp);
        self.write_setting(kind);
        self.hooks.log(
            timestamp,
            Event::ModelSwitch {
                from,
                to: self.model_id.to_string(),
            },
        );
        out
    }

    fn write_setting(&mut self, kind: ModelKind) {
        let bytes = match serde_json::to_vec(&ModelSetting { model: kind }) {
            Ok(b) => b,
            Err(e) => {
                self.diagnostics
                    .push(format!("cannot encode {SETTINGS_KEY}: {e}"));
                return;
            }
        };
        if let Err(e) = self.storage.blobs().put(SETTINGS_KEY, &bytes) {
            self.diagnostics.push(format!(
                "cannot save {SETTINGS_KEY} ({e}); choice not remembered"
            ));
        }
    }
}
