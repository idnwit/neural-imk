//! Both action models learn the toy typing task: edits from structure,
//! render from the word's language, generalizing to held-out words.
#![allow(clippy::print_stderr)]

mod common;

use common::{Eval, dataset, evaluate, mean, split_words, train};
use nimk_policy::{ImeLearner, ModelKind, build_action};

fn run(kind: ModelKind, epochs: usize) -> (Eval, Eval, f32, f32) {
    let (train_words, test_words) = split_words();
    let train_set = dataset(&train_words, 120, 1);
    let test_set = dataset(&test_words, 40, 2);
    let mut learner: Box<dyn ImeLearner> = build_action(kind, 7);
    let losses = train(learner.as_mut(), &train_set, epochs, 32, 3);
    let snap = learner.snapshot();
    let k = 20.min(losses.len() / 2);
    let (first, last) = (mean(&losses[..k]), mean(&losses[losses.len() - k..]));
    let (tr, te) = (
        evaluate(snap.as_ref(), &train_set),
        evaluate(snap.as_ref(), &test_set),
    );
    eprintln!(
        "{kind:?}: {} train ex, {} batches, loss {first:.3} -> {last:.3}\n  train {tr:?}\n  held-out {te:?}",
        train_set.len(),
        losses.len()
    );
    (tr, te, first, last)
}

fn check(kind: ModelKind, epochs: usize) {
    let (_, te, first, last) = run(kind, epochs);
    assert!(
        last < first * 0.5,
        "loss did not decrease: {first} -> {last}"
    );
    assert!(te.edit >= 0.98, "held-out edit accuracy {}", te.edit);
    assert!(
        te.render_commit >= 0.90,
        "held-out render accuracy {}",
        te.render_commit
    );
}

#[test]
fn linear_action_learns_toy_task() {
    check(ModelKind::LinearAction, 8);
}

/// Slow without optimizations: runs under `cargo test --release`, ignored
/// in debug builds (run with `--include-ignored` to force).
#[test]
#[cfg_attr(debug_assertions, ignore)]
fn gru_action_learns_toy_task() {
    check(ModelKind::GruAction, 6);
}
