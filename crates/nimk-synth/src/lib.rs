//! Synthetic mixed Vietnamese/English typing data for pre-training and
//! benchmarking, with no app context (every session uses one synthetic app).
//!
//! Pipeline: [`Corpus`] -> sentences that code-switch the way Vietnamese
//! users type (Vietnamese prose with English technical terms, English
//! phrases with Vietnamese words) -> per-word Telex key streams
//! ([`nimk_telex::encode`]) -> [`Script`] of `KeyEvent`s plus the intended
//! language of every word.
//!
//! Every key also carries its target [`nimk_types::Action`] (`Script::targets`,
//! ADR 0007): `Append` for word letters, `Pop` for Backspace after a typo,
//! `Commit` for the key ending a word, `Pass` for boundary keys with an empty
//! buffer and Cmd shortcuts; render is the word's (or previous word's)
//! language. These labels pre-train the edit head by behaviour cloning.
//!
//! A script can be written as an ordinary event log (so `nimk-replay` reads
//! it like real data) or fed straight into an engine to pre-train a model.
#![forbid(unsafe_code)]

mod corpus;
mod eval_report;
mod generate;
mod keymap;
mod log;
mod pretrain;

pub use corpus::Corpus;
pub use eval_report::{EvalReport, POS_BUCKETS};
pub use generate::{MixProfile, Script, ScriptWord, SynthConfig, generate};
pub use log::write_log;
pub use pretrain::{PretrainOptions, PretrainReport, pretrain};

/// Bundle id used for every synthetic `Context` event.
pub const SYNTHETIC_BUNDLE_ID: &str = "synthetic";
