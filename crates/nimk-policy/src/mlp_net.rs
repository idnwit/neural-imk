//! Parameters and allocation-free forward pass of [`crate::TinyMlp`].

use nimk_types::Features;

use crate::Policy;
use crate::codec::sigmoid;

/// Embedding dimension.
pub(crate) const EMB: usize = 16;
/// Hidden units.
pub(crate) const HIDDEN: usize = 64;

/// SplitMix64 PRNG (Steele et al. 2014); deterministic init without `rand`.
pub(crate) struct SplitMix64(u64);

impl SplitMix64 {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `[-a, a)`.
    pub(crate) fn uniform(&mut self, a: f32) -> f32 {
        let u = (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32;
        (2.0 * u - 1.0) * a
    }
}

/// Network weights.
///
/// `head` is flat: `w1` (`HIDDEN x input`, row per hidden unit, embedding
/// columns first), `b1` (`HIDDEN`), `w2` (`HIDDEN`), `b2` (1).
/// Invariant: `emb.len() == buckets * EMB` and `head.len() == head_len(dense_len)`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Net {
    pub(crate) buckets: u32,
    pub(crate) dense_len: usize,
    pub(crate) emb: Vec<f32>,
    pub(crate) head: Vec<f32>,
}

/// Borrowed view of the head tensors.
pub(crate) struct Head<'a> {
    pub(crate) w1: &'a [f32],
    pub(crate) b1: &'a [f32],
    pub(crate) w2: &'a [f32],
    pub(crate) b2: f32,
}

/// Activations kept for the backward pass; lives on the stack.
pub(crate) struct Acts {
    /// Mean of the embedding rows of all sparse ids.
    pub(crate) bag: [f32; EMB],
    /// Number of sparse ids pooled into `bag`.
    pub(crate) count: usize,
    /// Post-ReLU hidden activations.
    pub(crate) h: [f32; HIDDEN],
}

impl Acts {
    pub(crate) fn new() -> Self {
        Self {
            bag: [0.0; EMB],
            count: 0,
            h: [0.0; HIDDEN],
        }
    }
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

impl Net {
    pub(crate) fn input_len(dense_len: usize) -> usize {
        EMB + dense_len
    }

    pub(crate) fn head_len(dense_len: usize) -> usize {
        HIDDEN * Self::input_len(dense_len) + 2 * HIDDEN + 1
    }

    /// Seeded init: embeddings U(+-0.1), `w1` He-uniform (breaks symmetry),
    /// `w2` = 0 and `b2` = 0 so an untrained net predicts exactly 0.5;
    /// biases 0.
    pub(crate) fn init(buckets: u32, dense_len: usize, seed: u64) -> Self {
        let mut rng = SplitMix64::new(seed);
        let emb = (0..buckets as usize * EMB)
            .map(|_| rng.uniform(0.1))
            .collect();
        let input = Self::input_len(dense_len);
        let a1 = (6.0 / input as f32).sqrt();
        let mut head = Vec::with_capacity(Self::head_len(dense_len));
        head.extend((0..HIDDEN * input).map(|_| rng.uniform(a1)));
        // b1 and w2 (then b2 below) start at zero.
        head.extend(std::iter::repeat_n(0.0, 2 * HIDDEN));
        head.push(0.0);
        Self {
            buckets,
            dense_len,
            emb,
            head,
        }
    }

    /// Splits `head` into its tensors (relies on the struct invariant).
    pub(crate) fn head(&self) -> Head<'_> {
        let (w1, rest) = self.head.split_at(HIDDEN * Self::input_len(self.dense_len));
        let (b1, rest) = rest.split_at(HIDDEN);
        let (w2, rest) = rest.split_at(HIDDEN);
        Head {
            w1,
            b1,
            w2,
            b2: match rest.first() {
                Some(&b2) => b2,
                None => 0.0,
            },
        }
    }

    /// Embedding row index for a sparse id (wraps modulo `buckets`).
    #[inline]
    pub(crate) fn row_of(&self, id: u32) -> u32 {
        id % self.buckets.max(1)
    }

    #[inline]
    pub(crate) fn emb_row(&self, row: u32) -> &[f32] {
        let start = row as usize * EMB;
        match self.emb.get(start..start + EMB) {
            Some(row) => row,
            None => &[],
        }
    }

    /// Forward pass; returns the logit and fills `acts`. Does not allocate.
    pub(crate) fn forward(&self, f: &Features, acts: &mut Acts) -> f32 {
        acts.bag = [0.0; EMB];
        acts.count = 0;
        for &id in &f.sparse {
            let row = self.emb_row(self.row_of(id));
            for (b, e) in acts.bag.iter_mut().zip(row) {
                *b += e;
            }
            acts.count += 1;
        }
        if acts.count > 1 {
            let inv = 1.0 / acts.count as f32;
            acts.bag.iter_mut().for_each(|b| *b *= inv);
        }
        let head = self.head();
        let input = Self::input_len(self.dense_len);
        let dense = &f.dense[..self.dense_len.min(f.dense.len())];
        for ((row, &b), h) in head
            .w1
            .chunks_exact(input)
            .zip(head.b1)
            .zip(acts.h.iter_mut())
        {
            let (we, wd) = row.split_at(EMB);
            *h = (b + dot(we, &acts.bag) + dot(wd, dense)).max(0.0);
        }
        head.b2 + dot(head.w2, &acts.h)
    }

    /// `p(VI)` for `f`. Does not allocate.
    pub(crate) fn p_vi(&self, f: &Features) -> f32 {
        let mut acts = Acts::new();
        sigmoid(self.forward(f, &mut acts))
    }
}

/// Immutable snapshot of a [`crate::TinyMlp`].
#[derive(Debug)]
pub(crate) struct MlpPolicy {
    pub(crate) id: &'static str,
    pub(crate) net: Net,
}

impl Policy for MlpPolicy {
    fn id(&self) -> &str {
        self.id
    }

    fn predict(&self, features: &Features) -> f32 {
        self.net.p_vi(features)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_is_deterministic_and_bounded() {
        let mut a = SplitMix64::new(1);
        let mut b = SplitMix64::new(1);
        for _ in 0..1000 {
            let x = a.uniform(0.5);
            assert_eq!(x, b.uniform(0.5));
            assert!((-0.5..0.5).contains(&x));
        }
    }

    #[test]
    fn init_respects_layout() {
        let net = Net::init(32, 8, 7);
        assert_eq!(net.emb.len(), 32 * EMB);
        assert_eq!(net.head.len(), Net::head_len(8));
        assert_eq!(net, Net::init(32, 8, 7));
        assert_ne!(net, Net::init(32, 8, 8));
        let p = net.p_vi(&Features::default());
        assert!(p > 0.0 && p < 1.0);
    }
}
