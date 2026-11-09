//! Trainer: online updates, correction re-training, replay buffer, model
//! snapshots and per-model word statistics.
//!
//! The trainer owns one [`Core`] per loaded model (ADR 0013, shadow
//! training): every finalized word goes to each core whose online training
//! is on, each core scores the word for its statistics (`stats`), and only
//! the active core publishes snapshots to the hot path. The same logic runs
//! either on a background thread (production) or synchronously on the
//! caller's thread (`Inline`, deterministic). The engine submits samples and
//! drains [`TrainerOut`] messages at word boundaries; new models arrive as
//! `Arc<dyn ImeModel>` snapshots, so the hot path never takes a lock.

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use nimk_policy::{ImeLearner, ImeModel, ModelKind};
use nimk_storage::Storage;
use nimk_types::{Example, Lang, Observation};

mod stats;

pub use stats::{ModelStats, StatCategory};
pub(crate) use stats::{StatsTracker, WordInfo};
use stats::{WordAcc, WordResult};

/// Publish a snapshot after this many learner updates (and after every
/// correction).
pub(crate) const PUBLISH_EVERY: usize = 20;

/// Trainer tuning (from `EngineConfig`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TrainerParams {
    /// Most recent samples re-trained after a correction.
    pub(crate) correction_window: usize,
    /// Passes over the correction window.
    pub(crate) correction_steps: usize,
    /// Replay buffer capacity.
    pub(crate) replay_capacity: usize,
    /// Train one random minibatch every N samples (0 = never).
    pub(crate) replay_every: usize,
    /// Minibatch size.
    pub(crate) replay_batch: usize,
    /// Minibatch RNG seed.
    pub(crate) seed: u64,
}

/// Something the trainer reports back to the engine.
#[derive(Clone)]
pub(crate) enum TrainerOut {
    /// A learner update happened.
    Update {
        /// Learner id after the update.
        policy_id: String,
        /// Examples in the update.
        n_samples: u32,
        /// Mean weighted loss before the update.
        loss: f32,
    },
    /// New read-only model to serve predictions (active model only).
    Snapshot(Arc<dyn ImeModel>),
    /// Counters of a model changed.
    Stats(ModelKind, ModelStats),
    /// Human-readable warning.
    Diagnostic(String),
}

/// One real keystroke's model call, for the per-keystroke statistics.
#[derive(Debug, Clone)]
pub(crate) struct KeyObs {
    /// Word the keystroke belongs to.
    pub(crate) word_id: u64,
    /// Display decision the active model really made (its hot-path score
    /// is reused, never recomputed).
    pub(crate) active_render: Lang,
    /// Hysteresis margin in force.
    pub(crate) margin: f32,
    /// The observation, for shadow models to score; `None` when none is
    /// enabled.
    pub(crate) obs: Option<Box<Observation>>,
}

/// Word accumulators kept per core (words never labelled age out).
const ACCS_KEPT: usize = 512;

/// Where snapshots are saved.
#[derive(Clone)]
pub(crate) struct SaveTarget {
    /// Storage holding the blob.
    pub(crate) storage: Arc<dyn Storage>,
    /// Blob key, `model/<kind>/latest`.
    pub(crate) key: String,
}

/// Training state; owned by exactly one thread.
pub(crate) struct Core {
    kind: ModelKind,
    learner: Box<dyn ImeLearner>,
    /// Online training on: the learner updates (and saves) from labels.
    learn: bool,
    /// This core serves predictions: its snapshots are published and its
    /// stats use the rendering that was really committed.
    active: bool,
    /// The learner changed since it was loaded or last saved.
    dirty: bool,
    /// Read-only copy of the current weights, for scoring shadow words;
    /// dropped when the learner updates.
    scorer: Option<Arc<dyn ImeModel>>,
    stats: StatsTracker,
    /// Keystroke decisions of words not yet labelled.
    accs: VecDeque<(u64, WordAcc)>,
    params: TrainerParams,
    save: SaveTarget,
    replay: VecDeque<Example>,
    rng: u64,
    since_replay: usize,
    since_publish: usize,
    batch: Vec<Example>,
}

impl Core {
    /// Wraps a learner.
    #[must_use]
    pub(crate) fn new(
        kind: ModelKind,
        learner: Box<dyn ImeLearner>,
        params: TrainerParams,
        save: SaveTarget,
    ) -> Self {
        let cap = params.replay_capacity.min(1024);
        Self {
            kind,
            learner,
            learn: true,
            active: true,
            dirty: false,
            scorer: None,
            stats: StatsTracker::default(),
            accs: VecDeque::new(),
            params,
            save,
            replay: VecDeque::with_capacity(cap),
            rng: params.seed ^ 0x9E37_79B9_7F4A_7C15,
            since_replay: 0,
            since_publish: 0,
            batch: Vec::with_capacity(params.replay_batch.max(params.correction_window).min(1024)),
        }
    }

    /// Sets whether the learner trains online and whether it is the active
    /// (serving) core.
    #[must_use]
    pub(crate) fn with_flags(mut self, learn: bool, active: bool) -> Self {
        self.learn = learn;
        self.active = active;
        self
    }

    /// Starts from restored statistics.
    #[must_use]
    pub(crate) fn with_stats(mut self, stats: StatsTracker) -> Self {
        self.stats = stats;
        self
    }

    /// Counters now (for the engine's initial cache).
    pub(crate) fn stats_summary(&self) -> ModelStats {
        self.stats.summary()
    }

    /// splitmix64 step.
    fn next_u64(&mut self) -> u64 {
        self.rng = self.rng.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Runs one learner update over `self.batch` (then clears it) and
    /// reports it.
    fn update_batch(&mut self, out: &mut Vec<TrainerOut>) {
        if self.batch.is_empty() {
            return;
        }
        let loss = self.learner.update(&self.batch);
        self.touched();
        let n_samples = count(self.batch.len());
        self.batch.clear();
        self.since_publish += 1;
        out.push(TrainerOut::Update {
            policy_id: self.learner.id().to_owned(),
            n_samples,
            loss,
        });
    }

    fn touched(&mut self) {
        self.dirty = true;
        self.scorer = None;
    }

    /// Whether this core is measured: the active one always, a shadow only
    /// with online training on.
    fn counts(&self) -> bool {
        self.active || self.learn
    }

    /// Records the display decision this model makes at one keystroke. The
    /// active model's is given; a shadow scores the observation with its
    /// latest weights and applies the hysteresis to its own display state
    /// (at a commit step the fresh decision, as the engine does).
    pub(crate) fn on_key(&mut self, key: &KeyObs) {
        if !self.counts() {
            return;
        }
        let idx = match self.accs.iter().rposition(|(id, _)| *id == key.word_id) {
            Some(i) => i,
            None => {
                if self.accs.len() >= ACCS_KEPT {
                    self.accs.pop_front();
                }
                self.accs.push_back((key.word_id, WordAcc::default()));
                self.accs.len() - 1
            }
        };
        let display = if self.active {
            key.active_render
        } else {
            let Some(obs) = key.obs.as_deref() else {
                return;
            };
            let base = match self.accs.get(idx).and_then(|(_, a)| a.last()) {
                Some(last) => last,
                None => obs.shown,
            };
            let learner = &self.learner;
            let scorer = self.scorer.get_or_insert_with(|| learner.snapshot());
            let p = scorer.score(obs).p_vi;
            if !p.is_finite() {
                base
            } else if obs.mask.commit || (p - 0.5).abs() > key.margin {
                Lang::from_p_vi(p)
            } else {
                base
            }
        };
        if let Some((_, acc)) = self.accs.get_mut(idx) {
            acc.push(display);
        }
    }

    /// Counts the finalized word, before the learner sees it. The active
    /// model's rendering is the committed one; a shadow's is its own display
    /// decision at the commit step (its latest one when the word ended
    /// without a commit step). A shadow that saw no keystroke of the word
    /// does not count it.
    fn record_word(&mut self, info: WordInfo, out: &mut Vec<TrainerOut>) {
        if !self.counts() {
            return;
        }
        let found = match self.accs.iter().rposition(|(id, _)| *id == info.word_id) {
            Some(i) => self.accs.remove(i).map(|(_, a)| a),
            None => None,
        };
        let (last, acc) = match found {
            Some(a) => (a.last(), a),
            None => (None, WordAcc::default()),
        };
        let corrected = if self.active {
            info.corrected
        } else {
            let Some(last) = last else {
                return;
            };
            last != info.label
        };
        self.stats.record(WordResult {
            corrected,
            category: info.category,
            served: self.active,
            acc,
            label: info.label,
        });
        out.push(TrainerOut::Stats(self.kind, self.stats.summary()));
    }

    /// Learns from the examples of one finalized word (all its steps):
    /// one update over the whole word, then the correction and replay
    /// schedule as for a single sample. Render-only examples that are not a
    /// correction are dropped when the learner does not learn from
    /// confirmations ([`ImeLearner::learns_from_confirmations`]).
    pub(crate) fn on_examples(
        &mut self,
        examples: Vec<Example>,
        correction: bool,
        info: Option<WordInfo>,
        out: &mut Vec<TrainerOut>,
    ) {
        if let Some(info) = info {
            self.record_word(info, out);
        }
        if examples.is_empty() || !self.learn {
            return;
        }
        let confirmation = !correction && examples.iter().all(|ex| ex.edit_weight <= 0.0);
        if confirmation && !self.learner.learns_from_confirmations() {
            return;
        }
        let new = examples.len();
        if self.params.replay_capacity > 0 {
            for ex in &examples {
                if self.replay.len() >= self.params.replay_capacity {
                    self.replay.pop_front();
                }
                self.replay.push_back(ex.clone());
            }
        }
        self.batch.clear();
        self.batch.extend(examples);
        self.update_batch(out);

        if correction && self.params.correction_steps > 0 && self.params.correction_window > 0 {
            let start = self
                .replay
                .len()
                .saturating_sub(self.params.correction_window);
            let mut losses = 0.0;
            let mut n = 0u32;
            let mut steps = 0u32;
            for _ in 0..self.params.correction_steps {
                self.batch.clear();
                self.batch.extend(self.replay.range(start..).cloned());
                if self.batch.is_empty() {
                    break;
                }
                n = n.saturating_add(count(self.batch.len()));
                losses += self.learner.update(&self.batch);
                self.touched();
                steps += 1;
                self.since_publish += 1;
            }
            self.batch.clear();
            if steps > 0 {
                out.push(TrainerOut::Update {
                    policy_id: self.learner.id().to_owned(),
                    n_samples: n,
                    loss: losses / steps as f32,
                });
            }
        }

        self.since_replay += new;
        if self.params.replay_every > 0
            && self.since_replay >= self.params.replay_every
            && !self.replay.is_empty()
            && self.params.replay_batch > 0
        {
            self.since_replay = 0;
            self.batch.clear();
            for _ in 0..self.params.replay_batch {
                let len = self.replay.len() as u64;
                // `idx < len`, which came from a usize, so the cast is lossless.
                let idx = (self.next_u64() % len) as usize;
                if let Some(s) = self.replay.get(idx) {
                    self.batch.push(s.clone());
                }
            }
            self.update_batch(out);
        }

        if self.active && (correction || self.since_publish >= PUBLISH_EVERY) {
            self.since_publish = 0;
            out.push(TrainerOut::Snapshot(self.learner.snapshot()));
        }
    }

    /// Saves what changed: the learner (when online training is on and it
    /// is active or was updated, so an untouched shadow never copies a
    /// bundled model into the user layer) and the statistics.
    ///
    /// # Errors
    /// Returns the first storage error (both writes are attempted).
    pub(crate) fn save(&mut self) -> io::Result<()> {
        let mut result = Ok(());
        if self.learn && (self.active || self.dirty) {
            result = self
                .save
                .storage
                .blobs()
                .put(&self.save.key, &self.learner.save());
            if result.is_ok() {
                self.dirty = false;
            }
        }
        if self.stats.is_dirty() {
            let key = stats_key(self.kind);
            let put = match self.stats.to_bytes() {
                Ok(bytes) => self.save.storage.blobs().put(&key, &bytes),
                Err(e) => Err(io::Error::other(e)),
            };
            if put.is_ok() {
                self.stats.mark_saved();
            }
            result = result.and(put);
        }
        result
    }
}

/// Blob key of the statistics of `kind`.
pub(crate) fn stats_key(kind: ModelKind) -> String {
    format!("stats/{}", kind.name())
}

fn dispatch_key(cores: &mut [Core], key: &KeyObs) {
    for core in cores {
        core.on_key(key);
    }
}

/// Hands one word to every core (the last one takes the examples by value).
fn dispatch(
    cores: &mut [Core],
    examples: Vec<Example>,
    correction: bool,
    info: Option<WordInfo>,
    out: &mut Vec<TrainerOut>,
) {
    let Some((last, rest)) = cores.split_last_mut() else {
        return;
    };
    for core in rest {
        core.on_examples(examples.clone(), correction, info, out);
    }
    last.on_examples(examples, correction, info, out);
}

/// Saves every core, reporting failures as diagnostics.
fn save_all(cores: &mut [Core], out: &mut Vec<TrainerOut>) {
    for core in cores {
        if let Err(e) = core.save() {
            out.push(TrainerOut::Diagnostic(format!("snapshot save failed: {e}")));
        }
    }
}

/// Saves every core; the first error wins.
fn save_final(cores: &mut [Core]) -> io::Result<()> {
    let mut result = Ok(());
    for core in cores {
        let r = core.save();
        if result.is_ok() {
            result = r;
        }
    }
    result
}

/// Saturating `usize` -> `u32` for sample counts.
fn count(n: usize) -> u32 {
    n.min(u32::MAX as usize) as u32
}

enum Msg {
    Examples(Vec<Example>, bool, Option<WordInfo>),
    Key(Box<KeyObs>),
    Learn(ModelKind, bool),
    Save,
}

fn worker(mut cores: Vec<Core>, rx: Receiver<Msg>, tx: Sender<TrainerOut>) -> io::Result<()> {
    let mut out = Vec::new();
    for msg in rx {
        match msg {
            Msg::Examples(xs, correction, info) => {
                dispatch(&mut cores, xs, correction, info, &mut out);
            }
            Msg::Key(k) => dispatch_key(&mut cores, &k),
            Msg::Learn(kind, on) => set_learn(&mut cores, kind, on),
            Msg::Save => save_all(&mut cores, &mut out),
        }
        for o in out.drain(..) {
            if tx.send(o).is_err() {
                break;
            }
        }
    }
    save_final(&mut cores)
}

fn set_learn(cores: &mut [Core], kind: ModelKind, on: bool) {
    for core in cores.iter_mut().filter(|c| c.kind == kind) {
        core.learn = on;
    }
}

enum Backend {
    Inline {
        cores: Vec<Core>,
        queue: Vec<TrainerOut>,
    },
    Background {
        tx: Sender<Msg>,
        rx: Receiver<TrainerOut>,
        handle: JoinHandle<io::Result<()>>,
    },
}

/// Engine-side handle to the trainer.
pub(crate) struct Trainer {
    backend: Backend,
    /// Warnings raised on the engine side (e.g. trainer thread gone).
    notes: Vec<String>,
}

impl Trainer {
    /// Starts a trainer over `cores`. `background` moves them to a new
    /// thread; if the thread cannot be spawned it runs inline (reported via
    /// [`TrainerOut::Diagnostic`]).
    #[must_use]
    pub(crate) fn new(cores: Vec<Core>, background: bool) -> Self {
        if !background {
            return Self {
                backend: Backend::Inline {
                    cores,
                    queue: Vec::new(),
                },
                notes: Vec::new(),
            };
        }
        let (tx, rx) = mpsc::channel::<Msg>();
        let (otx, orx) = mpsc::channel::<TrainerOut>();
        // The cores travel through a one-shot channel so they survive a
        // failed spawn.
        let (ctx, crx) = mpsc::channel::<Vec<Core>>();
        let spawned = std::thread::Builder::new()
            .name("nimk-trainer".into())
            .spawn(move || match crx.recv() {
                Ok(cores) => worker(cores, rx, otx),
                Err(_) => Ok(()),
            });
        let reason = match spawned {
            Ok(handle) => match ctx.send(cores) {
                Ok(()) => {
                    return Self {
                        backend: Backend::Background {
                            tx,
                            rx: orx,
                            handle,
                        },
                        notes: Vec::new(),
                    };
                }
                Err(mpsc::SendError(cores)) => (cores, "trainer thread exited early".to_owned()),
            },
            Err(e) => (cores, format!("cannot spawn trainer thread ({e})")),
        };
        let (cores, msg) = reason;
        let queue = vec![TrainerOut::Diagnostic(format!("{msg}; training inline"))];
        Self {
            backend: Backend::Inline { cores, queue },
            notes: Vec::new(),
        }
    }

    /// Submits the examples of one finalized word; `info` (with a word
    /// label) also feeds the statistics.
    pub(crate) fn submit(
        &mut self,
        examples: Vec<Example>,
        correction: bool,
        info: Option<WordInfo>,
    ) {
        match &mut self.backend {
            Backend::Inline { cores, queue } => {
                dispatch(cores, examples, correction, info, queue);
            }
            Backend::Background { tx, .. } => {
                if tx.send(Msg::Examples(examples, correction, info)).is_err() {
                    self.notes
                        .push("trainer thread gone; sample dropped".into());
                }
            }
        }
    }

    /// Submits one keystroke's model call (statistics only).
    pub(crate) fn submit_key(&mut self, key: KeyObs) {
        match &mut self.backend {
            Backend::Inline { cores, .. } => dispatch_key(cores, &key),
            Backend::Background { tx, .. } => {
                if tx.send(Msg::Key(Box::new(key))).is_err() {
                    self.notes
                        .push("trainer thread gone; keystroke dropped".into());
                }
            }
        }
    }

    /// Turns online training of `kind` on or off.
    pub(crate) fn set_learn(&mut self, kind: ModelKind, on: bool) {
        match &mut self.backend {
            Backend::Inline { cores, .. } => set_learn(cores, kind, on),
            Backend::Background { tx, .. } => {
                if tx.send(Msg::Learn(kind, on)).is_err() {
                    self.notes
                        .push("trainer thread gone; setting not applied".into());
                }
            }
        }
    }

    /// Requests a snapshot save to storage.
    pub(crate) fn request_save(&mut self) {
        match &mut self.backend {
            Backend::Inline { cores, queue } => save_all(cores, queue),
            Backend::Background { tx, .. } => {
                if tx.send(Msg::Save).is_err() {
                    self.notes
                        .push("trainer thread gone; snapshot not saved".into());
                }
            }
        }
    }

    /// Drains pending outputs without blocking.
    pub(crate) fn drain(&mut self, out: &mut Vec<TrainerOut>) {
        out.extend(self.notes.drain(..).map(TrainerOut::Diagnostic));
        match &mut self.backend {
            Backend::Inline { queue, .. } => out.append(queue),
            Backend::Background { rx, .. } => out.extend(rx.try_iter()),
        }
    }

    /// Stops training, saves final snapshots and returns the outputs not
    /// yet drained.
    ///
    /// # Errors
    /// Returns the save error, or an error if the trainer thread panicked.
    pub(crate) fn shutdown(self, out: &mut Vec<TrainerOut>) -> io::Result<()> {
        match self.backend {
            Backend::Inline {
                mut cores,
                mut queue,
            } => {
                out.append(&mut queue);
                save_final(&mut cores)
            }
            Backend::Background { tx, rx, handle } => {
                drop(tx);
                let res = handle
                    .join()
                    .map_err(|_| io::Error::other("trainer thread panicked"));
                out.extend(rx.try_iter());
                res?
            }
        }
    }
}

#[cfg(test)]
mod tests;
