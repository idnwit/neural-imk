//! Shared domain types for NeuralIMK.
//!
//! This crate is the bottom layer: every other crate may depend on it, and it
//! depends on no other `nimk-*` crate. Types here are the contract between the
//! composer, the policy, storage and the replay harness.
#![forbid(unsafe_code)]

mod ctx;
mod event;
mod features;
mod key;
mod lang;

pub use ctx::{AppContext, CaretRel, DecisionCtx, Sample};
pub use event::{
    Boundary, DecisionRule, Event, LabelSource, Record, SCHEMA_VERSION, SwitchScope, Target,
};
pub use features::Features;
pub use key::{KeyEvent, KeyKind, Modifiers, keycode};
pub use lang::Lang;
