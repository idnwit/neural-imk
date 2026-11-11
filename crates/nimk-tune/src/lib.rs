//! Teaching a model from typing scripts, and fine-tuning it on the user's
//! own log.
//!
//! A [`Script`] is a run of keys with the action the model should take at
//! every key. They come from synthetic data (`nimk-synth`) or from a real
//! event log ([`scripts_from_log`]). [`finetune`] teaches scripts to the
//! snapshot already in a store; [`tune`] does that for the data typed since
//! the last run and leaves the result for the engine to install.
//!
//! Sits between `nimk-engine` (which it drives) and the crates above it:
//! the C ABI (`nimk-ffi`) for the menu button, and `nimk-synth` for
//! pre-training and benchmarks.
#![forbid(unsafe_code)]

mod from_log;
mod script;
mod teach;
pub mod track;
mod tune;

pub use from_log::{LogScripts, ScriptMeta, scripts_from_log};
pub use script::{Script, ScriptWord};
pub use teach::{
    ModelOnly, TeachOptions, check_diagnostics, finetune, snapshot_key, synthetic_context,
    teach_config, teach_script,
};
pub use tune::{
    STATE_KEY as TUNE_STATE_KEY, TuneOptions, TuneOutput, TunePending, TuneReport,
    pending as tune_pending, result_key as tune_result_key, tune,
};
