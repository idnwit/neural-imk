//! Runtime model selection: per-model checkpoints and the persisted choice.
//!
//! Each [`ModelKind`] keeps its own snapshot at `model/<kind>/latest`; the
//! chosen kind is remembered in the blob [`SETTINGS_KEY`] and read back by
//! [`Engine::new`].

use nimk_policy::ModelKind;
use nimk_storage::{BlobOrigin, MemStorage, Storage};
use nimk_types::{Boundary, Event};

use super::{
    Engine, EngineOutput, Threading, TrainerOut, build_action, load_learner, new_trainer,
    snapshot_key,
};

/// Blob holding the selected model, `{ "model": "linear_action" }`.
const SETTINGS_KEY: &str = "settings/model";

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
        let scratch_cfg = super::EngineConfig {
            threading: Threading::Inline,
            ..self.cfg.clone()
        };
        let scratch = new_trainer(
            &scratch_cfg,
            kind,
            std::sync::Arc::new(MemStorage::new()),
            build_action(kind, self.cfg.seed),
        );
        let old = std::mem::replace(&mut self.trainer, scratch);
        self.finish_trainer(old, timestamp);

        let key = snapshot_key(kind);
        let note = match self.storage.blobs().delete(&key) {
            Ok(true) => format!("reset {key}: user checkpoint deleted"),
            Ok(false) => format!("reset {key}: no user checkpoint"),
            Err(e) => format!("reset {key}: cannot delete user checkpoint ({e})"),
        };
        self.diagnostics.push(note);

        let learner = load_learner(
            self.storage.as_ref(),
            kind,
            self.cfg.seed,
            &mut self.diagnostics,
        );
        let model = learner.snapshot();
        let trainer = new_trainer(&self.cfg, kind, self.storage.clone(), learner);
        let scratch = std::mem::replace(&mut self.trainer, trainer);
        // Nothing of the scratch trainer is worth keeping.
        self.finish_trainer(scratch, timestamp);
        self.model_id = std::sync::Arc::from(model.id());
        self.model = model;
        self.last_snapshot_t = Some(timestamp);
        out
    }

    /// Shuts `old` down, logging its final updates and noting failures.
    fn finish_trainer(&mut self, old: crate::trainer::Trainer, timestamp: f64) {
        let mut outs = Vec::new();
        let saved = old.shutdown(&mut outs);
        for o in outs {
            match o {
                TrainerOut::Update {
                    policy_id,
                    n_samples,
                    loss,
                } => self.logger.log(
                    timestamp,
                    Event::ModelUpdate {
                        policy_id,
                        n_samples,
                        loss,
                    },
                ),
                TrainerOut::Diagnostic(d) => self.diagnostics.push(d),
                // The old model is being replaced.
                TrainerOut::Snapshot(_) => {}
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

        let learner = load_learner(
            self.storage.as_ref(),
            kind,
            self.cfg.seed,
            &mut self.diagnostics,
        );
        let model = learner.snapshot();
        let trainer = new_trainer(&self.cfg, kind, self.storage.clone(), learner);
        let old = std::mem::replace(&mut self.trainer, trainer);
        self.finish_trainer(old, timestamp);

        let from = self.model_id.to_string();
        self.model_id = std::sync::Arc::from(model.id());
        self.model = model;
        self.cfg.policy = kind;
        self.last_snapshot_t = Some(timestamp);
        self.write_setting(kind);
        self.logger.log(
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
