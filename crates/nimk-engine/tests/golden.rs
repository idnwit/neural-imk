//! Mechanics of the engine with scripted action models: Append / Pop /
//! Commit / Pass application, masks, hysteresis, labels and examples.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use nimk_engine::{EngineConfig, Overlay};
use nimk_types::{
    ActionScores, EditAction, Event, LabelSource, Lang, Modifiers, Observation, Target, keycode,
};

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
    assert!(outs[0].consumed, "Append consumes the key");
    assert_eq!(outs[0].marked.as_deref(), Some("t"));
    assert_eq!(outs[6].marked.as_deref(), Some("tiếng"));
    let space = &outs[7];
    assert!(!space.consumed, "Commit hands the key to the app");
    assert_eq!(space.marked, None);
    assert!(!outs[14].consumed);
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
    assert!(outs.iter().all(|o| o.overlay.is_none()));
}

#[test]
fn english_stays_english_and_moves_the_sticky_mode() {
    let (mut e, _s) = const_engine(0.1, inline_cfg());
    let mut t = 0.0;
    let outs = type_str(&mut e, "has string, test ", &mut t);
    assert_eq!(commits(&outs), vec!["has", "string", "test"]);
    assert_eq!(e.mode(), Lang::En);
    let first = outs.iter().find(|o| o.commit.is_some()).unwrap();
    assert_eq!(first.mode, Some(Lang::En), "mode moves at the commit");
    assert_eq!(first.overlay, Some(Overlay::Auto));
}

#[test]
fn every_model_call_logs_a_decision_without_rule() {
    let (mut e, storage) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "ab ", &mut t);
    let ds: Vec<_> = events(&storage)
        .into_iter()
        .filter_map(|ev| match ev {
            Event::Decision {
                target,
                at_key,
                policy_id,
                p_vi,
                rule,
                chosen,
                edit,
            } => Some((target, at_key, policy_id, p_vi, rule, chosen, edit)),
            _ => None,
        })
        .collect();
    let p = Some(0.9);
    let w0 = Target::Word(0);
    let id = || "fake@v0".to_owned();
    assert_eq!(
        ds,
        vec![
            (w0, 1, id(), p, None, Lang::Vi, Some(EditAction::Append)),
            (w0, 2, id(), p, None, Lang::Vi, Some(EditAction::Append)),
            (w0, 2, id(), p, None, Lang::Vi, Some(EditAction::Commit)),
        ]
    );
}

#[test]
fn backspace_pops_and_passes_when_buffer_is_empty() {
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
    assert!(
        !o.consumed,
        "empty buffer: Pop is masked, key reaches the app"
    );
    let mut t = 2.0;
    let outs = type_str(&mut e, "x ", &mut t);
    assert_eq!(commits(&outs), vec!["x"]);
}

/// Model that always wants `edit`, whatever the key.
fn always(edit: usize) -> std::sync::Arc<ScoreFn> {
    std::sync::Arc::new(move |_o: &Observation| {
        let mut e = [0.0; 4];
        e[edit] = 1.0;
        ActionScores { edit: e, p_vi: 0.9 }
    })
}

#[test]
fn mask_falls_back_to_pass() {
    // Append wanted everywhere: letters buffer, Cmd+A / Backspace / space-less
    // keys without a printable char cannot, and fall back to Pass.
    let (mut e, _s, _) = scripted_engine(always(0), inline_cfg());
    let o = e.on_key(&code_key(0, "a", Modifiers::COMMAND, 1.0));
    assert!(!o.consumed && o.marked.is_none(), "Append masked for Cmd+A");
    let o = e.on_key(&code_key(keycode::BACKSPACE, "\u{7f}", 0, 1.1));
    assert!(!o.consumed, "Append masked for Backspace");
    let o = e.on_key(&code_key(keycode::LEFT, "\u{F702}", 0, 1.2));
    assert!(!o.consumed);
    let o = e.on_key(&char_key('a', 1.3));
    assert!(o.consumed);
    // Append is masked for Option+b: the best legal edit wins (Pop here,
    // since every legal score ties at 0 and Pop comes first).
    let o = e.on_key(&code_key(0, "b", Modifiers::OPTION, 1.4));
    assert_eq!(o.marked.as_deref(), Some(""), "Option+key cannot append");

    // Pop and Commit wanted on an empty buffer: Pass.
    for edit in [1, 2] {
        let (mut e, _s, _) = scripted_engine(always(edit), inline_cfg());
        let o = e.on_key(&code_key(keycode::LEFT, "\u{F702}", 0, 1.0));
        assert!(!o.consumed && o.commit.is_none() && o.marked.is_none());
    }
}

#[test]
fn pass_keeps_the_buffer() {
    // Letters append, but 'x' is passed to the app mid-word.
    let script: std::sync::Arc<ScoreFn> = std::sync::Arc::new(|o: &Observation| {
        let pass = o.key.as_ref().is_some_and(|k| k.chars == "x");
        let mut e = [0.0; 4];
        e[if pass { 3 } else { 0 }] = 1.0;
        ActionScores { edit: e, p_vi: 0.1 }
    });
    let (mut e, _s, _) = scripted_engine(script, inline_cfg());
    let mut t = 0.0;
    let outs = type_str(&mut e, "axb", &mut t);
    assert!(outs[0].consumed);
    assert!(!outs[1].consumed, "Pass returns the key");
    assert!(outs[2].consumed);
    assert_eq!(
        outs[2].marked.as_deref(),
        Some("ab"),
        "buffer unchanged by Pass"
    );
}

fn cfg_margin(m: f32) -> EngineConfig {
    EngineConfig {
        switch_margin: m,
        ..inline_cfg()
    }
}

#[test]
fn hysteresis_follows_the_shown_rendering() {
    // p(VI) decided by the buffer before the key.
    let script = typist(|o| match o.raw.as_str() {
        "" => 0.1,   // leave the initial Vi (|0.1 - 0.5| > margin)
        "a" => 0.6,  // within margin of En: stay En
        "as" => 0.9, // confident Vi
        _ => 0.45,   // within margin: stay Vi
    });
    let (mut e, _s, _) = scripted_engine(script, cfg_margin(0.2));
    let mut t = 0.0;
    let outs = type_str(&mut e, "asf", &mut t);
    assert_eq!(outs[0].marked.as_deref(), Some("a"));
    assert_eq!(outs[1].marked.as_deref(), Some("as"), "0.6 stays En");
    assert_eq!(outs[2].marked.as_deref(), Some("à"), "0.9 flips to Vi");
    let o = e.on_key(&char_key('b', t + 0.1));
    assert_eq!(o.marked.as_deref(), Some("àb"), "0.45 keeps Vi");
}

#[test]
fn hysteresis_margin_is_strict() {
    let p = Arc::new(Mutex::new(0.9f32));
    let p2 = Arc::clone(&p);
    let (mut e, _s, _) = scripted_engine(typist(move |_| *p2.lock().unwrap()), cfg_margin(0.25));
    let feed = |e: &mut nimk_engine::Engine, c: char, pv: f32, t: f64| {
        *p.lock().unwrap() = pv;
        e.on_key(&char_key(c, t)).marked
    };
    assert_eq!(feed(&mut e, 'a', 0.9, 1.0).as_deref(), Some("a"));
    assert_eq!(
        feed(&mut e, 's', 0.25, 2.0).as_deref(),
        Some("á"),
        "|p-0.5| == margin stays"
    );
    assert_eq!(
        feed(&mut e, 'x', 0.24, 3.0).as_deref(),
        Some("asx"),
        "beyond margin flips"
    );
}

#[test]
fn non_finite_scores_do_not_break_the_engine() {
    let script: Arc<ScoreFn> = Arc::new(|_o: &Observation| ActionScores {
        edit: [f32::NAN; 4],
        p_vi: f32::NAN,
    });
    let (mut e, storage, _) = scripted_engine(script, inline_cfg());
    let o = e.on_key(&char_key('a', 1.0));
    assert!(!o.consumed, "NaN edit scores fall back to Pass");
    assert!(
        events(&storage)
            .iter()
            .any(|ev| matches!(ev, Event::Decision { p_vi: None, .. }))
    );
    assert!(e.take_diagnostics().is_empty());
}

#[test]
fn manual_switch_in_word_flips_marked_text_and_locks_render() {
    let (mut e, storage, seen) = scripted_engine(const_typist(0.1), inline_cfg());
    let mut t = 0.0;
    let outs = type_str(&mut e, "has", &mut t);
    assert_eq!(outs[2].marked.as_deref(), Some("has"));
    let o = double_shift(&mut e, &mut t);
    assert!(o.consumed);
    assert_eq!(o.marked.as_deref(), Some("há"));
    assert_eq!(o.mode, Some(Lang::Vi));
    assert_eq!(o.overlay, Some(Overlay::Manual));
    assert_eq!(o.replace, None);
    // The model still says En (0.1) but the user's choice is locked.
    let o = e.on_key(&char_key('f', t + 0.1));
    assert_eq!(o.marked.as_deref(), Some("hà"));
    let outs = type_str(&mut e, " ", &mut t);
    assert_eq!(commits(&outs), vec!["hà"]);
    let evs = events(&storage);
    assert!(evs.contains(&Event::ManualSwitch {
        scope: nimk_types::SwitchScope::InWord,
        word_id: Some(0),
        to: Lang::Vi
    }));
    assert_eq!(
        labels(&evs),
        vec![(Target::Word(0), Lang::Vi, 1.0, LabelSource::Correction)]
    );
    // Examples of every step of the word: render = Vi at weight 1.0, edit =
    // the edit taken, edit head untouched online.
    let seen = seen.lock().unwrap();
    // First update = the word itself (the correction then re-trains the
    // window several times).
    let steps: Vec<_> = seen.iter().take(5).collect();
    assert!(seen.len() >= 5, "h a s f + commit");
    assert!(steps.iter().all(|x| x.target.render == Lang::Vi));
    assert!(steps.iter().all(|x| (x.render_weight - 1.0).abs() < 1e-6));
    assert!(steps.iter().all(|x| x.edit_weight == 0.0));
    let edits: Vec<_> = steps.iter().map(|x| x.target.edit).collect();
    assert_eq!(
        edits,
        vec![
            EditAction::Append,
            EditAction::Append,
            EditAction::Append,
            EditAction::Append,
            EditAction::Commit
        ]
    );
    assert_eq!(steps[2].obs.raw, "ha");
    assert_eq!(steps[4].obs.raw, "hasf");
}

#[test]
fn edit_online_weight_reaches_the_examples() {
    let cfg = EngineConfig {
        edit_online_weight: 0.5,
        ..inline_cfg()
    };
    let (mut e, _s, seen) = scripted_engine(const_typist(0.9), cfg);
    let mut t = 0.0;
    type_str(&mut e, "ab ", &mut t);
    double_shift(&mut e, &mut t);
    let seen = seen.lock().unwrap();
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|x| (x.edit_weight - 0.5).abs() < 1e-6));
}

#[test]
fn implicit_labels_after_window_carry_implicit_weight() {
    let (mut e, storage, seen) = scripted_engine(const_typist(0.1), inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "a b c ", &mut t);
    let ls = labels(&events(&storage));
    assert_eq!(
        ls,
        vec![(Target::Word(0), Lang::En, 0.3, LabelSource::Implicit)]
    );
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "word 0 had Append + Commit");
        assert!(seen.iter().all(|x| (x.render_weight - 0.3).abs() < 1e-6));
    }
    // The timeout finalizes the rest on the next event.
    e.on_key(&flags(55, Modifiers::COMMAND, t + 60.0));
    assert_eq!(labels(&events(&storage)).len(), 3);
}

#[test]
fn ctrl_space_is_consumed_and_switches() {
    let (mut e, _s) = const_engine(0.9, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, "ab", &mut t);
    let o = e.on_key(&code_key(keycode::SPACE, " ", Modifiers::CONTROL, 1.0));
    assert!(o.consumed);
    assert_eq!(o.mode, Some(Lang::En), "in-word flip of the shown Vi");
    assert_eq!(o.commit, None, "hotkey never reaches the model");
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
    let o = double_shift(&mut e, &mut t);
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
fn word_ids_do_not_depend_on_render() {
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
fn observation_carries_the_context() {
    let seen: Arc<Mutex<Vec<Observation>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let script: Arc<ScoreFn> = {
        let inner = const_typist(0.9);
        Arc::new(move |o: &Observation| {
            sink.lock().unwrap().push(o.clone());
            inner(o)
        })
    };
    let (mut e, _s, _) = scripted_engine(script, inline_cfg());
    e.on_context(&ctx("com.apple.TextEdit", 3, 1.0));
    let mut t = 1.0;
    type_str(&mut e, "tieengs ", &mut t);
    // Hotkey events never reach the model nor the window.
    double_shift(&mut e, &mut t);
    type_str(&mut e, "a", &mut t);
    let obs = seen.lock().unwrap();
    let prior = &obs[0];
    assert!(prior.key.is_none());
    assert_eq!(prior.mask, nimk_types::ActionMask::default());
    assert_eq!(prior.bundle_id, "com.apple.TextEdit");
    assert_eq!(prior.shown, prior.mode);
    // 7 keys + space + 1 after the hotkey + prior.
    assert_eq!(obs.len(), 1 + 8 + 1);
    let last_word_key = &obs[7];
    assert_eq!(last_word_key.raw, "tieeng");
    assert_eq!(last_word_key.vi, "tiêng");
    assert!(last_word_key.viable_vi);
    assert!(last_word_key.mask.append && last_word_key.mask.pop && last_word_key.mask.commit);
    assert_eq!(last_word_key.recent.len(), 6);
    let space = &obs[8];
    assert_eq!(space.raw, "tieengs");
    assert_eq!(space.vi, "tiếng");
    assert!(space.valid_vi);
    assert!(!space.mask.append || space.key.as_ref().is_some_and(|k| k.chars == " "));
    let after = obs.last().unwrap();
    assert_eq!(after.raw, "");
    assert_eq!(
        after.prev_langs,
        vec![Lang::En],
        "hotkey rewrote the last word to En"
    );
    assert!(after.secs_since_switch < 1.0, "hotkey just fired");
    assert_eq!(after.recent.len(), 8, "hotkey events not in the window");
    assert_eq!(prior.secs_since_switch, f32::INFINITY);
}

#[test]
fn recent_window_is_capped() {
    let seen: Arc<Mutex<Vec<Observation>>> = Arc::default();
    let sink = Arc::clone(&seen);
    let inner = const_typist(0.9);
    let script: Arc<ScoreFn> = Arc::new(move |o: &Observation| {
        sink.lock().unwrap().push(o.clone());
        inner(o)
    });
    let (mut e, _s, _) = scripted_engine(script, inline_cfg());
    let mut t = 0.0;
    type_str(&mut e, &"b".repeat(30), &mut t);
    let obs = seen.lock().unwrap();
    assert_eq!(obs.last().unwrap().recent.len(), nimk_types::RECENT_WINDOW);
}

#[test]
fn full_buffer_masks_append() {
    let (mut e, _s) = const_engine(0.1, inline_cfg());
    let mut t = 0.0;
    let outs = type_str(&mut e, &"a".repeat(66), &mut t);
    assert!(outs[..64].iter().all(|o| o.consumed));
    // Append is masked at 64 keys; the typist commits instead, then the
    // next letters start a new word.
    assert_eq!(outs[64].commit.as_deref(), Some("a".repeat(64).as_str()));
    assert!(!outs[64].consumed);
    assert!(outs[65].consumed);
}
