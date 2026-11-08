//! The committed rendering is the model's decision at the commit step;
//! hysteresis only steadies the marked text mid-word.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use nimk_engine::EngineConfig;
use nimk_types::Observation;

fn cfg() -> EngineConfig {
    EngineConfig {
        switch_margin: 0.2,
        ..inline_cfg()
    }
}

fn is_space(o: &Observation) -> bool {
    o.key.as_ref().is_some_and(|k| k.chars == " ")
}

#[test]
fn commit_uses_the_fresh_decision_not_the_held_display() {
    // Confident Vi while typing, 0.35 at the commit key: within the margin,
    // so hysteresis would keep Vi, but the commit follows the model.
    let script = typist(|o| if is_space(o) { 0.35 } else { 0.9 });
    let (mut e, _s, _) = scripted_engine(script, cfg());
    let mut t = 0.0;
    let outs = type_str(&mut e, "as ", &mut t);
    assert_eq!(outs[1].marked.as_deref(), Some("\u{e1}"), "display Vi");
    assert_eq!(outs[2].marked.as_deref(), Some("as "), "pending En");
    assert_eq!(e.commit_pending(t + 1.0).commit.as_deref(), Some("as "));
}

#[test]
fn held_display_does_not_flip_mid_word() {
    // 0.35 mid-word stays Vi (hysteresis), and 0.65 at commit commits Vi.
    let script = typist(|o| match o.raw.as_str() {
        "" => 0.9,
        _ if is_space(o) => 0.65,
        _ => 0.35,
    });
    let (mut e, _s, _) = scripted_engine(script, cfg());
    let mut t = 0.0;
    let outs = type_str(&mut e, "as ", &mut t);
    assert_eq!(outs[1].marked.as_deref(), Some("\u{e1}"), "0.35 held at Vi");
    assert_eq!(outs[2].marked.as_deref(), Some("\u{e1} "));
}

#[test]
fn manual_lock_beats_the_fresh_decision() {
    let script = typist(|o| if is_space(o) { 0.05 } else { 0.9 });
    let (mut e, _s, _) = scripted_engine(script, cfg());
    let mut t = 0.0;
    type_str(&mut e, "as", &mut t);
    // Double Shift mid-word: locked to En, then flip back to Vi.
    double_shift(&mut e, &mut t);
    double_shift(&mut e, &mut t);
    let outs = type_str(&mut e, " ", &mut t);
    assert_eq!(outs[0].marked.as_deref(), Some("\u{e1} "), "lock wins");
}
