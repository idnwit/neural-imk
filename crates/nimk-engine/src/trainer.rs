//! Trainer: online updates, correction re-training, replay buffer and
//! model snapshots.
//!
//! The same [`Core`] logic runs either on a background thread (production)
//! or synchronously on the caller's thread (`Inline`, deterministic). The
//! engine submits samples and drains [`TrainerOut`] messages at word
//! boundaries; new policies arrive as `Arc<dyn Policy>` snapshots, so the
//! hot path never takes a lock.

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use nimk_policy::{Learner, Policy};
use nimk_storage::Storage;
use nimk_types::Sample;

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
        /// Samples in the update.
        n_samples: u32,
        /// Mean weighted loss before the update.
        loss: f32,
    },
    /// New read-only policy to serve predictions.
    Snapshot(Arc<dyn Policy>),
    /// Human-readable warning.
    Diagnostic(String),
}

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
    learner: Box<dyn Learner>,
    params: TrainerParams,
    save: SaveTarget,
    replay: VecDeque<Sample>,
    rng: u64,
    since_replay: usize,
    since_publish: usize,
    batch: Vec<Sample>,
}

impl Core {
    /// Wraps a learner.
    #[must_use]
    pub(crate) fn new(learner: Box<dyn Learner>, params: TrainerParams, save: SaveTarget) -> Self {
        let cap = params.replay_capacity.min(1024);
        Self {
            learner,
            params,
            save,
            replay: VecDeque::with_capacity(cap),
            rng: params.seed ^ 0x9E37_79B9_7F4A_7C15,
            since_replay: 0,
            since_publish: 0,
            batch: Vec::with_capacity(params.replay_batch.max(params.correction_window).min(1024)),
        }
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
        let n_samples = count(self.batch.len());
        self.batch.clear();
        self.since_publish += 1;
        out.push(TrainerOut::Update {
            policy_id: self.learner.id().to_owned(),
            n_samples,
            loss,
        });
    }

    /// Learns from one finalized sample.
    pub(crate) fn on_sample(
        &mut self,
        sample: Sample,
        correction: bool,
        out: &mut Vec<TrainerOut>,
    ) {
        if self.params.replay_capacity > 0 {
            if self.replay.len() >= self.params.replay_capacity {
                self.replay.pop_front();
            }
            self.replay.push_back(sample.clone());
        }
        self.batch.clear();
        self.batch.push(sample);
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

        self.since_replay += 1;
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

        if correction || self.since_publish >= PUBLISH_EVERY {
            self.since_publish = 0;
            out.push(TrainerOut::Snapshot(self.learner.snapshot()));
        }
    }

    /// Saves the learner to storage.
    ///
    /// # Errors
    /// Returns the storage error.
    pub(crate) fn save(&self) -> io::Result<()> {
        self.save
            .storage
            .blobs()
            .put(&self.save.key, &self.learner.save())
    }
}

/// Saturating `usize` -> `u32` for sample counts.
fn count(n: usize) -> u32 {
    n.min(u32::MAX as usize) as u32
}

enum Msg {
    Sample(Sample, bool),
    Save,
}

fn worker(mut core: Core, rx: Receiver<Msg>, tx: Sender<TrainerOut>) -> io::Result<()> {
    let mut out = Vec::new();
    for msg in rx {
        match msg {
            Msg::Sample(s, correction) => core.on_sample(s, correction, &mut out),
            Msg::Save => {
                if let Err(e) = core.save() {
                    out.push(TrainerOut::Diagnostic(format!("snapshot save failed: {e}")));
                }
            }
        }
        for o in out.drain(..) {
            if tx.send(o).is_err() {
                break;
            }
        }
    }
    core.save()
}

enum Backend {
    Inline {
        core: Box<Core>,
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
    /// Starts a trainer. `background` moves `core` to a new thread; if the
    /// thread cannot be spawned it runs inline (reported via [`TrainerOut::Diagnostic`]).
    #[must_use]
    pub(crate) fn new(core: Core, background: bool) -> Self {
        if !background {
            return Self {
                backend: Backend::Inline {
                    core: Box::new(core),
                    queue: Vec::new(),
                },
                notes: Vec::new(),
            };
        }
        let (tx, rx) = mpsc::channel::<Msg>();
        let (otx, orx) = mpsc::channel::<TrainerOut>();
        // The core travels through a one-shot channel so it survives a
        // failed spawn.
        let (ctx, crx) = mpsc::channel::<Box<Core>>();
        let spawned = std::thread::Builder::new()
            .name("nimk-trainer".into())
            .spawn(move || match crx.recv() {
                Ok(core) => worker(*core, rx, otx),
                Err(_) => Ok(()),
            });
        let core = Box::new(core);
        let reason = match spawned {
            Ok(handle) => match ctx.send(core) {
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
                Err(mpsc::SendError(core)) => (core, "trainer thread exited early".to_owned()),
            },
            Err(e) => (core, format!("cannot spawn trainer thread ({e})")),
        };
        let (core, msg) = reason;
        let queue = vec![TrainerOut::Diagnostic(format!("{msg}; training inline"))];
        Self {
            backend: Backend::Inline { core, queue },
            notes: Vec::new(),
        }
    }

    /// Submits one finalized sample.
    pub(crate) fn submit(&mut self, sample: Sample, correction: bool) {
        match &mut self.backend {
            Backend::Inline { core, queue } => core.on_sample(sample, correction, queue),
            Backend::Background { tx, .. } => {
                if tx.send(Msg::Sample(sample, correction)).is_err() {
                    self.notes
                        .push("trainer thread gone; sample dropped".into());
                }
            }
        }
    }

    /// Requests a snapshot save to storage.
    pub(crate) fn request_save(&mut self) {
        match &mut self.backend {
            Backend::Inline { core, queue } => {
                if let Err(e) = core.save() {
                    queue.push(TrainerOut::Diagnostic(format!("snapshot save failed: {e}")));
                }
            }
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

    /// Stops training, saves a final snapshot and returns the outputs not
    /// yet drained.
    ///
    /// # Errors
    /// Returns the save error, or an error if the trainer thread panicked.
    pub(crate) fn shutdown(self, out: &mut Vec<TrainerOut>) -> io::Result<()> {
        match self.backend {
            Backend::Inline { core, mut queue } => {
                out.append(&mut queue);
                core.save()
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
