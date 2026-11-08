//! `GruAction`: char-level GRU action model on the burn `flex` CPU backend.
//!
//! Threading: burn is built without its `rayon` feature, so every kernel
//! runs on the calling thread (hot path: the IME thread for `score`; the
//! trainer thread for `update`). No burn thread pool is created.

use std::sync::Arc;

use burn::module::Module;
use burn::optim::{AdamConfig, GradientsParams, ModuleOptimizer, SgdConfig};
use burn::store::ModuleRecord;
use burn::tensor::activation::{log_softmax, sigmoid, softmax};
use burn::tensor::{Bytes, Device, Int, Tensor, TensorData};
use nimk_types::{ActionScores, Example, Observation};

use crate::PolicyError;
use crate::action_model::{ImeLearner, ImeModel};
use crate::codec::{Reader, Writer, sat_u32};
use crate::encode::{BUNDLE_BUCKETS, CARET_CELLS, Encoded, SCALARS, SEQ_LEN, tok};
use crate::gru_net::{Batch, HIDDEN, Net};

const ID: &str = "gru_action@v1";
/// Snapshot kind string.
pub(crate) const KIND: &str = "gru_action";
const VERSION: u32 = 1;
/// Default Adam step size.
pub const GRU_LR: f64 = 3e-3;
/// Plain SGD step size for render-only (online) batches. Adam was wrong
/// for these: it moves every head weight by about `lr` per step whatever
/// the gradient, so even `3e-5` drifted the head (replay 96.9% vs 98.0%
/// frozen); SGD steps shrink as the head gets the word right.
pub const GRU_ONLINE_LR: f64 = 1e-3;

/// Scores returned when the backend fails to read a result back (never
/// observed; keeps `score` panic-free): uniform edits, `p_vi = 0.5`.
const NEUTRAL: ActionScores = ActionScores {
    edit: [0.25; 4],
    p_vi: 0.5,
};

/// Read-only snapshot: a non-autodiff copy of the weights.
struct GruModel {
    net: Net,
    device: Device,
}

impl ImeModel for GruModel {
    fn id(&self) -> &str {
        ID
    }

    fn score(&self, obs: &Observation) -> ActionScores {
        let e = Encoded::new(obs);
        let (edit, render) = self.net.forward(Batch::new([e].iter(), &self.device));
        let edit = softmax(edit, 1).into_data().try_into_vec::<f32>();
        let p_vi = sigmoid(render).into_data().try_into_vec::<f32>();
        match (edit, p_vi) {
            (Ok(edit), Ok(p)) if edit.len() == 4 && p.len() == 1 => ActionScores {
                edit: [edit[0], edit[1], edit[2], edit[3]],
                p_vi: p[0],
            },
            _ => NEUTRAL,
        }
    }
}

/// Char GRU over the token window + app context embeddings, two heads (see
/// `gru_net` for the diagram). About 35k parameters.
///
/// Loss per batch: `sum(edit_weight * CE(edit) + render_weight * BCE(render)) / len`.
///
/// Render-only examples (`edit_weight == 0`, i.e. online labels, ADR 0007)
/// train the render head alone: their gradient does not reach the shared
/// trunk (embeddings, GRU, hidden layer), so online learning can never
/// change the edit head's decisions. Without this, render-only updates
/// drifted the trunk until the model stopped committing words (replay:
/// 5746 of 10000 words committed). Render-only batches are stepped with
/// plain SGD at [`GRU_ONLINE_LR`] instead of Adam.
/// The edit head is trained on **unmasked** logits; at inference the engine
/// drops actions its [`nimk_types::ActionMask`] forbids and renormalizes.
///
/// [`ImeLearner::save`] writes the weights only (burnpack record bytes after
/// the `NIMK` header and the model dimensions); Adam state is dropped, so a
/// loaded model restarts its optimizer.
pub struct GruAction {
    net: Net,
    optim: ModuleOptimizer,
    /// Plain SGD for render-only (online) batches.
    online_optim: ModuleOptimizer,
    /// Autodiff device used for training.
    device: Device,
    lr: f64,
    /// splitmix64 state for context dropout (deterministic per seed).
    rng: u64,
}

impl std::fmt::Debug for GruAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GruAction")
            .field("params", &self.num_params())
            .field("lr", &self.lr)
            .finish()
    }
}

impl GruAction {
    /// Creates a neutral model with deterministic weights for `seed` and the
    /// default step size [`GRU_LR`].
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self::with_lr(seed, GRU_LR)
    }

    /// Creates a neutral model with an explicit Adam step size.
    #[must_use]
    pub fn with_lr(seed: u64, lr: f64) -> Self {
        let device = Device::flex().autodiff();
        Self {
            net: Net::init(seed, &device),
            optim: fresh_optimizer(),
            online_optim: SgdConfig::new().init(),
            device,
            lr,
            rng: seed ^ 0xd1b5_4a32_d192_ed03,
        }
    }

    /// Number of trainable scalars.
    #[must_use]
    pub fn num_params(&self) -> usize {
        self.net.num_params()
    }
}

fn fresh_optimizer() -> ModuleOptimizer {
    AdamConfig::new().init()
}

/// Weighted batch loss `[1]` (autodiff graph attached).
fn batch_loss(net: &Net, encoded: &[Encoded], batch: &[Example], device: &Device) -> Tensor<1> {
    let b = batch.len();
    let mut targets = Vec::with_capacity(b);
    let mut y = Vec::with_capacity(b);
    let mut we = Vec::with_capacity(b);
    let mut wr = Vec::with_capacity(b);
    let mut trunk = Vec::with_capacity(b);
    for ex in batch {
        targets.push(ex.target.edit.index() as i64);
        y.push(ex.target.render.as_target());
        we.push(ex.edit_weight);
        wr.push(ex.render_weight);
        trunk.push(if ex.edit_weight > 0.0 { 1.0f32 } else { 0.0 });
    }
    let trunk = Tensor::<2>::from_data(TensorData::new(trunk, [b, 1]), device);
    let (edit, render) = net.forward_train(Batch::new(encoded.iter(), device), trunk);
    let f1 = |v: Vec<f32>| Tensor::<1>::from_data(TensorData::new(v, [b]), device);
    let targets = Tensor::<2, Int>::from_data(TensorData::new(targets, [b, 1]), device);
    let ce = log_softmax(edit, 1).gather(1, targets).reshape([b]).neg();
    // Stable BCE with logits: max(x, 0) - x*y + ln(1 + e^-|x|).
    let x = render.reshape([b]);
    let bce =
        x.clone().clamp_min(0.0) - x.clone() * f1(y) + x.abs().neg().exp().add_scalar(1.0).log();
    (ce * f1(we) + bce * f1(wr)).sum().div_scalar(b as f32)
}

impl ImeLearner for GruAction {
    fn id(&self) -> &str {
        ID
    }

    fn update(&mut self, batch: &[Example]) -> f32 {
        if batch.is_empty() {
            return 0.0;
        }
        let mut encoded: Vec<Encoded> = batch.iter().map(|ex| Encoded::new(&ex.obs)).collect();
        for e in &mut encoded {
            context_dropout(e, &mut self.rng);
        }
        let loss = batch_loss(&self.net, &encoded, batch, &self.device);
        let value = loss.clone().into_scalar::<f32>();
        if !value.is_finite() {
            return value;
        }
        let grads = GradientsParams::from_grads(loss.backward(), &self.net);
        if batch.iter().all(|ex| ex.edit_weight <= 0.0) {
            // Render-only (online) batch: only the render head has gradients.
            self.net = self
                .online_optim
                .step(GRU_ONLINE_LR, self.net.clone(), grads);
        } else {
            self.net = self.optim.step(self.lr, self.net.clone(), grads);
        }
        value
    }

    /// `false`: the render head sits on shared dense features, so a label
    /// that merely confirms a commit acts like a recency bias toward the
    /// last word's language (replay 96.4% online vs 98.0% frozen when
    /// learning from every label; 97.5% learning from corrections only).
    fn learns_from_confirmations(&self) -> bool {
        false
    }

    fn snapshot(&self) -> Arc<dyn ImeModel> {
        Arc::new(GruModel {
            net: self.net.valid(),
            device: Device::flex(),
        })
    }

    fn save(&self) -> Vec<u8> {
        // `save` cannot fail by contract. If burn cannot encode the record
        // (not expected: in-memory write), an empty payload is written, which
        // `load` rejects as corrupt instead of restoring garbage.
        let record;
        let payload: &[u8] = match self.net.valid().into_record().into_bytes() {
            Ok(bytes) => {
                record = bytes;
                &record
            }
            Err(_) => &[],
        };
        let mut w = Writer::new(KIND, VERSION, 32 + payload.len());
        for d in dims() {
            w.u32(d);
        }
        w.len(payload.len());
        w.bytes(payload);
        w.finish()
    }

    fn load(&mut self, bytes: &[u8]) -> Result<(), PolicyError> {
        let mut r = Reader::open(bytes, KIND, VERSION)?;
        for (d, what) in dims().into_iter().zip(DIM_NAMES) {
            r.expect_dim(d, what)?;
        }
        let n = r.u32()? as usize;
        let payload = r.bytes(n)?;
        r.finish()?;
        let record = ModuleRecord::from_bytes(Bytes::from_bytes_vec(payload.to_vec()))
            .map_err(|e| PolicyError::Corrupt(format!("gru record: {e}")))?;
        let net = self
            .net
            .clone()
            .try_load_record(record)
            .map_err(|e| PolicyError::Corrupt(format!("gru record: {e}")))?;
        self.net = net.fork(&self.device);
        self.optim = fresh_optimizer();
        Ok(())
    }
}

/// splitmix64 step.
fn next_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Training-time regularizer: with probability 1/4 hide every token but the
/// incoming key, otherwise replace each context token with `OTHER` with
/// probability 1/10. Forces the heads to use the scalar features (Telex
/// flags, mask) instead of memorizing whole words; without it the GRU
/// reached only ~87% held-out render accuracy on the toy task (unseen
/// words), although it scores ~0.5 pt higher on in-vocabulary synthetic data.
fn context_dropout(e: &mut Encoded, rng: &mut u64) {
    let first = SEQ_LEN - e.len;
    if next_u64(rng).is_multiple_of(4) {
        e.tokens[..SEQ_LEN - 1].fill(tok::PAD);
        e.len = 1;
    } else {
        for t in &mut e.tokens[first..SEQ_LEN - 1] {
            if next_u64(rng).is_multiple_of(10) {
                *t = tok::OTHER;
            }
        }
    }
}

const DIM_NAMES: [&str; 6] = [
    "vocab",
    "seq len",
    "scalars",
    "hidden",
    "bundle buckets",
    "caret cells",
];

/// Model dimensions written before the record and checked on load.
fn dims() -> [u32; 6] {
    [
        sat_u32(tok::VOCAB),
        sat_u32(SEQ_LEN),
        sat_u32(SCALARS),
        sat_u32(HIDDEN),
        sat_u32(BUNDLE_BUCKETS),
        sat_u32(CARET_CELLS),
    ]
}
