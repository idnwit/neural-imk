//! Double-Shift hotkey and the pending word flip (ADR 0011, 0012). The boundary
//! table of ADR 0012 lives in `boundary.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use nimk_engine::{EngineConfig, Overlay};
use nimk_types::{
    EditAction, Event, KeyEvent, LabelSource, Lang, Modifiers, SwitchScope, Target, keycode,
};

const S: u32 = Modifiers::SHIFT;
const LS: u16 = keycode::LEFT_SHIFT;
const RS: u16 = keycode::RIGHT_SHIFT;

fn labels(evs: &[Event]) -> Vec<(Target, Lang, f32, LabelSource)> {
    evs.iter()
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

fn switches(evs: &[Event]) -> Vec<(SwitchScope, Option<u64>, Lang)> {
    evs.iter()
        .filter_map(|e| match e {
            Event::ManualSwitch { scope, word_id, to } => Some((*scope, *word_id, *to)),
            _ => None,
        })
        .collect()
}

/// Whether the hotkey fired on any event of the stream.
fn fires(events: &[KeyEvent]) -> bool {
    let (mut e, _s) = const_engine(0.9, inline_cfg());
    events.iter().any(|ev| {
        let o = e.on_key(ev);
        o.consumed && o.overlay == Some(Overlay::Manual)
    })
}

#[test]
fn double_shift_detection_table() {
    let a = |t| char_key('a', t);
    type Case = (&'static str, Vec<KeyEvent>, bool);
    let cases: Vec<Case> = vec![
        (
            "single tap does nothing",
            vec![flags(LS, S, 1.0), flags(LS, 0, 1.1)],
            false,
        ),
        (
            "double tap",
            vec![
                flags(LS, S, 1.0),
                flags(LS, 0, 1.1),
                flags(LS, S, 1.2),
                flags(LS, 0, 1.3),
            ],
            true,
        ),
        (
            "either shift",
            vec![
                flags(RS, S, 1.0),
                flags(RS, 0, 1.1),
                flags(LS, S, 1.2),
                flags(LS, 0, 1.3),
            ],
            true,
        ),
        (
            "second release too late",
            vec![
                flags(LS, S, 1.0),
                flags(LS, 0, 1.1),
                flags(LS, S, 1.5),
                flags(LS, 0, 1.6),
            ],
            false,
        ),
        (
            "tap held too long",
            vec![
                flags(LS, S, 1.0),
                flags(LS, 0, 1.5),
                flags(LS, S, 1.6),
                flags(LS, 0, 1.7),
            ],
            false,
        ),
        (
            "key between taps",
            vec![
                flags(LS, S, 1.0),
                flags(LS, 0, 1.1),
                a(1.15),
                flags(LS, S, 1.2),
                flags(LS, 0, 1.3),
            ],
            false,
        ),
        (
            "modifier between taps",
            vec![
                flags(LS, S, 1.0),
                flags(LS, 0, 1.1),
                flags(55, Modifiers::COMMAND, 1.15),
                flags(55, 0, 1.17),
                flags(LS, S, 1.2),
                flags(LS, 0, 1.3),
            ],
            false,
        ),
        (
            "shifted letter",
            vec![
                flags(LS, S, 1.0),
                code_key(0, "A", S, 1.05),
                flags(LS, 0, 1.1),
                flags(LS, S, 1.2),
                flags(LS, 0, 1.3),
            ],
            false,
        ),
    ];
    for (name, events, want) in cases {
        assert_eq!(fires(&events), want, "case {name}");
    }
}

#[test]
fn single_tap_passes_through() {
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "ab", &mut t);
    let o = shift_tap(&mut e, &mut t);
    assert_eq!(o, nimk_engine::EngineOutput::default());
    assert!(switches(&events(&storage)).is_empty());
    assert_eq!(e.mode(), Lang::Vi);
}

#[test]
fn double_shift_can_be_disabled() {
    let cfg = EngineConfig {
        double_shift: false,
        // Ctrl+Space is off by default; this test opts in.
        ctrl_space: true,
        ..inline_cfg()
    };
    let (mut e, storage) = const_engine(0.9, cfg);
    let mut t = 0.0;
    type_str(&mut e, "ab", &mut t);
    shift_tap(&mut e, &mut t);
    let o = shift_tap(&mut e, &mut t);
    assert!(!o.consumed);
    assert!(switches(&events(&storage)).is_empty());
    // Ctrl+Space still works.
    let o = e.on_key(&code_key(keycode::SPACE, " ", Modifiers::CONTROL, t + 1.0));
    assert!(o.consumed);
    assert_eq!(switches(&events(&storage)).len(), 1);
}

fn key_at(code: u16, chars: &str, mods: u32, t: &mut f64) -> KeyEvent {
    *t += 0.1;
    code_key(code, chars, mods, *t)
}

fn word_commit_ids(storage: &nimk_storage::MemStorage) -> Vec<u64> {
    events(storage)
        .into_iter()
        .filter_map(|ev| match ev {
            Event::WordCommit { word_id, .. } => Some(word_id),
            _ => None,
        })
        .collect()
}

#[test]
fn word_stays_pending_until_the_next_word() {
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    let outs = type_str(&mut e, "tieengs ", &mut t);
    // Nothing is inserted yet: the word and its space are marked text.
    assert!(commits(&outs).is_empty());
    let last = outs.last().unwrap();
    assert!(last.consumed);
    assert_eq!(last.marked.as_deref(), Some("ti\u{1ebf}ng "));
    assert!(word_commit_ids(&storage).is_empty());
    // A letter commits it and starts the next word.
    let o = type_str(&mut e, "x", &mut t).remove(0);
    assert_eq!(o.commit.as_deref(), Some("ti\u{1ebf}ng "));
    assert_eq!(o.marked.as_deref(), Some("x"));
    assert!(o.consumed);
    assert_eq!(word_commit_ids(&storage), vec![0]);
}

#[test]
fn flip_while_pending_changes_marked_text_and_labels_correction() {
    let (mut e, storage, seen) = scripted_engine(const_typist(0.9), inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "tieengs ", &mut t);
    let o = double_shift(&mut e, &mut t);
    assert!(o.consumed);
    assert_eq!(o.marked.as_deref(), Some("tieengs "));
    assert_eq!(o.commit, None);
    assert_eq!(o.mode, Some(Lang::En));
    assert_eq!(o.overlay, Some(Overlay::Manual));
    assert_eq!(e.mode(), Lang::En);
    // Flip back and forth: the last flip wins.
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.marked.as_deref(), Some("ti\u{1ebf}ng "));
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.marked.as_deref(), Some("tieengs "));
    let o = type_str(&mut e, "a", &mut t).remove(0);
    assert_eq!(o.commit.as_deref(), Some("tieengs "));
    let evs = events(&storage);
    assert_eq!(
        switches(&evs),
        vec![
            (SwitchScope::AfterCommit, Some(0), Lang::En),
            (SwitchScope::AfterCommit, Some(0), Lang::Vi),
            (SwitchScope::AfterCommit, Some(0), Lang::En),
        ]
    );
    assert_eq!(
        labels(&evs),
        vec![(Target::Word(0), Lang::En, 1.0, LabelSource::Correction)]
    );
    let seen = seen.lock().unwrap();
    let word: Vec<_> = seen.iter().take(8).collect();
    assert_eq!(word.len(), 8, "7 appends + the commit");
    assert!(word.iter().all(|x| x.target.render == Lang::En));
    assert_eq!(word[7].target.edit, EditAction::Commit);
}

#[test]
fn unflipped_pending_word_is_labelled_implicitly() {
    let cfg = inline_cfg();
    let (mut e, storage) = const_engine(0.9, cfg.clone());
    let mut t = 0.0;
    type_str(&mut e, "tieengs x", &mut t);
    assert!(labels(&events(&storage)).is_empty());
    t += cfg.label_timeout_secs + 5.0;
    e.on_key(&char_key('y', t));
    assert_eq!(
        labels(&events(&storage)),
        vec![(Target::Word(0), Lang::Vi, 0.3, LabelSource::Implicit)]
    );
}

#[test]
fn model_decides_the_rendering_of_the_pending_word() {
    for (p, want) in [(0.1, "has "), (0.9, "h\u{e1} ")] {
        let (mut e, _s) = const_engine(p, inline_cfg());
        let mut t = 0.0;
        type_str(&mut e, "has ", &mut t);
        let o = type_str(&mut e, "x", &mut t).remove(0);
        assert_eq!(o.commit.as_deref(), Some(want));
    }
}

#[test]
fn focus_change_and_model_switch_commit_pending() {
    let (mut e, _s) = const_engine(0.9, inline_cfg());
    e.on_context(&ctx("a", 1, 1.0));
    let mut t = 1.0;
    type_str(&mut e, "has ", &mut t);
    let o = e.on_context(&ctx("b", 2, t + 1.0));
    assert_eq!(o.commit.as_deref(), Some("h\u{e1} "));
    type_str(&mut e, "has ", &mut t);
    let o = e.switch_model(nimk_engine::ModelKind::GruAction, t + 1.0);
    assert_eq!(o.commit.as_deref(), Some("h\u{e1} "));
}

#[test]
fn word_ids_do_not_depend_on_the_rendering() {
    let ids = |p: f32| {
        let (mut e, storage) = const_engine(p, inline_cfg());
        let mut t = 0.0;
        type_str(&mut e, "tieengs, has @ Vieetj", &mut t);
        e.on_key(&key_at(keycode::RETURN, "\r", 0, &mut t));
        e.shutdown().unwrap();
        events(&storage)
            .into_iter()
            .filter_map(|ev| match ev {
                Event::Key { word_id, .. } => Some(word_id),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(0.9), ids(0.1));
}

#[test]
fn focus_prior_comes_from_the_model_and_hotkey_corrects_it() {
    // The model says En for the prior (observation without a key).
    let script = typist(|o| if o.key.is_none() { 0.05 } else { 0.9 });
    let (mut e, storage, seen) = scripted_engine(script, inline_cfg());
    let o = e.on_context(&ctx("com.apple.Terminal", 9, 1.0));
    assert_eq!(o.mode, Some(Lang::En));
    assert_eq!(o.overlay, None, "the focus prior shows no overlay");
    assert_eq!(e.mode(), Lang::En);
    let mut t = 1.0;
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.mode, Some(Lang::Vi));
    let evs = events(&storage);
    assert!(evs.iter().any(|ev| matches!(
        ev,
        Event::Decision {
            target: Target::Focus(0),
            at_key: 0,
            p_vi: Some(_),
            rule: None,
            chosen: Lang::En,
            ..
        }
    )));
    assert_eq!(
        labels(&evs),
        vec![(Target::Focus(0), Lang::Vi, 1.0, LabelSource::Correction)]
    );
    let seen = seen.lock().unwrap();
    let ex = seen.first().unwrap();
    assert!(ex.obs.key.is_none());
    assert_eq!(ex.target.render, Lang::Vi);
    assert_eq!(ex.target.edit, EditAction::Pass);
}

#[test]
fn typed_past_the_last_word_flips_mode_only() {
    // A new word is in progress: the hotkey flips its rendering (in-word),
    // not the committed word.
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "tieengs has", &mut t);
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.marked.as_deref(), Some("has"));
    let evs = events(&storage);
    assert_eq!(
        switches(&evs),
        vec![(SwitchScope::InWord, Some(1), Lang::En)]
    );
    // Word 0 was never corrected.
    assert!(labels(&evs).iter().all(|l| l.0 != Target::Word(0)));
}

#[test]
fn logs_are_deterministic() {
    let run = || {
        let (mut e, storage) = const_engine(0.9, inline_cfg());
        e.on_context(&ctx("a", 1, 1.0));
        let mut t = 1.0;
        type_str(&mut e, "tieengs Vieetj. ", &mut t);
        double_shift(&mut e, &mut t);
        type_str(&mut e, "has", &mut t);
        double_shift(&mut e, &mut t);
        e.shutdown().unwrap();
        log_bytes(&storage)
    };
    let a = run();
    assert!(a.len() > 30);
    assert_eq!(a, run());
}

#[test]
fn vscode_doubled_flags_events_still_flip_once() {
    let (mut e, _s) = const_engine(0.9, inline_cfg());
    let trace = [
        flags(LS, S, 1.384),
        flags(LS, S, 1.391),
        flags(LS, 0, 1.473),
        flags(LS, 0, 1.476),
        flags(LS, S, 1.551),
        flags(LS, S, 1.555),
        flags(LS, 0, 1.675),
        flags(LS, 0, 1.679),
    ];
    let outs: Vec<_> = trace.iter().map(|k| e.on_key(k)).collect();
    // Duplicates are never consumed and never switch.
    assert!(outs.iter().all(|o| o.overlay.is_none() != o.consumed));
    assert_eq!(outs.iter().filter(|o| o.consumed).count(), 1);
    assert!(outs[6].consumed || outs[7].consumed);
    assert!(!outs[7].consumed, "the duplicate release is a no-op");
}

#[test]
fn double_shift_right_after_focus_reports_the_new_mode() {
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    e.on_context(&ctx("com.apple.dt.Xcode", 1, 1.0));
    let mut t = 1.0;
    let o = double_shift(&mut e, &mut t);
    assert!(o.consumed);
    assert_eq!(o.mode, Some(Lang::En));
    assert_eq!(o.overlay, Some(Overlay::Manual));
    assert!(switches(&events(&storage)).contains(&(SwitchScope::BeforeTyping, None, Lang::En)));
}
