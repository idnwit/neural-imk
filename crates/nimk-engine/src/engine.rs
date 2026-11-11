//! Public engine API and the wiring between the internal modules.
//!
//! The engine is mechanical (ADR 0007): per key it builds an
//! [`Observation`], asks the action model and applies the chosen edit. The
//! per-key flow lives in the private submodule `flow`, the hotkey in
//! `switch`; this file builds the engine and implements the public entry
//! points. A word the model ends stays "pending" (marked text) until the
//! user starts the next word (ADR 0011).

mod flow;
mod machine;
mod model;
mod retro;
mod switch;

pub use model::CheckpointSource;
pub use retro::{RetroFlip, RetroPolicy};

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use nimk_hook::{EventLogHook, Hook, ModelStatsHook, UsageHook};
use nimk_policy::{ImeLearner, ImeModel, ModelKind, build_action};
use nimk_storage::{EventSink, Storage};
use nimk_types::{
    Action, AppContext, Event, Example, KeyEvent, LabelSource, Lang, ModelStats, StatCategory,
    Target,
};

use crate::composer::Composer;
use crate::config::LiveParams;
use crate::hookbus::{HookBus, NullSink};
use crate::hotkey::{HotkeyDetector, HotkeyParams};
use crate::labeler::{Labeler, LabelerParams, Step};
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
    /// Text to `insertText` (commits and clears the marked text). The
    /// pending word plus its boundary characters, or the word being composed.
    pub commit: Option<String>,
    /// New marked text; `Some("")` clears it. Applied after `commit`.
    pub marked: Option<String>,
    /// Sticky session mode (the mode the next word starts in) after this
    /// call; `None` = unchanged.
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
    /// Local time offset from UTC at start, minutes.
    pub utc_offset_min: i32,
}

/// Blob key of the latest model snapshot for `kind`.
fn snapshot_key(kind: ModelKind) -> String {
    format!("model/{}/latest", kind.name())
}

/// Builds the learner for `kind`, loading `model/<kind>/latest` when it
/// exists. A missing blob gives a fresh model silently; an unreadable or
/// corrupt one gives a fresh model and a note in `diagnostics`.
fn load_learner(
    storage: &dyn Storage,
    kind: ModelKind,
    seed: u64,
    diagnostics: &mut Vec<String>,
) -> Box<dyn ImeLearner> {
    let key = snapshot_key(kind);
    let mut learner = build_action(kind, seed);
    match storage.blobs().get(&key) {
        Ok(None) => {}
        Ok(Some(bytes)) => {
            if let Err(e) = learner.load(&bytes) {
                diagnostics.push(format!("snapshot {key} unusable ({e}); starting fresh"));
                learner = build_action(kind, seed);
            }
        }
        Err(e) => diagnostics.push(format!("cannot read snapshot {key} ({e}); starting fresh")),
    }
    learner
}

/// Loads every model kind (the trainer holds them all, ADR 0013).
fn load_all(
    storage: &dyn Storage,
    seed: u64,
    diagnostics: &mut Vec<String>,
) -> Vec<(ModelKind, Box<dyn ImeLearner>)> {
    ModelKind::ALL
        .iter()
        .map(|k| (*k, load_learner(storage, *k, seed, diagnostics)))
        .collect()
}

/// A started trainer and what the engine caches from it.
struct Started {
    trainer: Trainer,
    /// Snapshot of the active model.
    model: Arc<dyn ImeModel>,
    loaded: [bool; 2],
}

/// Starts a trainer around `learners`, `active` serving predictions. Each
/// learner saves to the key of its kind and trains online per
/// `cfg.models`. `active` must be among `learners`.
fn start_trainer(
    cfg: &EngineConfig,
    active: ModelKind,
    storage: &Arc<dyn Storage>,
    learners: Vec<(ModelKind, Box<dyn ImeLearner>)>,
) -> Started {
    let params = TrainerParams {
        correction_window: cfg.correction_window,
        correction_steps: cfg.correction_steps,
        replay_capacity: cfg.replay_capacity,
        replay_every: cfg.replay_every,
        replay_batch: cfg.replay_batch,
        seed: cfg.seed,
    };
    let mut model = None;
    let mut loaded = [false; 2];
    let mut cores = Vec::with_capacity(learners.len());
    for (kind, learner) in learners {
        if kind == active {
            model = Some(learner.snapshot());
        }
        let save = SaveTarget {
            storage: Arc::clone(storage),
            key: snapshot_key(kind),
        };
        let core = Core::new(kind, learner, params, save)
            .with_flags(cfg.models.online_training(kind), kind == active);
        loaded[kind.index()] = true;
        cores.push(core);
    }
    let model = match model {
        Some(m) => m,
        None => build_action(active, cfg.seed).snapshot(),
    };
    Started {
        trainer: Trainer::new(cores, cfg.threading == Threading::Background),
        model,
        loaded,
    }
}

/// A word being committed, remembered until its label arrives.
#[derive(Debug, Clone, Copy)]
struct WordMeta {
    id: u64,
    category: StatCategory,
}

/// Most committed words whose category is remembered for their labels.
const META_KEPT: usize = 512;

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

/// A word the model ended, still shown as marked text (ADR 0011). Its
/// `WordCommit` record and label are written when it is committed for real,
/// so a flip or a Backspace into the word can still change it.
#[derive(Debug, Clone)]
struct PendingWord {
    id: u64,
    raw: String,
    vi: String,
    /// Rendering shown (and committed).
    lang: Lang,
    /// The user flipped the rendering: the word is labelled a correction.
    locked: bool,
    /// What ended the word, for the `WordCommit` record.
    ended_by: nimk_types::Boundary,
    /// Boundary characters typed after the word (still marked text).
    boundary: String,
    /// Model calls of the word, for its label.
    steps: Vec<Step>,
}

impl PendingWord {
    fn word(&self) -> &str {
        match self.lang {
            Lang::Vi => &self.vi,
            Lang::En => &self.raw,
        }
    }

    /// The marked text: rendering plus boundary.
    fn text(&self) -> String {
        format!("{}{}", self.word(), self.boundary)
    }
}

/// The IME engine. One instance per IME process; not `Sync`, call it from
/// the IMK main thread only.
pub struct Engine {
    cfg: EngineConfig,
    storage: Arc<dyn Storage>,
    model: Arc<dyn ImeModel>,
    model_id: Arc<str>,
    trainer: Trainer,
    /// Counters per model kind (`ModelKind::index`), parsed from the
    /// `model_stats` hook summary when hook messages are drained.
    stats: [ModelStats; 2],
    /// Which kinds the trainer holds.
    loaded: [bool; 2],
    /// Categories of committed words awaiting their label.
    word_meta: VecDeque<WordMeta>,
    hooks: HookBus,
    /// Latest summary JSON per hook, updated when hook messages are drained.
    summaries: HashMap<&'static str, String>,
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
    /// The word the model ended and the user has not moved past yet.
    pending: Option<PendingWord>,
    /// Words committed for real, oldest first, for caret flips (ADR 0015).
    history: VecDeque<retro::HistWord>,
    /// The last hotkey press only switched the sticky mode and nothing has
    /// happened since: a caret flip may still undo it.
    undo_switch: Option<retro::UndoSwitch>,
    /// Parameters in force for the focused app (category overrides applied).
    live: LiveParams,
    /// Raw mode: a command is being typed and keys pass straight to the app
    /// (ADR 0012).
    raw: bool,
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
        let mut cfg = cfg;
        let mut diagnostics = Vec::new();
        if let Some(kind) = model::read_setting(storage.as_ref(), &mut diagnostics) {
            cfg.policy = kind;
        }
        model::read_models_setting(storage.as_ref(), &mut cfg.models, &mut diagnostics);
        let learners = load_all(storage.as_ref(), cfg.seed, &mut diagnostics);
        let mut engine = Self::with_models(cfg, storage, session, learners);
        diagnostics.append(&mut engine.diagnostics);
        engine.diagnostics = diagnostics;
        engine
    }

    /// Creates an engine around a given learner without loading snapshots
    /// (benchmarks, synthetic pre-training, tests). It is the only model;
    /// there is no shadow.
    #[must_use]
    pub fn with_model(
        cfg: EngineConfig,
        storage: Arc<dyn Storage>,
        session: SessionInfo,
        learner: Box<dyn ImeLearner>,
    ) -> Self {
        let kind = cfg.policy;
        Self::with_models(cfg, storage, session, vec![(kind, learner)])
    }

    /// Creates an engine around the given learners (one per kind); the one
    /// of `cfg.policy` serves, the others are shadows when their online
    /// training is on. A missing `cfg.policy` learner is built fresh.
    #[must_use]
    pub fn with_models(
        cfg: EngineConfig,
        storage: Arc<dyn Storage>,
        session: SessionInfo,
        mut learners: Vec<(ModelKind, Box<dyn ImeLearner>)>,
    ) -> Self {
        let mut diagnostics = Vec::new();
        if !learners.iter().any(|(k, _)| *k == cfg.policy) {
            learners.push((cfg.policy, build_action(cfg.policy, cfg.seed)));
        }
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
        let hooks: Vec<Box<dyn Hook>> = vec![
            Box::new(EventLogHook::new(sink)),
            Box::new(ModelStatsHook::new()),
            Box::new(UsageHook::new()),
        ];
        let hooks = HookBus::new(session.id.clone(), Arc::clone(&storage), hooks, background);

        let Started {
            trainer,
            model,
            loaded,
        } = start_trainer(&cfg, cfg.policy, &storage, learners);
        let model_id: Arc<str> = Arc::from(model.id());
        let labeler = Labeler::new(LabelerParams {
            implicit: cfg.hotkeys,
            correction_words: cfg.correction_words,
            timeout_secs: cfg.label_timeout_secs,
            implicit_weight: cfg.implicit_weight,
            edit_weight: cfg.edit_online_weight,
        });
        let live = cfg.live_for("");
        let hotkey = HotkeyDetector::new(HotkeyParams {
            tap_secs: live.shift_tap_secs,
            double_secs: live.double_shift_secs,
            double_shift: live.double_shift,
            ctrl_space: live.ctrl_space,
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
            storage,
            model,
            model_id,
            trainer,
            stats: [ModelStats::default(); 2],
            loaded,
            word_meta: VecDeque::with_capacity(16),
            hooks,
            summaries: HashMap::new(),
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
            pending: None,
            history: VecDeque::with_capacity(retro::HISTORY_KEPT),
            undo_switch: None,
            live,
            raw: false,
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
            utc_offset_min: session.utc_offset_min,
        };
        engine.hooks.log(0.0, start);
        engine.drain_hooks();
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
        self.hooks.log(
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
        self.apply_live(&ctx.bundle_id);
        self.raw = false;
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

    /// Commits the marked text as-is: the word being composed, or the pending
    /// word with its boundary (IMK `commitComposition`, focus loss).
    pub fn commit_pending(&mut self, timestamp: f64) -> EngineOutput {
        let mut out = EngineOutput::default();
        self.tick(timestamp);
        self.commit_word(nimk_types::Boundary::External, timestamp, &mut out);
        self.drain_trainer(timestamp);
        out
    }

    /// Seconds of silence after which the host should call
    /// [`Engine::commit_pending`] while text is marked (0 = never). Follows
    /// the focused app's category.
    #[must_use]
    pub fn idle_commit_secs(&self) -> f64 {
        self.live.idle_commit_secs
    }

    /// The user moved the caret without a key (mouse click): raw mode ends.
    pub fn note_navigation(&mut self) {
        self.raw = false;
    }

    /// Replaces the live parameters (everything but the model, the seed and
    /// the trainer settings, which apply at the next start) with those of
    /// `cfg`, for the focused app.
    pub fn reconfigure(&mut self, cfg: EngineConfig) {
        let keep = (
            self.cfg.policy,
            self.cfg.seed,
            self.cfg.threading,
            self.cfg.models,
        );
        self.cfg = EngineConfig {
            policy: keep.0,
            seed: keep.1,
            threading: keep.2,
            models: keep.3,
            ..cfg
        };
        let bundle = match &self.focus {
            Some(f) => f.ctx.bundle_id.clone(),
            None => String::new(),
        };
        self.apply_live(&bundle);
    }

    /// Resolves the category parameters for `bundle_id`; rebuilds the hotkey
    /// detector when its parameters changed.
    fn apply_live(&mut self, bundle_id: &str) {
        let live = self.cfg.live_for(bundle_id);
        if (
            live.shift_tap_secs,
            live.double_shift_secs,
            live.double_shift,
            live.ctrl_space,
        ) != (
            self.live.shift_tap_secs,
            self.live.double_shift_secs,
            self.live.double_shift,
            self.live.ctrl_space,
        ) {
            self.hotkey = HotkeyDetector::new(HotkeyParams {
                tap_secs: live.shift_tap_secs,
                double_secs: live.double_shift_secs,
                double_shift: live.double_shift,
                ctrl_space: live.ctrl_space,
            });
        }
        self.live = live;
    }

    /// Feeds an external label (replay ground truth, synthetic data) exactly
    /// as if it had been derived from a hotkey or an implicit timeout.
    ///
    /// The target must still be pending (committed or focused, and not yet
    /// labelled; a word still shown as marked text is committed first); otherwise the label is dropped with a diagnostic. With
    /// `hotkeys = false` implicit labels are never generated, so every
    /// committed word stays pending until its label is injected.
    pub fn inject_label(&mut self, target: Target, label: Lang, weight: f32, source: LabelSource) {
        // A label can only follow its word's `WordCommit` record, so a word
        // still pending here was committed in the source session: commit it
        // now (offline use; its text is not needed).
        if self
            .pending
            .as_ref()
            .is_some_and(|p| Target::Word(p.id) == target)
        {
            let t = self.last_t;
            self.finalize_pending(t, &mut EngineOutput::default());
        }
        // `refinalize`: a caret flip (ADR 0015) corrects a word whose
        // implicit label was already logged.
        match self.labeler.refinalize(target, label, weight, source) {
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

    /// Latest summary JSON of hook `name`, as of the last word boundary;
    /// `None` if that hook has not published one.
    #[must_use]
    pub fn hook_summary(&self, name: &str) -> Option<&str> {
        self.summaries.get(name).map(String::as_str)
    }

    /// Records that the host re-read the wall clock (`wall_unix_ms`, Unix
    /// milliseconds; `utc_offset_min`, local offset from UTC), e.g. after
    /// the Mac woke from sleep. Logs an [`Event::Clock`] at the last event
    /// time so wall-clock consumers (the `usage` hook) re-anchor. The
    /// engine itself never reads the clock.
    pub fn note_wall_clock(&mut self, wall_unix_ms: u64, utc_offset_min: i32) {
        let t = self.last_t;
        self.hooks.log(
            t,
            Event::Clock {
                wall_unix_ms,
                utc_offset_min,
            },
        );
        self.drain_hooks();
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
        d.extend(self.hooks.take_diagnostics());
        d
    }

    /// Finalizes pending labels, flushes the log and saves a snapshot.
    ///
    /// # Errors
    /// Fails if flushing or saving fails.
    pub fn shutdown(mut self) -> Result<(), EngineError> {
        let t = self.last_t;
        self.settle_switch(t);
        // The host already committed any marked text; this only settles the
        // pending word's label.
        self.commit_word(
            nimk_types::Boundary::External,
            t,
            &mut EngineOutput::default(),
        );
        self.flush_taught();
        let mut finals = Vec::new();
        self.labeler.flush(&mut finals);
        self.emit_labels(t, finals);
        let mut outs = Vec::new();
        let trainer = std::mem::replace(&mut self.trainer, Trainer::new(Vec::new(), false));
        let saved = trainer.shutdown(&mut outs);
        for o in outs {
            // Updates and scores are logged (the hooks still count them);
            // a last snapshot or warning has no one left to reach.
            drop(self.log_trainer_out(t, o));
        }
        let flushed = self.hooks.shutdown();
        saved?;
        flushed?;
        Ok(())
    }
}
