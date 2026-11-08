use std::collections::BTreeMap;
use std::sync::Arc;

use nimk_types::{Features, Sample};

use crate::codec::{Reader, Writer, log_loss, sat_u32, sigmoid, weighted_mean};
use crate::mlp_net::{Acts, EMB, HIDDEN, MlpPolicy, Net};
use crate::{Learner, Policy, PolicyError};

const ID: &str = "tiny_mlp@v1";
const KIND: &str = "tiny_mlp";
const VERSION: u32 = 1;

/// Adam hyper-parameters (Kingma & Ba 2015).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdamParams {
    /// Step size.
    pub lr: f32,
    /// First-moment decay.
    pub beta1: f32,
    /// Second-moment decay.
    pub beta2: f32,
    /// Denominator epsilon.
    pub eps: f32,
}

impl Default for AdamParams {
    /// `lr = 0.01, beta1 = 0.9, beta2 = 0.999, eps = 1e-8`.
    fn default() -> Self {
        Self {
            lr: 0.01,
            beta1: 0.9,
            beta2: 0.999,
            eps: 1e-8,
        }
    }
}

/// Adam state for one parameter tensor; `m`/`v` mirror the parameters.
#[derive(Debug, Clone)]
struct Moments {
    m: Vec<f32>,
    v: Vec<f32>,
}

impl Moments {
    fn zeros(n: usize) -> Self {
        Self {
            m: vec![0.0; n],
            v: vec![0.0; n],
        }
    }
}

/// Applies one Adam step to `params[i]` for every `grads[i]`, using moments
/// at the same offsets. `lr_t` already includes bias correction.
#[inline]
fn adam(
    p: &AdamParams,
    lr_t: f32,
    params: &mut [f32],
    grads: &[f32],
    m: &mut [f32],
    v: &mut [f32],
) {
    for (((w, &g), m), v) in params
        .iter_mut()
        .zip(grads)
        .zip(m.iter_mut())
        .zip(v.iter_mut())
    {
        *m = p.beta1 * *m + (1.0 - p.beta1) * g;
        *v = p.beta2 * *v + (1.0 - p.beta2) * g * g;
        *w -= lr_t * *m / (v.sqrt() + p.eps);
    }
}

/// Embedding bag (dim 16) over sparse ids, concatenated with dense features, one hidden layer of 64 ReLU units, sigmoid output. Hand-written forward/backward, Adam.
///
/// Each [`Learner::update`] is one Adam step on the batch gradient
/// `sum(weight_i * grad_i) / batch_len`. Embedding rows use lazy Adam: only
/// rows touched by the batch are updated (global step for bias correction).
///
/// Snapshot payload: `buckets: u32, dense_len: u32`, embeddings
/// (`buckets x 16` f32), head (see `mlp_net::Net`). Adam moments are not
/// saved; a loaded model restarts its optimizer state. [`Learner::id`] is
/// the fixed `tiny_mlp@v1`.
#[derive(Debug, Clone)]
pub struct TinyMlp {
    net: Net,
    adam: AdamParams,
    emb_moments: Moments,
    head_moments: Moments,
    beta1_pow: f32,
    beta2_pow: f32,
    /// Reused gradient buffer for the head.
    g_head: Vec<f32>,
}

impl TinyMlp {
    /// Creates an untrained model.
    #[must_use]
    pub fn new(sparse_buckets: u32, dense_len: usize, seed: u64) -> Self {
        Self::with_params(sparse_buckets, dense_len, seed, AdamParams::default())
    }

    /// Creates an untrained model with explicit optimizer settings.
    #[must_use]
    pub fn with_params(sparse_buckets: u32, dense_len: usize, seed: u64, adam: AdamParams) -> Self {
        let net = Net::init(sparse_buckets.max(1), dense_len, seed);
        let emb_moments = Moments::zeros(net.emb.len());
        let head_moments = Moments::zeros(net.head.len());
        let g_head = vec![0.0; net.head.len()];
        Self {
            net,
            adam,
            emb_moments,
            head_moments,
            beta1_pow: 1.0,
            beta2_pow: 1.0,
            g_head,
        }
    }

    /// Accumulates the gradient of `scale * log_loss` for one sample into
    /// `self.g_head` and `g_emb`; returns `p(VI)` before the update.
    fn backward(&mut self, s: &Sample, scale: f32, g_emb: &mut BTreeMap<u32, [f32; EMB]>) -> f32 {
        let f: &Features = &s.features;
        let mut acts = Acts::new();
        let p = sigmoid(self.net.forward(f, &mut acts));
        let dz = (p - s.label.as_target()) * scale;
        if dz == 0.0 || !dz.is_finite() {
            return p;
        }
        let dense_len = self.net.dense_len;
        let input = Net::input_len(dense_len);
        let head = self.net.head();
        let (g_w1, rest) = self.g_head.split_at_mut(HIDDEN * input);
        let (g_b1, rest) = rest.split_at_mut(HIDDEN);
        let (g_w2, g_b2) = rest.split_at_mut(HIDDEN);
        if let Some(b2) = g_b2.first_mut() {
            *b2 += dz;
        }
        let dense = &f.dense[..dense_len.min(f.dense.len())];
        let mut d_bag = [0.0f32; EMB];
        let rows = head
            .w1
            .chunks_exact(input)
            .zip(g_w1.chunks_exact_mut(input));
        for (j, (w_row, g_row)) in rows.enumerate() {
            let (h, w2) = (acts.h[j], head.w2[j]);
            g_w2[j] += dz * h;
            if h <= 0.0 {
                continue;
            }
            let dh = dz * w2;
            g_b1[j] += dh;
            let (g_e, g_d) = g_row.split_at_mut(EMB);
            for (k, g) in g_e.iter_mut().enumerate() {
                *g += dh * acts.bag[k];
                d_bag[k] += dh * w_row[k];
            }
            for (g, &x) in g_d.iter_mut().zip(dense) {
                *g += dh * x;
            }
        }
        if acts.count > 0 {
            let inv = 1.0 / acts.count as f32;
            for &id in &f.sparse {
                let g = g_emb.entry(self.net.row_of(id)).or_insert([0.0; EMB]);
                for (g, d) in g.iter_mut().zip(&d_bag) {
                    *g += d * inv;
                }
            }
        }
        p
    }

    fn apply(&mut self, g_emb: &BTreeMap<u32, [f32; EMB]>) {
        let a = self.adam;
        self.beta1_pow *= a.beta1;
        self.beta2_pow *= a.beta2;
        let lr_t = a.lr * (1.0 - self.beta2_pow).sqrt() / (1.0 - self.beta1_pow);
        let hm = &mut self.head_moments;
        adam(
            &a,
            lr_t,
            &mut self.net.head,
            &self.g_head,
            &mut hm.m,
            &mut hm.v,
        );
        let em = &mut self.emb_moments;
        for (&row, g) in g_emb {
            let r = row as usize * EMB..row as usize * EMB + EMB;
            if let (Some(w), Some(m), Some(v)) = (
                self.net.emb.get_mut(r.clone()),
                em.m.get_mut(r.clone()),
                em.v.get_mut(r),
            ) {
                adam(&a, lr_t, w, g, m, v);
            }
        }
    }
}

impl Learner for TinyMlp {
    fn id(&self) -> &str {
        ID
    }

    fn update(&mut self, batch: &[Sample]) -> f32 {
        if batch.is_empty() {
            return 0.0;
        }
        self.g_head.iter_mut().for_each(|g| *g = 0.0);
        let mut g_emb = BTreeMap::new();
        let inv_len = 1.0 / batch.len() as f32;
        let (mut num, mut den) = (0.0, 0.0);
        for s in batch {
            let p = self.backward(s, s.weight * inv_len, &mut g_emb);
            num += s.weight * log_loss(p, s.label.as_target());
            den += s.weight;
        }
        self.apply(&g_emb);
        weighted_mean(num, den)
    }

    fn snapshot(&self) -> Arc<dyn Policy> {
        Arc::new(MlpPolicy {
            id: ID,
            net: self.net.clone(),
        })
    }

    fn save(&self) -> Vec<u8> {
        let n = &self.net;
        let mut w = Writer::new(KIND, VERSION, 8 + (n.emb.len() + n.head.len()) * 4);
        w.u32(n.buckets);
        w.len(n.dense_len);
        w.f32s(&n.emb);
        w.f32s(&n.head);
        w.finish()
    }

    fn load(&mut self, bytes: &[u8]) -> Result<(), PolicyError> {
        let mut r = Reader::open(bytes, KIND, VERSION)?;
        r.expect_dim(self.net.buckets, "sparse buckets")?;
        r.expect_dim(sat_u32(self.net.dense_len), "dense len")?;
        let emb = r.f32s(self.net.emb.len())?;
        let head = r.f32s(self.net.head.len())?;
        r.finish()?;
        self.net.emb = emb;
        self.net.head = head;
        self.emb_moments = Moments::zeros(self.net.emb.len());
        self.head_moments = Moments::zeros(self.net.head.len());
        self.beta1_pow = 1.0;
        self.beta2_pow = 1.0;
        Ok(())
    }
}
