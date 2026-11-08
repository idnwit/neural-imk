use std::sync::Arc;

use nimk_types::{Features, Sample};

use crate::codec::{Reader, Writer, log_loss, sat_u32, sigmoid, weighted_mean};
use crate::{Learner, Policy, PolicyError};

const ID: &str = "linear@v1";
const KIND: &str = "linear";
const VERSION: u32 = 1;

/// FTRL-Proximal hyper-parameters (McMahan et al. 2013, "Ad Click Prediction").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FtrlParams {
    /// Per-coordinate learning-rate scale.
    pub alpha: f32,
    /// Learning-rate smoothing term.
    pub beta: f32,
    /// L1 regularization (drives rare weights to exactly 0).
    pub l1: f32,
    /// L2 regularization.
    pub l2: f32,
}

impl Default for FtrlParams {
    /// `alpha = 0.5, beta = 1.0, l1 = 0.01, l2 = 0.01`: a fast-adapting
    /// setting for a few hundred labels per day with light regularization.
    fn default() -> Self {
        Self {
            alpha: 0.5,
            beta: 1.0,
            l1: 0.01,
            l2: 0.01,
        }
    }
}

/// Coordinate layout shared by learner and snapshot:
/// `[0, buckets)` sparse ids, `[buckets, buckets + dense_len)` dense, last = bias.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layout {
    buckets: u32,
    dense_len: usize,
}

impl Layout {
    fn len(self) -> usize {
        self.buckets as usize + self.dense_len + 1
    }

    fn bias(self) -> usize {
        self.len() - 1
    }

    /// Calls `f(coord, x)` for every non-zero input, including the bias.
    /// Sparse ids are count features (x = 1 per occurrence); out-of-range ids
    /// wrap modulo `buckets`, extra dense values are ignored.
    #[inline]
    fn for_each(self, feats: &Features, mut f: impl FnMut(usize, f32)) {
        let buckets = self.buckets.max(1);
        for &id in &feats.sparse {
            f((id % buckets) as usize, 1.0);
        }
        for (i, &x) in feats.dense.iter().take(self.dense_len).enumerate() {
            if x != 0.0 {
                f(self.buckets as usize + i, x);
            }
        }
        f(self.bias(), 1.0);
    }
}

/// FTRL-Proximal logistic regression over hashed sparse + dense features.
///
/// Sparse ids are binary (count) inputs, dense values are real inputs, plus a
/// bias. Each sample's gradient is scaled by its weight. Snapshot payload:
/// `buckets: u32, dense_len: u32`, then `z` and `n` (`f32 x len` each);
/// hyper-parameters are not stored.
#[derive(Debug, Clone)]
pub struct Linear {
    layout: Layout,
    params: FtrlParams,
    z: Vec<f32>,
    n: Vec<f32>,
}

impl Linear {
    /// Creates an untrained model with [`FtrlParams::default`].
    #[must_use]
    pub fn new(sparse_buckets: u32, dense_len: usize) -> Self {
        Self::with_params(sparse_buckets, dense_len, FtrlParams::default())
    }

    /// Creates an untrained model with explicit hyper-parameters.
    #[must_use]
    pub fn with_params(sparse_buckets: u32, dense_len: usize, params: FtrlParams) -> Self {
        let layout = Layout {
            buckets: sparse_buckets.max(1),
            dense_len,
        };
        Self {
            layout,
            params,
            z: vec![0.0; layout.len()],
            n: vec![0.0; layout.len()],
        }
    }

    /// Closed-form FTRL weight of coordinate `i`.
    #[inline]
    fn weight(&self, i: usize) -> f32 {
        let (Some(&z), Some(&n)) = (self.z.get(i), self.n.get(i)) else {
            return 0.0;
        };
        let p = self.params;
        if z.abs() <= p.l1 {
            0.0
        } else {
            -(z - z.signum() * p.l1) / ((p.beta + n.sqrt()) / p.alpha + p.l2)
        }
    }

    fn p_vi(&self, f: &Features) -> f32 {
        let mut logit = 0.0;
        self.layout.for_each(f, |i, x| logit += self.weight(i) * x);
        sigmoid(logit)
    }

    fn step(&mut self, s: &Sample) {
        let g0 = (self.p_vi(&s.features) - s.label.as_target()) * s.weight;
        if g0 == 0.0 || !g0.is_finite() {
            return;
        }
        let layout = self.layout;
        layout.for_each(&s.features, |i, x| {
            let w = self.weight(i);
            let (Some(z), Some(n)) = (self.z.get(i).copied(), self.n.get(i).copied()) else {
                return;
            };
            let g = g0 * x;
            let n_new = n + g * g;
            let sigma = (n_new.sqrt() - n.sqrt()) / self.params.alpha;
            if let Some(zr) = self.z.get_mut(i) {
                *zr = z + g - sigma * w;
            }
            if let Some(nr) = self.n.get_mut(i) {
                *nr = n_new;
            }
        });
    }
}

/// Immutable snapshot: materialized weights.
#[derive(Debug)]
struct LinearPolicy {
    layout: Layout,
    w: Vec<f32>,
}

impl Policy for LinearPolicy {
    fn id(&self) -> &str {
        ID
    }

    fn predict(&self, features: &Features) -> f32 {
        let mut logit = 0.0;
        self.layout.for_each(features, |i, x| {
            if let Some(w) = self.w.get(i) {
                logit += w * x;
            }
        });
        sigmoid(logit)
    }
}

impl Learner for Linear {
    fn id(&self) -> &str {
        ID
    }

    fn update(&mut self, batch: &[Sample]) -> f32 {
        let (mut num, mut den) = (0.0, 0.0);
        for s in batch {
            num += s.weight * log_loss(self.p_vi(&s.features), s.label.as_target());
            den += s.weight;
        }
        for s in batch {
            self.step(s);
        }
        weighted_mean(num, den)
    }

    fn snapshot(&self) -> Arc<dyn Policy> {
        let w = (0..self.layout.len()).map(|i| self.weight(i)).collect();
        Arc::new(LinearPolicy {
            layout: self.layout,
            w,
        })
    }

    fn save(&self) -> Vec<u8> {
        let mut w = Writer::new(KIND, VERSION, 8 + self.layout.len() * 8);
        w.u32(self.layout.buckets);
        w.len(self.layout.dense_len);
        w.f32s(&self.z);
        w.f32s(&self.n);
        w.finish()
    }

    fn load(&mut self, bytes: &[u8]) -> Result<(), PolicyError> {
        let mut r = Reader::open(bytes, KIND, VERSION)?;
        r.expect_dim(self.layout.buckets, "sparse buckets")?;
        r.expect_dim(sat_u32(self.layout.dense_len), "dense len")?;
        let z = r.f32s(self.layout.len())?;
        let n = r.f32s(self.layout.len())?;
        r.finish()?;
        if n.iter().any(|&v| v < 0.0) {
            return Err(PolicyError::Corrupt("negative FTRL accumulator".to_owned()));
        }
        self.z = z;
        self.n = n;
        Ok(())
    }
}
