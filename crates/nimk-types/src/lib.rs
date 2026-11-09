//! Shared domain types for NeuralIMK.
//!
//! This crate is the bottom layer: every other crate may depend on it, and it
//! depends on no other `nimk-*` crate. Types here are the contract between the
//! composer, the policy, storage and the replay harness.
#![forbid(unsafe_code)]

mod action;
mod ctx;
mod event;
mod key;
mod lang;
mod stat;
mod transform;
mod truth;

pub use action::{
    Action, ActionMask, ActionScores, EditAction, Example, Observation, RECENT_WINDOW,
};
pub use ctx::{AppContext, CaretRel};
pub use event::{
    Boundary, DecisionRule, Event, LabelSource, Record, SCHEMA_VERSION, SwitchScope, Target,
};
pub use key::{KeyEvent, KeyKind, Modifiers, keycode};
pub use lang::Lang;
pub use stat::StatCategory;
pub use transform::Transform;
pub use truth::{ground_truth, word_correct};
