//! Synthetic mixed Vietnamese/English typing data for pre-training and
//! benchmarking, with no app context (every session uses one synthetic app).
//!
//! Pipeline: [`Corpus`] -> sentences that code-switch the way Vietnamese
//! users type (Vietnamese prose with English technical terms, English
//! phrases with Vietnamese words) -> per-word Telex key streams
//! ([`nimk_telex::encode`]) -> [`Script`] of `KeyEvent`s plus the intended
//! language of every word.
//!
//! A script can be written as an ordinary event log (so `nimk-replay` reads
//! it like real data) or fed straight into an engine to pre-train a model.
#![forbid(unsafe_code)]

mod corpus;
mod generate;
mod keymap;
mod log;
mod pretrain;

pub use corpus::Corpus;
pub use generate::{MixProfile, Script, ScriptWord, SynthConfig, generate};
pub use log::write_log;
pub use pretrain::{PretrainReport, pretrain};

/// Bundle id used for every synthetic `Context` event.
pub const SYNTHETIC_BUNDLE_ID: &str = "synthetic";
