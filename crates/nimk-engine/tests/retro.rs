//! Caret flip (ADR 0015): the hotkey with no pending word, then
//! `retro_flip` of the word the host found at the caret.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use nimk_engine::{Engine, Overlay, RetroFlip};
use nimk_types::{Event, LabelSource, Lang, SwitchScope, Target};

const TIENG: &str = "ti\u{1ebf}ng";

fn labels(evs: &[Event]) -> Vec<(Target, Lang, LabelSource)> {
    evs.iter()
        .filter_map(|e| match e {
            Event::Label {
                target,
                label,
                source,
                ..
            } => Some((*target, *label, *source)),
            _ => None,
        })
        .collect()
}

fn retro_switches(evs: &[Event]) -> Vec<(Option<u64>, Lang)> {
    evs.iter()
        .filter_map(|e| match e {
            Event::ManualSwitch {
                scope: SwitchScope::Retro,
                word_id,
                to,
            } => Some((*word_id, *to)),
            _ => None,
        })
        .collect()
}

/// Types `text` in `bundle` and commits everything: no word pending.
fn typed(e: &mut Engine, bundle: &str, text: &str, t: &mut f64) {
    e.on_context(&ctx(bundle, 1, *t));
    type_str(e, text, t);
    *t += 0.1;
    e.commit_pending(*t);
}

#[test]
fn flips_a_committed_word_in_place_and_labels_it() {
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    typed(&mut e, "com.microsoft.VSCode", "tieengs xin chao", &mut t);
    assert_eq!(e.mode(), Lang::Vi);
    let o = double_shift(&mut e, &mut t);
    assert_eq!((o.mode, o.overlay), (Some(Lang::En), Some(Overlay::Manual)));
    assert_eq!(
        o.marked, None,
        "no pending word: the host tries a caret flip"
    );

    let flip = e.retro_flip(TIENG, t).unwrap();
    assert_eq!(
        flip,
        RetroFlip {
            replacement: "tieengs".into(),
            to: Lang::En,
            word_id: Some(0),
        }
    );
    assert_eq!(e.mode(), Lang::Vi, "the press's mode switch is undone");
    let evs = events(&storage);
    assert_eq!(retro_switches(&evs), vec![(Some(0), Lang::En)]);
    assert!(labels(&evs).contains(&(Target::Word(0), Lang::En, LabelSource::Correction)));

    // Flipping the replaced word again restores it.
    double_shift(&mut e, &mut t);
    let back = e.retro_flip("tieengs", t).unwrap();
    assert_eq!((back.replacement.as_str(), back.to), (TIENG, Lang::Vi));
    let evs = events(&storage);
    assert_eq!(
        retro_switches(&evs),
        vec![(Some(0), Lang::En), (Some(0), Lang::Vi)]
    );
    assert_eq!(
        labels(&evs)
            .iter()
            .rfind(|l| l.0 == Target::Word(0))
            .copied(),
        Some((Target::Word(0), Lang::Vi, LabelSource::Correction))
    );
}

#[test]
fn most_recent_match_wins() {
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    typed(&mut e, "dev.zed.Zed", "tieengs a tieengs", &mut t);
    double_shift(&mut e, &mut t);
    let flip = e.retro_flip(TIENG, t).unwrap();
    assert_eq!(flip.word_id, Some(2));
    assert_eq!(retro_switches(&events(&storage)), vec![(Some(2), Lang::En)]);
}

#[test]
fn only_right_after_a_hotkey_with_no_word() {
    let (mut e, _s) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    typed(&mut e, "dev.zed.Zed", "tieengs", &mut t);
    assert_eq!(e.retro_flip(TIENG, t), None, "no hotkey press");

    double_shift(&mut e, &mut t);
    type_str(&mut e, "a", &mut t);
    assert_eq!(e.retro_flip(TIENG, t), None, "an event came in between");

    // A press that flipped the pending word is not undoable.
    let (mut e, _s) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    e.on_context(&ctx("dev.zed.Zed", 1, t));
    type_str(&mut e, "tieengs ", &mut t);
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.marked.as_deref(), Some("tieengs "));
    assert_eq!(e.retro_flip(TIENG, t), None);
}

#[test]
fn unknown_word_gets_the_telex_flip_without_a_label() {
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    typed(&mut e, "dev.zed.Zed", "xin", &mut t);
    double_shift(&mut e, &mut t);
    let flip = e.retro_flip("vieejt", t).unwrap();
    assert_eq!(
        (flip.replacement.as_str(), flip.to, flip.word_id),
        ("vi\u{1ec7}t", Lang::Vi, None)
    );
    let evs = events(&storage);
    assert_eq!(retro_switches(&evs), vec![(None, Lang::Vi)]);
    assert!(labels(&evs).iter().all(|l| l.2 != LabelSource::Correction));
}

#[test]
fn words_of_another_app_are_not_matched() {
    let (mut e, _s) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    typed(&mut e, "dev.zed.Zed", "tieengs", &mut t);
    e.on_context(&ctx("com.apple.TextEdit", 2, t));
    double_shift(&mut e, &mut t);
    let flip = e.retro_flip(TIENG, t).unwrap();
    assert_eq!(flip.word_id, None, "stateless fallback");
    assert_eq!(flip.replacement, "tieengs");
}

#[test]
fn nothing_to_flip_keeps_the_mode_switch() {
    let (mut e, _s) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    typed(&mut e, "dev.zed.Zed", "xin", &mut t);
    double_shift(&mut e, &mut t);
    assert_eq!(e.retro_flip("hello", t), None);
    assert_eq!(e.mode(), Lang::En, "the plain mode switch stands");
}

#[test]
fn caret_flip_before_typing_drops_the_focus_prior_correction() {
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    let mut t = 1.0;
    e.on_context(&ctx("com.apple.TextEdit", 3, t));
    double_shift(&mut e, &mut t);
    assert!(e.retro_flip("vieejt", t).is_some());
    e.on_key(&flags(55, 0, t + 0.1));
    let evs = events(&storage);
    assert!(
        !labels(&evs).iter().any(|l| matches!(l.0, Target::Focus(_))),
        "{:?}",
        labels(&evs)
    );
}

#[test]
fn replay_retro_undoes_the_press() {
    let (mut e, _s) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    typed(&mut e, "dev.zed.Zed", "xin", &mut t);
    double_shift(&mut e, &mut t);
    assert_eq!(e.mode(), Lang::En);
    e.replay_retro();
    assert_eq!(e.mode(), Lang::Vi);
}

#[test]
fn retro_policy_follows_the_app_category() {
    let cfg = nimk_engine::EngineConfig {
        threading: nimk_engine::Threading::Inline,
        ..nimk_engine::EngineConfig::default()
    };
    let (mut e, _s) = const_engine(0.9, cfg);
    e.on_context(&ctx("com.github.wez.wezterm", 1, 0.0));
    let p = e.retro_policy();
    assert!(p.text && p.terminal);
    e.on_context(&ctx("dev.zed.Zed", 1, 0.1));
    let p = e.retro_policy();
    assert!(p.text && !p.terminal);
}
