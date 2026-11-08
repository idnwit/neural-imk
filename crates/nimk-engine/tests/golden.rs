//! Golden behaviour of the engine with a deterministic constant policy.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use nimk_engine::{EngineConfig, Overlay};
use nimk_types::{Event, LabelSource, Lang, Modifiers, SwitchScope, Target, keycode};

fn labels(events: &[Event]) -> Vec<(Target, Lang, f32, LabelSource)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Label {
                target,
                label,
                weight,
                source,
            } => Some((*target, *label, *weight, *source)),
            _ => None,
        })
        .collect()
}

#[test]
fn vietnamese_words_commit_with_diacritics() {
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    let mut t = 100.0;
    let outs = type_str(&mut e, "tieengs Vieetj ", &mut t);
    assert_eq!(commits(&outs), vec!["tiếng", "Việt"]);
    let space = &outs[7];
    assert!(!space.consumed, "boundary goes to the app");
    assert_eq!(space.marked, None);
    assert_eq!(e.mode(), Lang::Vi);
    let evs = events(&storage);
    assert!(matches!(evs.first(), Some(Event::SessionStart { .. })));
    let words: Vec<_> = evs
        .iter()
        .filter_map(|e| match e {
            Event::WordCommit {
                word_id,
                raw,
                vi,
                mode,
                ..
            } => Some((*word_id, raw.clone(), vi.clone(), *mode)),
            _ => None,
        })
        .collect();
    assert_eq!(
        words,
        vec![
            (0, "tieengs".into(), "tiếng".into(), Lang::Vi),
            (1, "Vieetj".into(), "Việt".into(), Lang::Vi)
        ]
    );
    assert!(
        outs.iter().all(|o| o.overlay.is_none()),
        "starts in VI, never left it"
    );
}

#[test]
fn english_stays_english() {
    let (mut e, _s) = const_engine(0.1, inline_cfg());
    let mut t = 0.0;
    let outs = type_str(&mut e, "has string, test ", &mut t);
    assert_eq!(commits(&outs), vec!["has", "string", "test"]);
    assert_eq!(e.mode(), Lang::En);
}

#[test]
fn backspace_edits_and_passes_through_when_empty() {
    let (mut e, _s) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "as", &mut t);
    let bs = |t: f64| code_key(keycode::BACKSPACE, "\u{7f}", 0, t);
    let o = e.on_key(&bs(1.0));
    assert!(o.consumed);
    assert_eq!(o.marked.as_deref(), Some("a"));
    let o = e.on_key(&bs(1.1));
    assert!(o.consumed);
    assert_eq!(o.marked.as_deref(), Some(""));
    let o = e.on_key(&bs(1.2));
    assert!(!o.consumed, "empty word: backspace reaches the app");
    let mut t = 2.0;
    let outs = type_str(&mut e, "x ", &mut t);
    assert_eq!(commits(&outs), vec!["x"]);
}

#[test]
fn manual_switch_in_word_flips_marked_text_and_labels_correction() {
    let (mut e, storage) = const_engine(0.1, inline_cfg());
    let mut t = 0.0;
    let outs = type_str(&mut e, "has", &mut t);
    assert_eq!(outs[2].marked.as_deref(), Some("has"));
    let o = shift_tap(&mut e, &mut t);
    assert!(o.consumed);
    assert_eq!(o.marked.as_deref(), Some("há"));
    assert_eq!(o.mode, Some(Lang::Vi));
    assert_eq!(o.overlay, Some(Overlay::Manual));
    let outs = type_str(&mut e, " ", &mut t);
    assert_eq!(commits(&outs), vec!["há"]);
    let evs = events(&storage);
    assert!(evs.contains(&Event::ManualSwitch {
        scope: SwitchScope::InWord,
        word_id: Some(0),
        to: Lang::Vi
    }));
    assert_eq!(
        labels(&evs),
        vec![(Target::Word(0), Lang::Vi, 1.0, LabelSource::Correction)]
    );
    assert!(evs.iter().any(|e| matches!(e, Event::ModelUpdate { .. })));
}

#[test]
fn manual_switch_after_commit_labels_previous_words() {
    let (mut e, storage) = const_engine(0.1, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "has for ", &mut t);
    let o = shift_tap(&mut e, &mut t);
    assert_eq!(o.commit, None, "committed text is not rewritten");
    assert_eq!(o.mode, Some(Lang::Vi));
    let evs = events(&storage);
    assert!(evs.contains(&Event::ManualSwitch {
        scope: SwitchScope::AfterCommit,
        word_id: Some(1),
        to: Lang::Vi
    }));
    assert_eq!(
        labels(&evs),
        vec![
            (Target::Word(1), Lang::Vi, 1.0, LabelSource::Correction),
            (Target::Word(0), Lang::Vi, 0.5, LabelSource::Correction),
        ]
    );
    assert_eq!(e.mode(), Lang::Vi, "the next word starts in the new mode");
}

#[test]
fn manual_switch_before_typing_corrects_focus_prior() {
    let (mut e, storage) = const_engine(0.5, inline_cfg());
    let o = e.on_context(&ctx("com.apple.TextEdit", 7, 1.0));
    assert!(!o.consumed);
    let mut t = 1.0;
    shift_tap(&mut e, &mut t);
    let evs = events(&storage);
    assert!(evs.contains(&Event::ManualSwitch {
        scope: SwitchScope::BeforeTyping,
        word_id: None,
        to: Lang::En
    }));
    assert_eq!(
        labels(&evs),
        vec![(Target::Focus(0), Lang::En, 1.0, LabelSource::Correction)]
    );
}

#[test]
fn implicit_labels_after_window() {
    let (mut e, storage) = const_engine(0.1, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "a b c ", &mut t);
    let ls = labels(&events(&storage));
    assert_eq!(
        ls,
        vec![(Target::Word(0), Lang::Vi, 0.3, LabelSource::Implicit)]
    );
    // Timeout finalizes the rest on the next event.
    e.on_key(&flags(55, Modifiers::COMMAND, t + 60.0));
    assert_eq!(labels(&events(&storage)).len(), 3);
}

#[test]
fn ctrl_space_is_consumed_and_switches() {
    let (mut e, _s) = const_engine(0.1, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "ab", &mut t);
    let o = e.on_key(&code_key(keycode::SPACE, " ", Modifiers::CONTROL, 1.0));
    assert!(o.consumed);
    assert_eq!(o.mode, Some(Lang::En), "starts in VI, flips to EN");
    assert_eq!(o.commit, None, "hotkey never reaches the composer");
}

#[test]
fn word_ids_do_not_depend_on_mode() {
    let text = "tieengs Vieetj has, for test dduwowngf . ";
    let run = |p: f32| {
        let (mut e, storage) = const_engine(p, inline_cfg());
        let mut t = 0.0;
        type_str(&mut e, text, &mut t);
        events(&storage)
            .into_iter()
            .filter_map(|e| match e {
                Event::WordCommit { word_id, raw, .. } => Some((word_id, raw)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let vi = run(0.95);
    assert_eq!(vi, run(0.05));
    assert_eq!(vi.len(), 6);
}

#[test]
fn hotkeys_off_waits_for_injected_labels() {
    let cfg = EngineConfig {
        hotkeys: false,
        ..inline_cfg()
    };
    let (mut e, storage) = const_engine(0.1, cfg);
    let mut t = 0.0;
    type_str(&mut e, "ab", &mut t);
    let o = shift_tap(&mut e, &mut t);
    assert!(o.consumed, "hotkey swallowed to keep word ids aligned");
    assert_eq!(o.mode, None, "but no switch");
    type_str(&mut e, " cd ef gh ", &mut t);
    e.on_key(&flags(55, 0, t + 100.0));
    assert!(labels(&events(&storage)).is_empty(), "no implicit labels");
    e.inject_label(Target::Word(1), Lang::Vi, 1.0, LabelSource::Correction);
    e.inject_label(Target::Word(1), Lang::Vi, 1.0, LabelSource::Correction);
    assert_eq!(
        labels(&events(&storage)),
        vec![(Target::Word(1), Lang::Vi, 1.0, LabelSource::Correction)]
    );
    let diags = e.take_diagnostics();
    assert!(diags.iter().any(|d| d.contains("not pending")), "{diags:?}");
}

#[test]
fn commit_pending_commits_marked_text() {
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "vieetj", &mut t);
    let o = e.commit_pending(t + 1.0);
    assert_eq!(o.commit.as_deref(), Some("việt"));
    assert_eq!(e.commit_pending(t + 2.0).commit, None);
    assert!(events(&storage).iter().any(|e| matches!(
        e,
        Event::WordCommit {
            boundary: nimk_types::Boundary::External,
            ..
        }
    )));
}

#[test]
fn commit_rule_rejects_incomplete_syllable() {
    let (mut e, storage) = const_engine(0.5, inline_cfg());
    let mut t = 0.0;
    let outs = type_str(&mut e, "Vieet ", &mut t);
    assert_eq!(
        outs[4].marked.as_deref(),
        Some("Viêt"),
        "viable prefix stays VI mid-word"
    );
    assert_eq!(commits(&outs), vec!["Vieet"]);
    assert!(events(&storage).iter().any(|ev| matches!(
        ev,
        Event::Decision {
            target: Target::Word(0),
            at_key: 5,
            rule: Some(nimk_types::DecisionRule::InvalidVi),
            chosen: Lang::En,
            ..
        }
    )));
}
