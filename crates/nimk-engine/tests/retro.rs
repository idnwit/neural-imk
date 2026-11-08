//! Double-Shift hotkey and retroactive correction (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use nimk_engine::{Engine, EngineConfig, Overlay, Replace};
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

fn replace(delete_before: u32, text: &str) -> Option<Replace> {
    Some(Replace {
        delete_before,
        text: text.to_owned(),
    })
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

#[test]
fn retro_replaces_word_and_space() {
    let (mut e, storage, seen) = scripted_engine(const_typist(0.9), inline_cfg());
    let mut t = 0.0;
    let outs = type_str(&mut e, "tieengs ", &mut t);
    assert_eq!(commits(&outs), vec!["tiếng"]);
    let o = double_shift(&mut e, &mut t);
    assert!(o.consumed);
    // "tiếng" + " " = 6 UTF-16 units (precomposed ế).
    assert_eq!(o.replace, replace(6, "tieengs "));
    assert_eq!(o.commit, None);
    assert_eq!(o.marked, None);
    assert_eq!(o.mode, Some(Lang::En));
    assert_eq!(o.overlay, Some(Overlay::Manual));
    assert_eq!(e.mode(), Lang::En);
    let evs = events(&storage);
    assert_eq!(
        switches(&evs),
        vec![(SwitchScope::AfterCommit, Some(0), Lang::En)]
    );
    assert_eq!(
        labels(&evs),
        vec![(Target::Word(0), Lang::En, 1.0, LabelSource::Correction)]
    );
    // Every step of the word is trained towards En at weight 1.0.
    {
        let seen = seen.lock().unwrap();
        let word: Vec<_> = seen.iter().take(8).collect();
        assert_eq!(word.len(), 8, "7 appends + the commit");
        assert!(word.iter().all(|x| x.target.render == Lang::En));
        assert!(word.iter().all(|x| (x.render_weight - 1.0).abs() < 1e-6));
        assert_eq!(word[7].target.edit, EditAction::Commit);
    }
    // The next word starts in the new mode and typing continues normally.
    let outs = type_str(&mut e, "ab ", &mut t);
    assert_eq!(commits(&outs), vec!["ab"]);
}

#[test]
fn retro_toggles_back_and_forth() {
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "tieengs ", &mut t);
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.replace, replace(6, "tieengs "));
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.replace, replace(8, "tiếng "));
    assert_eq!(o.mode, Some(Lang::Vi));
    // Every correction overrides the previous label of the word.
    let ls = labels(&events(&storage));
    assert_eq!(ls.len(), 2);
    assert_eq!((ls[0].1, ls[1].1), (Lang::En, Lang::Vi));
    assert!(ls.iter().all(|l| l.3 == LabelSource::Correction));
}

#[test]
fn retro_correction_overrides_label_finalized_by_timeout() {
    let cfg = inline_cfg();
    let (mut e, storage) = const_engine(0.9, cfg.clone());
    let mut t = 0.0;
    type_str(&mut e, "tieengs ", &mut t);
    // The implicit label times out (event time) before the hotkey.
    t += cfg.label_timeout_secs + 5.0;
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.replace, replace(6, "tieengs "));
    let ls = labels(&events(&storage));
    assert_eq!(ls.len(), 2, "{ls:?}");
    assert_eq!(
        ls[0],
        (Target::Word(0), Lang::Vi, 0.3, LabelSource::Implicit)
    );
    assert_eq!(
        ls[1],
        (Target::Word(0), Lang::En, 1.0, LabelSource::Correction)
    );
}

#[test]
fn retro_counts_utf16_for_viet_and_punctuation() {
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    let outs = type_str(&mut e, "Vieetj. ", &mut t);
    assert_eq!(commits(&outs), vec!["Việt"]);
    let o = double_shift(&mut e, &mut t);
    // "Việt" (4) + ". " (2).
    assert_eq!(o.replace, replace(6, "Vieetj. "));
    assert_eq!(
        labels(&events(&storage)),
        vec![(Target::Word(0), Lang::En, 1.0, LabelSource::Correction)]
    );
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.replace, replace(8, "Việt. "));
}

#[test]
fn retro_counts_astral_boundary_as_two_units() {
    let (mut e, _s) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "tieengs", &mut t);
    t += 0.1;
    // Commit key that is a printable astral character.
    let o = e.on_key(&code_key(40, "\u{1F642}", 0, t));
    assert_eq!(o.commit.as_deref(), Some("tiếng"));
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.replace, replace(5 + 2, "tieengs\u{1F642}"));
}

#[test]
fn retro_without_boundary_commit_directly_by_comma() {
    let (mut e, _s) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "tieengs,", &mut t);
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.replace, replace(6, "tieengs,"));
}

#[test]
fn single_tap_does_not_forget_the_word() {
    let (mut e, _s) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "tieengs ", &mut t);
    shift_tap(&mut e, &mut t);
    t += 1.0; // too late to pair with the next tap
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.replace, replace(6, "tieengs "));
}

#[test]
fn replace_is_cleared_by_other_keys() {
    type Step = fn(&mut Engine, &mut f64);
    let steps: Vec<(&str, Step)> = vec![
        ("new letter then erased", |e, t| {
            type_str(e, "a", t);
            *t += 0.1;
            e.on_key(&code_key(keycode::BACKSPACE, "\u{7f}", 0, *t));
        }),
        ("backspace on empty", |e, t| {
            *t += 0.1;
            let o = e.on_key(&code_key(keycode::BACKSPACE, "\u{7f}", 0, *t));
            assert!(!o.consumed);
        }),
        ("arrow", |e, t| {
            *t += 0.1;
            e.on_key(&code_key(keycode::LEFT, "\u{F702}", 0, *t));
        }),
        ("shortcut", |e, t| {
            *t += 0.1;
            e.on_key(&code_key(8, "c", Modifiers::COMMAND, *t));
        }),
        ("return", |e, t| {
            *t += 0.1;
            e.on_key(&code_key(keycode::RETURN, "\r", 0, *t));
        }),
        ("too many boundary chars", |e, t| {
            type_str(e, &" ".repeat(20), t);
        }),
    ];
    for (name, step) in steps {
        let (mut e, storage) = const_engine(0.9, inline_cfg());
        let mut t = 0.0;
        type_str(&mut e, "tieengs ", &mut t);
        step(&mut e, &mut t);
        let o = double_shift(&mut e, &mut t);
        assert_eq!(o.replace, None, "case {name}");
        assert_eq!(o.marked, None, "case {name}");
        assert_eq!(o.mode, Some(Lang::En), "case {name}: still flips the mode");
        assert!(labels(&events(&storage)).is_empty(), "case {name}");
        assert!(
            switches(&events(&storage)).contains(&(SwitchScope::AfterCommit, Some(0), Lang::En)),
            "case {name}"
        );
    }
}

#[test]
fn replace_is_cleared_by_focus_change_and_before_typing_flips_prior() {
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    e.on_context(&ctx("com.apple.TextEdit", 1, 1.0));
    let mut t = 1.0;
    type_str(&mut e, "tieengs ", &mut t);
    e.on_context(&ctx("com.apple.Notes", 2, t + 1.0));
    t += 1.0;
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.replace, None);
    assert_eq!(o.mode, Some(Lang::En));
    let evs = events(&storage);
    assert!(switches(&evs).contains(&(SwitchScope::BeforeTyping, None, Lang::En)));
    assert!(labels(&evs).contains(&(Target::Focus(1), Lang::En, 1.0, LabelSource::Correction)));
}

#[test]
fn focus_prior_comes_from_the_model_and_hotkey_corrects_it() {
    // The model says En for the prior (observation without a key).
    let script = typist(|o| if o.key.is_none() { 0.05 } else { 0.9 });
    let (mut e, storage, seen) = scripted_engine(script, inline_cfg());
    let o = e.on_context(&ctx("com.apple.Terminal", 9, 1.0));
    assert_eq!(o.mode, Some(Lang::En));
    assert_eq!(o.overlay, Some(Overlay::Auto));
    assert_eq!(e.mode(), Lang::En);
    let mut t = 1.0;
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.mode, Some(Lang::Vi));
    assert_eq!(o.replace, None);
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
    assert_eq!(o.replace, None);
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
