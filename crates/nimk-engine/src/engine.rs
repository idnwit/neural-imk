//! Public engine API and the wiring between the internal modules.
//!
//! The engine is mechanical (ADR 0007): per key it builds an
//! [`Observation`], asks the action model and applies the chosen edit. The
//! per-key flow lives in the private submodule `flow`, the hotkey and
//! retroactive correction (ADR 0008) in `switch`; this file builds the
//! engine and implements the public entry points.

mod flow;
mod switch;

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use nimk_policy::{ImeLearner, ImeModel, ModelKind, build_action};
use nimk_storage::{EventSink, Storage};
use nimk_types::{Action, AppContext, Event, Example, KeyEvent, LabelSource, Lang, Target};

use crate::composer::Composer;
use crate::hotkey::{HotkeyDetector, HotkeyParams};
use crate::labeler::{Labeler, LabelerParams, Step};
use crate::logger::{Logger, NullSink};
use crate::trainer::{Core, SaveTarget, Trainer, TrainerOut, TrainerParams};
use crate::{EngineConfig, Threading};

/// Why the overlay is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// The model changed the sticky mode.
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
    /// call; `None` = unchanged.
    pub mode: Option<Lang>,
    /// Show the mode overlay at the caret; only when the sticky mode changes.
    pub overlay: Option<Overlay>,
    /// Retroactive correction of the just-committed word (ADR 0008).
    /// Applied before `commit` and `marked`.
    pub replace: Option<Replace>,
}

/// Replace text already inserted before the caret (ADR 0008).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replace {
    /// Number of UTF-16 code units to delete immediately before the caret
    /// (the committed word plus the boundary characters typed after it).
    pub delete_before: u32,
    /// Text to insert in their place (new rendering + the same boundary).
    pub text: String,
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
fn snapshot_key(kind: ModelKind) -> String {
    let name = match kind {
        ModelKind::LinearAction => "linear_action",
        ModelKind::GruAction => "gru_action",
    };
    format!("model/{name}/latest")
}

/// Telex flags of the current raw buffer (features, not rules).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TelexFlags {
    valid_vi: bool,
    viable_vi: bool,
}

/// Focused app/field as last reported by [`Engine::on_context`].
#[derive(Debug, Clone, PartialEq)]
struct Focus {
    ctx: AppContext,
    id: u64,
}

/// The last committed word, kept while only boundary characters were typed
/// after it so a double Shift can still rewrite it (ADR 0008).
#[derive(Debug, Clone, PartialEq, Eq)]
struct LastWord {
    id: u64,
    raw: String,
    vi: String,
    /// Rendering that was committed.
    lang: Lang,
    /// Characters typed (and passed to the app) after the word.
    boundary: String,
}

impl LastWord {
    fn text(&self, lang: Lang) -> &str {
        match lang {
            Lang::Vi => &self.vi,
            Lang::En => &self.raw,
        }
    }
}

/// The IME engine. One instance per IME process; not `Sync`, call it from
/// the IMK main thread only.
pub struct Engine {
    cfg: EngineConfig,
    model: Arc<dyn ImeModel>,
    model_id: Arc<str>,
    trainer: Trainer,
    logger: Logger,
    labeler: Labeler,
    hotkey: HotkeyDetector,
    composer: Composer,
    /// Sticky session mode: the rendering the next word starts in. Changed
    /// by the model at commits and focus changes, and by manual switches.
    mode: Lang,
    /// Rendering currently shown for the buffer (`mode` when it is empty).
    shown: Lang,
    /// A manual switch happened while composing the current word: its
    /// rendering is locked until the word ends.
    locked: bool,
    /// Telex flags of the buffer as of its latest edit.
    telex: TelexFlags,
    /// Model calls of the current word.
    word_steps: Vec<Step>,
    /// Teacher-forced examples not yet handed to the trainer
    /// (`EngineConfig::teach_batch`).
    taught: Vec<Example>,
    /// Previous non-hotkey key-downs, oldest first.
    recent: VecDeque<KeyEvent>,
    last_word: Option<LastWord>,
    focus: Option<Focus>,
    next_focus: u64,
    /// Renderings of up to 3 previous words in this field, most recent first.
    prev_langs: Vec<Lang>,
    last_switch_t: Option<f64>,
    /// A word was committed since the last focus change.
    committed_since_focus: bool,
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
        let mut learner = build_action(cfg.policy, cfg.seed);
        match storage.blobs().get(&key) {
            Ok(None) => {}
            Ok(Some(bytes)) => {
                if let Err(e) = learner.load(&bytes) {
                    diagnostics.push(format!("snapshot {key} unusable ({e}); starting fresh"));
                    learner = build_action(cfg.policy, cfg.seed);
                }
            }
            Err(e) => diagnostics.push(format!("cannot read snapshot {key} ({e}); starting fresh")),
        }
        let mut engine = Self::with_model(cfg, storage, session, learner);
        diagnostics.append(&mut engine.diagnostics);
        engine.diagnostics = diagnostics;
        engine
    }

    /// Creates an engine around a given learner without loading snapshots
    /// (benchmarks, synthetic pre-training, tests).
    #[must_use]
    pub fn with_model(
        cfg: EngineConfig,
        storage: Arc<dyn Storage>,
        session: SessionInfo,
        learner: Box<dyn ImeLearner>,
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

        let model = learner.snapshot();
        let model_id: Arc<str> = Arc::from(model.id());
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
        let trainer = Trainer::new(Core::new(learner, params, save), background);
        let labeler = Labeler::new(LabelerParams {
            implicit: cfg.hotkeys,
            correction_words: cfg.correction_words,
            timeout_secs: cfg.label_timeout_secs,
            implicit_weight: cfg.implicit_weight,
            edit_weight: cfg.edit_online_weight,
        });
        let hotkey = HotkeyDetector::new(HotkeyParams {
            tap_secs: cfg.shift_tap_secs,
            double_secs: cfg.double_shift_secs,
            double_shift: cfg.double_shift,
            ctrl_space: cfg.ctrl_space,
        });
        let config_json = match serde_json::to_string(&cfg) {
            Ok(json) => json,
            Err(e) => {
                diagnostics.push(format!("cannot serialize config: {e}"));
                String::new()
            }
        };

        let mut engine = Self {
            cfg,
            model,
            model_id,
            trainer,
            logger,
            labeler,
            hotkey,
            composer: Composer::new(),
            // Initial prior; the model overrides it at the first focus.
            mode: Lang::Vi,
            shown: Lang::Vi,
            locked: false,
            telex: TelexFlags::default(),
            word_steps: Vec::new(),
            taught: Vec::new(),
            recent: VecDeque::with_capacity(nimk_types::RECENT_WINDOW + 1),
            last_word: None,
            focus: None,
            next_focus: 0,
            prev_langs: Vec::with_capacity(3),
            last_switch_t: None,
            committed_since_focus: false,
            last_snapshot_t: None,
            last_t: 0.0,
            diagnostics,
        };
        let start = Event::SessionStart {
            wall_unix_ms: session.wall_unix_ms,
            app_version: session.app_version,
            config_json,
            policy_id: engine.model_id.to_string(),
        };
        engine.logger.log(0.0, start);
        engine
    }

    /// App focus changed or a new caret position is known. Assigns a new
    /// focus id (and asks the model for a mode prior) when app or field
    /// changed.
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
        self.recent.clear();
        self.last_word = None;
        self.committed_since_focus = false;
        self.locked = false;
        self.decide_prior(id, ctx.timestamp, &mut out);
        self.drain_trainer(ctx.timestamp);
        out
    }

    /// Handles one key. Never panics; on internal error returns
    /// `consumed = false` so the key reaches the app. Word ids are 0, 1, 2...
    /// per session, one per committed non-empty word; they depend only on
    /// the buffer's commits, never on the rendering, so replay and synthetic
    /// ids match.
    pub fn on_key(&mut self, key: &KeyEvent) -> EngineOutput {
        match catch_unwind(AssertUnwindSafe(|| self.handle_key(key, None))) {
            Ok(out) => out,
            Err(_) => {
                self.diagnostics
                    .push("internal panic in on_key; key passed through".into());
                EngineOutput::default()
            }
        }
    }

    /// Teacher forcing for pre-training: like [`Engine::on_key`] but applies
    /// `target` instead of the model's decision and queues one [`Example`]
    /// (observation, effective target, the given weights) to the trainer at
    /// once (or, with `EngineConfig::teach_batch` > 1, in batches of that
    /// many; the remainder is trained at [`Engine::shutdown`]). The model is
    /// not consulted, so the logged `Decision` has
    /// `p_vi: None`. An edit the action mask forbids falls back to `Pass`
    /// (and the example records `Pass`). Hotkeys are still detected.
    ///
    /// Words typed this way keep no per-step examples for later labels
    /// (they were already taught); labels still flow through the labeler,
    /// so [`Engine::inject_label`] works as usual.
    ///
    /// [`Example`]: nimk_types::Example
    pub fn teach(
        &mut self,
        key: &KeyEvent,
        target: Action,
        edit_weight: f32,
        render_weight: f32,
    ) -> EngineOutput {
        let forced = flow::Forced {
            target,
            edit_weight,
            render_weight,
        };
        match catch_unwind(AssertUnwindSafe(|| self.handle_key(key, Some(forced)))) {
            Ok(out) => out,
            Err(_) => {
                self.diagnostics
                    .push("internal panic in teach; key passed through".into());
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

    /// Sticky session mode: the mode the next word starts in.
    #[must_use]
    pub fn mode(&self) -> Lang {
        self.mode
    }

    /// Id of the model currently serving predictions.
    #[must_use]
    pub fn policy_id(&self) -> Arc<str> {
        Arc::clone(&self.model_id)
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
        self.flush_taught();
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
