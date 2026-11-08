//! Burn network behind `GruAction`: architecture, deterministic init and
//! batch tensor construction.
//!
//! ```text
//! tokens [B,T] -> Embedding(139, 32) -> GRU(64) -> last hidden [B,64] --+
//! scalars [B,18] -------------------------------------------------------+
//! bundle  [B,1] -> Embedding(256, 16) ----------------------------------+-> concat [B,106]
//! caret   [B,1] -> Embedding(65, 8) ------------------------------------+
//!   -> Linear(106, 64) + ReLU -> edit: Linear(64, 4)   (logits)
//!                             -> render: Linear(64, 1) (logit of p_vi)
//! ```

use burn::module::{Module, ModuleMapper, Param};
use burn::nn::gru::{Gru, GruConfig};
use burn::nn::{Embedding, EmbeddingConfig, Initializer, Linear, LinearConfig};
use burn::tensor::activation::relu;
use burn::tensor::{Device, Int, Tensor, TensorData};

use crate::encode::{BUNDLE_BUCKETS, CARET_CELLS, Encoded, SCALARS, SEQ_LEN, tok};

/// Token embedding width.
pub(crate) const TOK_DIM: usize = 32;
/// GRU hidden size.
pub(crate) const HIDDEN: usize = 64;
/// Bundle embedding width.
pub(crate) const BUNDLE_DIM: usize = 16;
/// Caret embedding width.
pub(crate) const CARET_DIM: usize = 8;
/// Width of the shared hidden layer.
pub(crate) const MLP: usize = 64;
const CONCAT: usize = HIDDEN + SCALARS + BUNDLE_DIM + CARET_DIM;

/// The action network. Field names are the record paths (part of the
/// snapshot format: renaming a field breaks old snapshots).
#[derive(Module, Debug)]
pub(crate) struct Net {
    tok: Embedding,
    gru: Gru,
    bundle: Embedding,
    caret: Embedding,
    hidden: Linear,
    edit: Linear,
    render: Linear,
}

/// Input tensors for a batch.
pub(crate) struct Batch {
    tokens: Tensor<2, Int>,
    scalars: Tensor<2>,
    bundle: Tensor<2, Int>,
    caret: Tensor<2, Int>,
}

impl Batch {
    /// Builds tensors for `encoded` on `device` (one allocation per input).
    pub(crate) fn new<'a>(
        encoded: impl ExactSizeIterator<Item = &'a Encoded>,
        device: &Device,
    ) -> Self {
        let b = encoded.len();
        let mut tokens = Vec::with_capacity(b * SEQ_LEN);
        let mut scalars = Vec::with_capacity(b * SCALARS);
        let mut bundle = Vec::with_capacity(b);
        let mut caret = Vec::with_capacity(b);
        for e in encoded {
            tokens.extend(e.tokens.iter().map(|&t| i64::from(t)));
            scalars.extend_from_slice(&e.scalars);
            bundle.push(i64::from(e.bundle));
            caret.push(i64::from(e.caret));
        }
        Self {
            tokens: Tensor::from_data(TensorData::new(tokens, [b, SEQ_LEN]), device),
            scalars: Tensor::from_data(TensorData::new(scalars, [b, SCALARS]), device),
            bundle: Tensor::from_data(TensorData::new(bundle, [b, 1]), device),
            caret: Tensor::from_data(TensorData::new(caret, [b, 1]), device),
        }
    }
}

impl Net {
    /// Builds the network with deterministic weights derived from `seed`
    /// (burn's device RNG is process-global, so it is not used). The two
    /// output heads start at zero: untrained `p_vi = 0.5`, uniform edits.
    pub(crate) fn init(seed: u64, device: &Device) -> Self {
        let zeros = Initializer::Zeros;
        let net = Self {
            tok: EmbeddingConfig::new(tok::VOCAB, TOK_DIM)
                .with_initializer(zeros.clone())
                .init(device),
            gru: GruConfig::new(TOK_DIM, HIDDEN, true)
                .with_initializer(zeros.clone())
                .init(device),
            bundle: EmbeddingConfig::new(BUNDLE_BUCKETS, BUNDLE_DIM)
                .with_initializer(zeros.clone())
                .init(device),
            caret: EmbeddingConfig::new(CARET_CELLS, CARET_DIM)
                .with_initializer(zeros.clone())
                .init(device),
            hidden: LinearConfig::new(CONCAT, MLP)
                .with_initializer(zeros.clone())
                .init(device),
            edit: LinearConfig::new(MLP, 4)
                .with_initializer(zeros.clone())
                .init(device),
            render: LinearConfig::new(MLP, 1)
                .with_initializer(zeros)
                .init(device),
        };
        net.map(&mut SeededInit::new(seed))
    }

    /// Returns `(edit logits [B,4], render logit [B,1])`.
    pub(crate) fn forward(&self, x: Batch) -> (Tensor<2>, Tensor<2>) {
        let z = self.trunk(x);
        (self.edit.forward(z.clone()), self.render.forward(z))
    }

    /// Training forward: like [`Net::forward`], but the render loss of row
    /// `i` back-propagates into the shared trunk only where
    /// `trunk_mask[i] == 1` (`[B,1]`, 0/1); elsewhere only the render head
    /// learns. Values are identical to [`Net::forward`].
    pub(crate) fn forward_train(&self, x: Batch, trunk_mask: Tensor<2>) -> (Tensor<2>, Tensor<2>) {
        let z = self.trunk(x);
        let frozen = z.clone().detach();
        let z_render = frozen.clone() + (z.clone() - frozen) * trunk_mask;
        (self.edit.forward(z), self.render.forward(z_render))
    }

    /// Shared representation `[B, MLP]` feeding both heads.
    fn trunk(&self, x: Batch) -> Tensor<2> {
        let [b, t] = x.tokens.dims();
        let emb = self.tok.forward(x.tokens);
        let states = self.gru.forward(emb, None);
        let last = states.slice([0..b, t - 1..t]).reshape([b, HIDDEN]);
        let bundle = self.bundle.forward(x.bundle).reshape([b, BUNDLE_DIM]);
        let caret = self.caret.forward(x.caret).reshape([b, CARET_DIM]);
        let z = Tensor::cat(vec![last, x.scalars, bundle, caret], 1);
        relu(self.hidden.forward(z))
    }
}

/// Replaces every float parameter with deterministic values: embeddings
/// `U(-0.5, 0.5)`, matrices Xavier-uniform, biases and the output heads 0.
struct SeededInit {
    state: u64,
    path: Vec<String>,
}

impl SeededInit {
    fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9e37_79b9_7f4a_7c15,
            path: Vec::new(),
        }
    }

    /// splitmix64 -> uniform in `[-1, 1)`.
    fn next_unit(&mut self) -> f32 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        ((z >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }

    fn in_module(&self, names: &[&str]) -> bool {
        self.path.iter().any(|p| names.contains(&p.as_str()))
    }
}

impl ModuleMapper for SeededInit {
    fn enter_module(&mut self, name: &str, _container_type: &str) {
        self.path.push(name.to_owned());
    }

    fn exit_module(&mut self, _name: &str, _container_type: &str) {
        self.path.pop();
    }

    fn map_float<const D: usize>(&mut self, param: Param<Tensor<D>>) -> Param<Tensor<D>> {
        let head = self.in_module(&["edit", "render"]);
        let embedding = self.in_module(&["tok", "bundle", "caret"]);
        param.map(|t| {
            let shape = t.shape();
            let dims = shape.dims::<D>();
            let n: usize = dims.iter().product();
            let scale = if head || D == 1 {
                0.0
            } else if embedding {
                0.5
            } else {
                let fan: usize = dims.iter().sum();
                (6.0 / fan.max(1) as f32).sqrt()
            };
            let values: Vec<f32> = (0..n).map(|_| self.next_unit() * scale).collect();
            let fresh = Tensor::<D>::from_data(TensorData::new(values, dims), &t.device());
            fresh.set_require_grad(t.is_require_grad())
        })
    }
}
