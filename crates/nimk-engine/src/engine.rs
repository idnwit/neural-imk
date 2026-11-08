//! Public engine API and the wiring between the internal modules.
//!
//! The per-key flow (decisions, commits, manual switches) lives in the
//! private submodule `flow`; this file builds the engine and implements the
//! public entry points.

mod flow;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use nimk_policy::{FeatureExtractor, Model, Policy, PolicyKind};
use nimk_storage::{EventSink, Storage};
use nimk_types::{AppContext, Event, Features, KeyEvent, LabelSource, Lang, Target};

use crate::composer::Composer;
use crate::hotkey::HotkeyDetector;
use crate::labeler::{Labeler, LabelerParams};
use crate::logger::{Logger, NullSink};
use crate::trainer::{Core, SaveTarget, Trainer, TrainerOut, TrainerParams};
use crate::{EngineConfig, Threading};

/// Why the overlay is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// The policy changed the sticky mode (never shown for word-scoped
    /// hard-rule renderings).
    Auto,
    /// The user pressed the hotkey.
    Manual,
}

/// What the shim must do after an engine call. Mirrors `NimkOut` in the C ABI.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EngineOutput {
    /// `false`: hand the key back to the app untouched.
    pub consumed: bool,
    /// Text to `insertText` (commits and clears the marked text).
    pub commit: Option<String>,
    /// New marked text; `Some("")` clears it. Applied after `commit`.
    pub marked: Option<String>,
    /// Sticky session mode (the mode the next word starts in) after this
    /// call; `None` = unchanged. Word-scoped hard-rule renderings
    /// (`InvalidVi`) do not appear here.
    pub mode: Option<Lang>,
    /// Show the mode overlay at the caret; only when the sticky mode changes.
    pub overlay: Option<Overlay>,
}

/// Engine errors (only from [`Engine::shutdown`]; construction never fails).
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// Storage failed.
    #[error("storage: {0}")]
    Storage(#[from] std::io::Error),
}

/// Identity of one IME process run, supplied by the host (the FFI layer
/// may read the wall clock; the engine may not).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    /// Random session id, embedded in every log record.
    pub id: String,
    /// Wall clock at start, Unix milliseconds.
    pub wall_unix_ms: u64,
    /// IME build version.
    pub app_version: String,
}

/// Blob key of the latest model snapshot for `kind`.
fn snapshot_key(kind: PolicyKind) -> String {
    let name = match kind {
        PolicyKind::Baseline => "baseline",
        PolicyKind::Linear => "linear",
        PolicyKind::TinyMlp => "tiny_mlp",
    };
    format!("model/{name}/latest")
}

/// Telex flags of the word being composed, kept for the commit-time rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct WordTelex {
    /// Some Telex transform took effect.
    has_telex: bool,
    /// The Telex rendering is a complete valid syllable.
    valid_vi: bool,
}

/// Focused app/field as last reported by [`Engine::on_context`].
#[derive(Debug, Clone, PartialEq)]
struct Focus {
    ctx: AppContext,
    id: u64,
}

/// The IME engine. One instance per IME process; not `Sync`, call it from
/// the IMK main thread only.
pub struct Engine {
    cfg: EngineConfig,
    extractor: Arc<dyn FeatureExtractor>,
    policy: Arc<dyn Policy>,
    policy_id: Arc<str>,
    trainer: Trainer,
    logger: Logger,
    labeler: Labeler,
    hotkey: HotkeyDetector,
    composer: Composer,
    /// Sticky session mode: changed only by policy decisions that pass
    /// hysteresis and by manual switches.
    mode: Lang,
    /// Word-scoped rendering override set by a hard rule (`InvalidVi`);
    /// cleared at commit.
    word_mode: Option<Lang>,
    /// A manual switch happened between words: the policy may not move the
    /// sticky mode until the next word commits (the user's choice wins).
    manual_hold: bool,
    focus: Option<Focus>,
    next_focus: u64,
    /// Modes of up to 3 previous words in this field, most recent first.
    prev_langs: Vec<Lang>,
    last_switch_t: Option<f64>,
    /// A manual switch happened while composing the current word.
    in_word_correction: bool,
    /// A word was committed since the last focus change.
    committed_since_focus: bool,
    /// Features of the latest decision of the current word.
    word_features: Features,
    /// Telex flags of the current word as of its latest key.
    word_telex: WordTelex,
    last_snapshot_t: Option<f64>,
    last_t: f64,
    diagnostics: Vec<String>,
}

impl Engine {
    /// Creates an engine. Never fails: a missing or corrupt snapshot falls
    /// back to a fresh `cfg.policy` model, and an unavailable event sink falls
    /// back to a null sink; both are reported via [`Engine::take_diagnostics`].
    /// Writes `SessionStart` as the first record.
    #[must_use]
    pub fn new(cfg: EngineConfig, storage: Arc<dyn Storage>, session: SessionInfo) -> Self {
        let key = snapshot_key(cfg.policy);
        let mut diagnostics = Vec::new();
        let mut model = nimk_policy::build(cfg.policy, cfg.seed);
        match storage.blobs().get(&key) {
            Ok(None) => {}
            Ok(Some(bytes)) => {
                if let Err(e) = model.learner.load(&bytes) {
                    diagnostics.push(format!("snapshot {key} unusable ({e}); starting fresh"));
                    model = nimk_policy::build(cfg.policy, cfg.seed);
                }
            }
            Err(e) => diagnostics.push(format!("cannot read snapshot {key} ({e}); starting fresh")),
        }
        let mut engine = Self::with_model(cfg, storage, session, model);
        diagnostics.append(&mut engine.diagnostics);
        engine.diagnostics = diagnostics;
        engine
    }

    /// Creates an engine around a given model without loading snapshots
    /// (benchmarks, synthetic pre-training).
    #[must_use]
    pub fn with_model(
        cfg: EngineConfig,
        storage: Arc<dyn Storage>,
        session: SessionInfo,
        model: Model,
    ) -> Self {
        let mut diagnostics = Vec::new();
        let background = cfg.threading == Threading::Background;
        let sink: Box<dyn EventSink> = match storage.event_sink(&session.id) {
            Ok(sink) => sink,
            Err(e) => {
                diagnostics.push(format!(
                    "event sink unavailable ({e}); events are not logged"
                ));
                Box::new(NullSink)
            }
        };
        let logger = Logger::new(session.id.clone(), sink, background);

        let policy = model.learner.snapshot();
        let policy_id: Arc<str> = Arc::from(policy.id());
        let params = TrainerParams {
            correction_window: cfg.correction_window,
            correction_steps: cfg.correction_steps,
            replay_capacity: cfg.replay_capacity,
            replay_every: cfg.replay_every,
            replay_batch: cfg.replay_batch,
            seed: cfg.seed,
        };
        let save = SaveTarget {
            storage,
            key: snapshot_key(cfg.policy),
        };
        let trainer = Trainer::new(Core::new(model.learner, params, save), background);
        let labeler = Labeler::new(LabelerParams {
            implicit: cfg.hotkeys,
            correction_words: cfg.correction_words,
            timeout_secs: cfg.label_timeout_secs,
            implicit_weight: cfg.implicit_weight,
        });
        let hotkey = HotkeyDetector::new(cfg.shift_tap_secs, cfg.shift_tap, cfg.ctrl_space);
        let config_json = match serde_json::to_string(&cfg) {
            Ok(json) => json,
            Err(e) => {
                diagnostics.push(format!("cannot serialize config: {e}"));
                String::new()
            }
        };

        let mut engine = Self {
            cfg,
            extractor: model.extractor,
            policy,
            policy_id,
            trainer,
            logger,
            labeler,
            hotkey,
            composer: Composer::new(),
            // Prior: start in VI (the IME targets Vietnamese users); only
            // the hard rules force EN until the model is confident.
            mode: Lang::Vi,
            word_mode: None,
            manual_hold: false,
            focus: None,
            next_focus: 0,
            prev_langs: Vec::with_capacity(3),
            last_switch_t: None,
            in_word_correction: false,
            committed_since_focus: false,
            word_features: Features::default(),
            word_telex: WordTelex::default(),
            last_snapshot_t: None,
            last_t: 0.0,
            diagnostics,
        };
        let start = Event::SessionStart {
            wall_unix_ms: session.wall_unix_ms,
            app_version: session.app_version,
            config_json,
            policy_id: engine.policy_id.to_string(),
        };
        engine.logger.log(0.0, start);
        engine
    }

    /// App focus changed or a new caret position is known. Assigns a new
    /// focus id (and makes a prior decision) when app or field changed.
    ///
    /// A word still being composed when the focus changes is committed
    /// first (boundary `External`), so no typed text is lost.
    pub fn on_context(&mut self, ctx: &AppContext) -> EngineOutput {
        let mut out = EngineOutput::default();
        self.tick(ctx.timestamp);
        self.logger.log(
            ctx.timestamp,
            Event::Context {
                bundle_id: ctx.bundle_id.clone(),
                field_id: ctx.field_id,
                caret: ctx.caret,
            },
        );
        let changed = self
            .focus
            .as_ref()
            .is_none_or(|f| f.ctx.bundle_id != ctx.bundle_id || f.ctx.field_id != ctx.field_id);
        if !changed {
            if let Some(f) = self.focus.as_mut() {
                f.ctx = ctx.clone();
            }
            return out;
        }
        self.commit_word(nimk_types::Boundary::External, ctx.timestamp, &mut out);
        let id = self.next_focus;
        self.next_focus += 1;
        self.focus = Some(Focus {
            ctx: ctx.clone(),
            id,
        });
        self.prev_langs.clear();
        self.committed_since_focus = false;
        self.in_word_correction = false;
        self.manual_hold = false;
        self.decide_prior(id, ctx.timestamp, &mut out);
        self.drain_trainer(ctx.timestamp);
        out
    }

    /// Handles one key. Never panics; on internal error returns
    /// `consumed = false` so the key reaches the app. Word ids are 0, 1, 2...
    /// per session, one per committed non-empty word; they depend only on
    /// boundary keys, never on the mode, so replay and synthetic ids match.
    pub fn on_key(&mut self, key: &KeyEvent) -> EngineOutput {
        match catch_unwind(AssertUnwindSafe(|| self.handle_key(key))) {
            Ok(out) => out,
            Err(_) => {
                self.diagnostics
                    .push("internal panic in on_key; key passed through".into());
                EngineOutput::default()
            }
        }
    }

    /// Commits the marked text as-is (IMK `commitComposition`, focus loss).
    pub fn commit_pending(&mut self, timestamp: f64) -> EngineOutput {
        let mut out = EngineOutput::default();
        self.tick(timestamp);
        self.commit_word(nimk_types::Boundary::External, timestamp, &mut out);
        self.drain_trainer(timestamp);
        out
    }

    /// Feeds an external label (replay ground truth, synthetic data) exactly
    /// as if it had been derived from a hotkey or an implicit timeout.
    ///
    /// The target must still be pending (committed or focused, and not yet
    /// labelled); otherwise the label is dropped with a diagnostic. With
    /// `hotkeys = false` implicit labels are never generated, so every
    /// committed word stays pending until its label is injected.
    pub fn inject_label(&mut self, target: Target, label: Lang, weight: f32, source: LabelSource) {
        match self.labeler.finalize(target, label, weight, source) {
            Some(fl) => {
                let t = self.last_t;
                self.emit_labels(t, vec![fl]);
                self.drain_trainer(t);
            }
            None => self
                .diagnostics
                .push(format!("inject_label: {target:?} is not pending; dropped")),
        }
    }

    /// Sticky session mode: the mode the next word starts in. A word
    /// rendered in EN by a hard rule does not change it.
    #[must_use]
    pub fn mode(&self) -> Lang {
        self.mode
    }

    /// Id of the policy currently serving predictions.
    #[must_use]
    pub fn policy_id(&self) -> Arc<str> {
        Arc::clone(&self.policy_id)
    }

    /// Drains human-readable warnings (fallbacks, dropped log records) for
    /// the host's debug log.
    pub fn take_diagnostics(&mut self) -> Vec<String> {
        let mut d = std::mem::take(&mut self.diagnostics);
        d.extend(self.logger.take_diagnostics());
        d
    }

    /// Finalizes pending labels, flushes the log and saves a snapshot.
    ///
    /// # Errors
    /// Fails if flushing or saving fails.
    pub fn shutdown(mut self) -> Result<(), EngineError> {
        let t = self.last_t;
        let mut finals = Vec::new();
        self.labeler.flush(&mut finals);
        self.emit_labels(t, finals);
        let mut outs = Vec::new();
        let saved = self.trainer.shutdown(&mut outs);
        for o in outs {
            if let TrainerOut::Update {
                policy_id,
                n_samples,
                loss,
            } = o
            {
                self.logger.log(
                    t,
                    Event::ModelUpdate {
                        policy_id,
                        n_samples,
                        loss,
                    },
                );
            }
        }
        let flushed = self.logger.shutdown();
        saved?;
        flushed?;
        Ok(())
    }
}
