//! `LinearAction`: two linear heads over hashed sparse features (hand-written).
//!
//! Inputs are the shared [`Encoded`] observation turned into hashed n-gram
//! ids ([`SparseFeats`]) plus the dense scalars. One weight row of
//! [`OUT`] floats per input: outputs `0..4` are the edit logits (softmax over
//! [`EditAction::ALL`]), output `4` is the render logit (sigmoid = `p_vi`).
//! All weights start at zero, so an untrained model is exactly neutral.

use std::sync::Arc;

use nimk_types::{ActionScores, Example, Observation};

use crate::PolicyError;
use crate::action_model::{ImeLearner, ImeModel};
use crate::codec::{Reader, Writer, sat_u32, sigmoid};
use crate::encode::{Encoded, SCALARS, SEQ_LEN, fnv1a, tok};

const ID: &str = "linear_action@v1";
/// Snapshot kind string.
pub(crate) const KIND: &str = "linear_action";
const VERSION: u32 = 1;

/// Hash buckets for sparse features.
pub const ACTION_BUCKETS: u32 = 1 << 15;
/// Default SGD step size (per row, see [`LinearAction`]).
pub const LINEAR_LR: f32 = 0.1;
/// Step-size factor for render-only examples (`edit_weight == 0`: online
/// labels). Online labels arrive one word at a time, so at the full step the
/// model chases the last few words (synthetic replay: 94.5% word accuracy
/// online vs 96.6% frozen); at 0.03 online learning is a small gain (97.0%).
pub const LINEAR_ONLINE_SCALE: f32 = 0.03;
/// Outputs per weight row: 4 edit logits + 1 render logit.
const OUT: usize = 5;
/// Maximum sparse ids per observation (bounded by [`SEQ_LEN`]).
const MAX_FEATS: usize = 8 * SEQ_LEN;
const ROWS: usize = ACTION_BUCKETS as usize + SCALARS + 1;
const BIAS_ROW: usize = ROWS - 1;

/// Hashed sparse feature rows of one observation; fixed capacity, no heap.
pub(crate) struct SparseFeats {
    ids: [u32; MAX_FEATS],
    n: usize,
}

impl SparseFeats {
    /// Extracts the features of `e`.
    pub(crate) fn new(e: &Encoded) -> Self {
        let mut f = Self {
            ids: [0; MAX_FEATS],
            n: 0,
        };
        let t = &e.tokens;
        let key = t[SEQ_LEN - 1];
        let first = SEQ_LEN - e.len;
        let start = e.buffer_start();
        // Positional unigrams of the 4 newest tokens.
        for i in 0..4 {
            f.push(1, &[i as u8, t[SEQ_LEN - 1 - i]]);
        }
        // The word so far, framed as `^raw$`: 1..4-grams.
        let mut word = [tok::PAD; SEQ_LEN + 1];
        let raw = &t[start..SEQ_LEN - 1];
        word[0] = tok::SEP;
        word[1..=raw.len()].copy_from_slice(raw);
        word[raw.len() + 1] = tok::PAD;
        let word = &word[..raw.len() + 2];
        for n in 1..=4u8 {
            for g in word.windows(usize::from(n)) {
                f.push(1 + u32::from(n), g);
            }
        }
        // Key interactions with structural state.
        let mask = (e.scalars[crate::encode::scalar::MASK_APPEND] > 0.0) as u8
            | ((e.scalars[crate::encode::scalar::MASK_POP] > 0.0) as u8) << 1
            | ((e.scalars[crate::encode::scalar::MASK_COMMIT] > 0.0) as u8) << 2;
        f.push(6, &[key, mask]);
        f.push(7, &[raw.last().map_or(tok::SEP, |&t| t), key]);
        f.push(8, &[key, raw.len().min(3) as u8]);
        // History before the buffer: bag of bigrams.
        if start >= first + 2 {
            for g in t[first..start - 1].windows(2) {
                f.push(9, g);
            }
        }
        // App context.
        let b = e.bundle.to_le_bytes();
        f.push(10, &b);
        f.push(11, &[e.caret]);
        f.push(12, &[b[0], b[1], e.caret]);
        f
    }

    fn push(&mut self, ns: u32, bytes: &[u8]) {
        if let Some(slot) = self.ids.get_mut(self.n) {
            *slot = fnv1a(ns, bytes) % ACTION_BUCKETS;
            self.n += 1;
        }
    }

    pub(crate) fn ids(&self) -> &[u32] {
        &self.ids[..self.n]
    }
}

/// Weights shared by learner and snapshot: `ROWS x OUT`, row-major.
#[derive(Debug, Clone)]
struct Weights {
    w: Vec<f32>,
}

impl Weights {
    fn zeros() -> Self {
        Self {
            w: vec![0.0; ROWS * OUT],
        }
    }

    fn row(&self, r: usize) -> &[f32] {
        self.w.get(r * OUT..r * OUT + OUT).map_or(&[], |row| row)
    }

    /// Raw logits for `(feats, e)`.
    fn logits(&self, feats: &SparseFeats, e: &Encoded) -> [f32; OUT] {
        let mut z = [0.0f32; OUT];
        let mut add = |row: &[f32], x: f32| {
            for (z, w) in z.iter_mut().zip(row) {
                *z += x * w;
            }
        };
        for &id in feats.ids() {
            add(self.row(id as usize), 1.0);
        }
        for (s, &x) in e.scalars.iter().enumerate() {
            if x != 0.0 {
                add(self.row(ACTION_BUCKETS as usize + s), x);
            }
        }
        add(self.row(BIAS_ROW), 1.0);
        z
    }

    fn score(&self, obs: &Observation) -> ActionScores {
        let e = Encoded::new(obs);
        scores_from_logits(&self.logits(&SparseFeats::new(&e), &e))
    }
}

/// Softmax over the 4 edit logits and sigmoid of the render logit.
fn scores_from_logits(z: &[f32; OUT]) -> ActionScores {
    let mut edit = [z[0], z[1], z[2], z[3]];
    let max = edit.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut sum = 0.0;
    for v in &mut edit {
        *v = (*v - max).exp();
        sum += *v;
    }
    for v in &mut edit {
        *v /= sum;
    }
    ActionScores {
        edit,
        p_vi: sigmoid(z[4]),
    }
}

/// Weighted loss terms of one example: `(edit CE, render BCE)`, unweighted.
pub(crate) fn example_losses(s: &ActionScores, ex: &Example) -> (f32, f32) {
    let pe = s.edit[ex.target.edit.index()].max(1e-7);
    (
        -pe.ln(),
        crate::codec::log_loss(s.p_vi, ex.target.render.as_target()),
    )
}

/// Read-only snapshot of a [`LinearAction`].
struct LinearActionModel {
    weights: Weights,
}

impl ImeModel for LinearActionModel {
    fn id(&self) -> &str {
        ID
    }

    fn score(&self, obs: &Observation) -> ActionScores {
        self.weights.score(obs)
    }
}

/// Two linear heads (4-way softmax edit, sigmoid render) over hashed char
/// n-grams of the token window, key x mask interactions, app context and
/// the dense scalars. Trained with plain minibatch SGD on the rows the batch
/// touches.
///
/// Why SGD and not Adam: Adam's per-coordinate normalization gives a rare
/// hashed n-gram the same step as the key token, so memorized word n-grams
/// override structural evidence on unseen words (toy task, held-out words:
/// Adam 97% edit / 83% render accuracy, SGD 100% / 94%). SGD lets frequent
/// features accumulate proportionally more weight.
///
/// Loss per batch (reported): `sum(edit_weight * CE + render_weight * BCE) / len`.
///
/// Step: each touched weight row moves by `lr * sum / sqrt(hits)`, where
/// `sum` is the summed gradient of the `hits` examples in the batch that
/// touched it. A dense row (bias, scalars) is touched by every example; a
/// hashed n-gram usually by one. Plain batch-mean SGD starved rare n-grams
/// in 128-example pre-training batches, per-row mean (`sum / hits`)
/// memorized word n-grams and lost unseen words (toy task 0.80), per-example
/// SGD let the dense rows chase each example; `sqrt` balances in-vocabulary
/// accuracy (synthetic held-out render 0.967) and unseen words (toy 0.92).
/// Render-only examples are scaled by [`LINEAR_ONLINE_SCALE`].
///
/// Edit logits are trained unmasked; the engine applies [`nimk_types::ActionMask`]
/// at inference.
///
/// Snapshot payload: `buckets: u32, scalars: u32, weights (ROWS x 5 f32)`.
/// SGD keeps no optimizer state, so save/load is lossless.
#[derive(Debug, Clone)]
pub struct LinearAction {
    weights: Weights,
    lr: f32,
    g: Vec<f32>,
    touched: Vec<u32>,
    /// Examples of the current batch that touched each row (0 = untouched).
    hits: Vec<u32>,
}

impl Default for LinearAction {
    fn default() -> Self {
        Self::new()
    }
}

impl LinearAction {
    /// Creates a neutral (all-zero) model with step size [`LINEAR_LR`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_lr(LINEAR_LR)
    }

    /// Creates a neutral model with an explicit SGD step size (applied to
    /// the per-row mean gradient, see [`LinearAction`]).
    #[must_use]
    pub fn with_lr(lr: f32) -> Self {
        Self {
            weights: Weights::zeros(),
            lr,
            g: vec![0.0; ROWS * OUT],
            touched: Vec::with_capacity(4096),
            hits: vec![0; ROWS],
        }
    }

    fn touch(&mut self, row: usize) {
        if let Some(h) = self.hits.get_mut(row) {
            if *h == 0 {
                self.touched.push(sat_u32(row));
            }
            *h = h.saturating_add(1);
        }
    }

    /// Adds `x * dz` to the gradient of `row`.
    fn accumulate(&mut self, row: usize, x: f32, dz: &[f32; OUT]) {
        self.touch(row);
        if let Some(g) = self.g.get_mut(row * OUT..row * OUT + OUT) {
            for (g, d) in g.iter_mut().zip(dz) {
                *g += x * d;
            }
        }
    }

    fn backward(&mut self, ex: &Example, scale: f32) -> f32 {
        let e = Encoded::new(&ex.obs);
        let feats = SparseFeats::new(&e);
        let s = scores_from_logits(&self.weights.logits(&feats, &e));
        let (ce, bce) = example_losses(&s, ex);
        let mut dz = [0.0f32; OUT];
        let target = ex.target.edit.index();
        for (k, d) in dz.iter_mut().take(4).enumerate() {
            let y = if k == target { 1.0 } else { 0.0 };
            *d = (s.edit[k] - y) * ex.edit_weight * scale;
        }
        dz[4] = (s.p_vi - ex.target.render.as_target()) * ex.render_weight * scale;
        if dz.iter().all(|d| d.is_finite()) {
            for &id in feats.ids() {
                self.accumulate(id as usize, 1.0, &dz);
            }
            for (i, &x) in e.scalars.iter().enumerate() {
                if x != 0.0 {
                    self.accumulate(ACTION_BUCKETS as usize + i, x, &dz);
                }
            }
            self.accumulate(BIAS_ROW, 1.0, &dz);
        }
        ex.edit_weight * ce + ex.render_weight * bce
    }

    fn apply(&mut self) {
        let lr = self.lr;
        for &row in &self.touched {
            let r = row as usize;
            let hits = self.hits.get_mut(r).map_or(1, std::mem::take).max(1);
            let step = lr / (hits as f32).sqrt();
            let span = r * OUT..r * OUT + OUT;
            let (Some(w), Some(g)) = (self.weights.w.get_mut(span.clone()), self.g.get_mut(span))
            else {
                continue;
            };
            for (w, g) in w.iter_mut().zip(g.iter_mut()) {
                *w -= step * *g;
                *g = 0.0;
            }
        }
        self.touched.clear();
    }
}

impl ImeLearner for LinearAction {
    fn id(&self) -> &str {
        ID
    }

    fn update(&mut self, batch: &[Example]) -> f32 {
        if batch.is_empty() {
            return 0.0;
        }
        let inv = 1.0 / batch.len() as f32;
        let mut loss = 0.0;
        for ex in batch {
            let scale = if ex.edit_weight > 0.0 {
                1.0
            } else {
                LINEAR_ONLINE_SCALE
            };
            loss += self.backward(ex, scale);
        }
        self.apply();
        loss * inv
    }

    fn snapshot(&self) -> Arc<dyn ImeModel> {
        Arc::new(LinearActionModel {
            weights: self.weights.clone(),
        })
    }

    fn save(&self) -> Vec<u8> {
        let mut w = Writer::new(KIND, VERSION, 8 + self.weights.w.len() * 4);
        w.u32(ACTION_BUCKETS);
        w.len(SCALARS);
        w.f32s(&self.weights.w);
        w.finish()
    }

    fn load(&mut self, bytes: &[u8]) -> Result<(), PolicyError> {
        let mut r = Reader::open(bytes, KIND, VERSION)?;
        r.expect_dim(ACTION_BUCKETS, "sparse buckets")?;
        r.expect_dim(sat_u32(SCALARS), "scalars")?;
        let w = r.f32s(ROWS * OUT)?;
        r.finish()?;
        *self = Self::with_lr(self.lr);
        self.weights.w = w;
        Ok(())
    }
}
