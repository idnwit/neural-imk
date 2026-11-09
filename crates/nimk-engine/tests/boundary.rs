//! The unified boundary/commit state machine of ADR 0012: Idle, Composing,
//! Pending (word + one boundary char) and Raw (commands).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::*;
use nimk_engine::{Engine, EngineConfig, EngineOutput, Overlay};
use nimk_types::{Event, LabelSource, Lang, Modifiers, SwitchScope, Target, keycode};

/// English model: words render as typed.
fn en() -> (Engine, nimk_storage::MemStorage) {
    const_engine(0.1, inline_cfg())
}

fn vi() -> (Engine, nimk_storage::MemStorage) {
    const_engine(0.9, inline_cfg())
}

fn key(e: &mut Engine, code: u16, chars: &str, mods: u32, t: &mut f64) -> EngineOutput {
    *t += 0.1;
    e.on_key(&code_key(code, chars, mods, *t))
}

fn ret(e: &mut Engine, t: &mut f64) -> EngineOutput {
    key(e, keycode::RETURN, "\r", 0, t)
}

fn decisions(storage: &nimk_storage::MemStorage) -> usize {
    events(storage)
        .iter()
        .filter(|e| matches!(e, Event::Decision { .. }))
        .count()
}

fn passed(o: &EngineOutput) -> bool {
    !o.consumed && o.marked.is_none() && o.commit.is_none()
}

#[test]
fn idle_symbols_and_digits_pass_without_a_decision() {
    let (mut e, s) = en();
    let mut t = 0.0;
    for c in [',', '1', '.', ' ', '-', '~'] {
        let o = type_str(&mut e, &c.to_string(), &mut t).remove(0);
        assert!(passed(&o), "{c:?}: {o:?}");
    }
    let o = key(&mut e, keycode::BACKSPACE, "\u{7f}", 0, &mut t);
    assert!(passed(&o));
    assert_eq!(decisions(&s), 0);
}

#[test]
fn has_dot_stays_marked_and_flippable() {
    let (mut e, s) = en();
    let mut t = 0.0;
    let outs = type_str(&mut e, "has.", &mut t);
    assert!(commits(&outs).is_empty());
    assert_eq!(outs[3].marked.as_deref(), Some("has."));
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.marked.as_deref(), Some("h\u{e1}."));
    let o = type_str(&mut e, "q", &mut t).remove(0);
    assert_eq!(o.commit.as_deref(), Some("h\u{e1}."));
    assert_eq!(o.marked.as_deref(), Some("q"));
    let labels: Vec<_> = events(&s)
        .into_iter()
        .filter_map(|ev| match ev {
            Event::Label {
                target,
                label,
                source,
                ..
            } => Some((target, label, source)),
            _ => None,
        })
        .collect();
    assert_eq!(
        labels,
        vec![(Target::Word(0), Lang::Vi, LabelSource::Correction)]
    );
}

/// Regression (real install, 2026-11-09): the old engine appended the
/// boundary to the raw word, so Telex saw `taij:` and rendered `taị:`.
/// A boundary must end the word before Telex runs.
#[test]
fn boundary_after_tone_key_keeps_tone_placement() {
    for (typed, word) in [
        ("taij", "t\u{1ea1}i"),
        ("loaij", "lo\u{1ea1}i"),
        ("hoaij", "ho\u{1ea1}i"),
        ("muaf", "m\u{f9}a"),
    ] {
        for boundary in [':', ',', '.', ';', ')'] {
            let (mut e, _s) = vi();
            let mut t = 0.0;
            let outs = type_str(&mut e, &format!("{typed}{boundary}"), &mut t);
            let shown = outs.last().and_then(|o| o.marked.clone());
            assert_eq!(
                shown,
                Some(format!("{word}{boundary}")),
                "{typed}{boundary}"
            );
            let o = type_str(&mut e, " ", &mut t).remove(0);
            assert_eq!(
                o.commit,
                Some(format!("{word}{boundary}")),
                "{typed}{boundary}"
            );
        }
    }
}

#[test]
fn hello_world_commits_hello_comma_on_the_space_and_passes_it() {
    let (mut e, _s) = en();
    let mut t = 0.0;
    let outs = type_str(&mut e, "Hello, world", &mut t);
    assert_eq!(outs[5].marked.as_deref(), Some("Hello,"));
    assert!(outs[5].commit.is_none());
    // The space: commit "Hello," and let the space reach the app.
    assert_eq!(outs[6].commit.as_deref(), Some("Hello,"));
    assert!(!outs[6].consumed && outs[6].marked.is_none());
    assert_eq!(outs[11].marked.as_deref(), Some("world"));
    assert_eq!(e.commit_pending(t + 1.0).commit.as_deref(), Some("world"));
}

#[test]
fn ls_dash_la_return() {
    let (mut e, _s) = en();
    let mut t = 0.0;
    let outs = type_str(&mut e, "ls -la", &mut t);
    assert_eq!(outs[2].marked.as_deref(), Some("ls "));
    assert_eq!(outs[3].commit.as_deref(), Some("ls "));
    assert!(!outs[3].consumed, "the dash goes to the app");
    assert_eq!(outs[5].marked.as_deref(), Some("la"));
    let o = ret(&mut e, &mut t);
    assert_eq!(o.commit.as_deref(), Some("la"));
    assert!(!o.consumed, "Return reaches the app after the insert");
    assert!(passed(&ret(&mut e, &mut t)));
}

#[test]
fn identifiers_and_paths() {
    let (mut e, _s) = en();
    let mut t = 0.0;
    let outs = type_str(&mut e, "user_name", &mut t);
    assert_eq!(commits(&outs), vec!["user_"]);
    assert_eq!(e.commit_pending(t + 1.0).commit.as_deref(), Some("name"));

    // `cd ~/Docs`: the tilde commits "cd " and passes; the slash then enters
    // raw mode from Idle and the rest goes straight to the app.
    let (mut e, s) = en();
    let mut t = 0.0;
    let outs = type_str(&mut e, "cd ~/Docs", &mut t);
    assert_eq!(commits(&outs), vec!["cd "]);
    assert!(!outs[3].consumed && outs[4..].iter().all(passed));
    assert_eq!(decisions(&s), 2 + 1, "c, d and the space only");
}

#[test]
fn digit_after_a_word_is_a_boundary() {
    let (mut e, _s) = en();
    let mut t = 0.0;
    let outs = type_str(&mut e, "abc12", &mut t);
    assert_eq!(outs[3].marked.as_deref(), Some("abc1"));
    assert_eq!(outs[4].commit.as_deref(), Some("abc1"));
    assert!(!outs[4].consumed);
}

#[test]
fn commit_keys_commit_composing_and_pending_then_pass() {
    type Case = (&'static str, u16, &'static str, u32);
    let cases: Vec<Case> = vec![
        ("return", keycode::RETURN, "\r", 0),
        ("shift-return", keycode::RETURN, "\r", Modifiers::SHIFT),
        ("cmd-return", keycode::RETURN, "\r", Modifiers::COMMAND),
        ("ctrl-return", keycode::RETURN, "\r", Modifiers::CONTROL),
        ("opt-return", keycode::RETURN, "\r", Modifiers::OPTION),
        ("tab", keycode::TAB, "\t", 0),
        ("shift-tab", keycode::TAB, "\u{19}", Modifiers::SHIFT),
        ("escape", keycode::ESCAPE, "\u{1b}", 0),
        ("left", keycode::LEFT, "\u{F702}", 0),
        ("up", keycode::UP, "\u{F700}", 0),
        ("home", 115, "\u{F729}", 0),
        ("page down", 121, "\u{F72D}", 0),
        ("forward delete", 117, "\u{F728}", 0),
        ("cmd-c", 8, "c", Modifiers::COMMAND),
        ("ctrl-c", 8, "\u{3}", Modifiers::CONTROL),
        ("opt-b", 11, "b", Modifiers::OPTION),
    ];
    for (name, code, chars, mods) in cases {
        for pending in [false, true] {
            let (mut e, s) = vi();
            let mut t = 0.0;
            type_str(&mut e, if pending { "has " } else { "has" }, &mut t);
            let o = key(&mut e, code, chars, mods, &mut t);
            let want = if pending { "h\u{e1} " } else { "h\u{e1}" };
            assert_eq!(o.commit.as_deref(), Some(want), "{name} pending={pending}");
            assert!(!o.consumed, "{name}: the key goes to the app");
            assert!(o.marked.is_none(), "{name}");
            let words = events(&s)
                .iter()
                .filter(|ev| matches!(ev, Event::WordCommit { .. }))
                .count();
            assert_eq!(words, 1, "{name}");
            // Idle now: nothing to commit, nothing to flip.
            assert_eq!(e.commit_pending(t + 1.0), EngineOutput::default());
        }
    }
}

#[test]
fn second_boundary_commits_and_passes_even_for_a_trigger() {
    let (mut e, _s) = en();
    let mut t = 0.0;
    let outs = type_str(&mut e, "http://x.com", &mut t);
    assert_eq!(outs[4].marked.as_deref(), Some("http:"));
    assert_eq!(outs[5].commit.as_deref(), Some("http:"));
    assert!(!outs[5].consumed, "the first slash goes to the app, Idle");
    // The second slash enters raw mode; the rest passes untouched.
    assert!(outs[6..].iter().all(passed));
    // Raw ends at a terminator; the next letter composes again.
    let o = ret(&mut e, &mut t);
    assert!(passed(&o));
    assert!(type_str(&mut e, "a", &mut t)[0].consumed);
}

#[test]
fn trigger_in_the_middle_of_a_word_is_a_boundary() {
    let (mut e, _s) = en();
    let mut t = 0.0;
    let outs = type_str(&mut e, "va/hoac", &mut t);
    assert_eq!(outs[2].marked.as_deref(), Some("va/"));
    assert_eq!(outs[3].commit.as_deref(), Some("va/"));
    assert_eq!(outs[3].marked.as_deref(), Some("h"));
    assert_eq!(outs[6].marked.as_deref(), Some("hoac"));
}

#[test]
fn raw_commands() {
    let raw_plain = |text: &str, ends_at: Option<usize>| {
        let (mut e, s) = en();
        let mut t = 0.0;
        let outs = type_str(&mut e, text, &mut t);
        let n = ends_at.unwrap_or(outs.len());
        assert!(outs[..n].iter().all(passed), "{text}: {outs:?}");
        assert_eq!(decisions(&s), 0, "{text}: raw keys reach no model");
        (e, outs, s)
    };
    // `/help` then Return.
    let (mut e, _, _) = raw_plain("/help", None);
    let mut t = 10.0;
    assert!(passed(&ret(&mut e, &mut t)));
    assert!(type_str(&mut e, "a", &mut t)[0].consumed, "Idle again");

    // `@src/main.rs` is one raw run (the slash is not a terminator).
    raw_plain("@src/main.rs", None);

    // `/model gru`: raw ends at the space, "gru" is an ordinary word.
    let (mut e, outs, _) = raw_plain("/model ", None);
    assert!(passed(outs.last().unwrap()));
    let mut t = 10.0;
    let outs = type_str(&mut e, "gru", &mut t);
    assert_eq!(outs[2].marked.as_deref(), Some("gru"));
    assert_eq!(e.commit_pending(t + 1.0).commit.as_deref(), Some("gru"));

    // `!ls -la`: raw until the first space, then `-` is an Idle symbol and
    // "la" a word.
    let (mut e, _, _) = raw_plain("!ls ", None);
    let mut t = 10.0;
    let outs = type_str(&mut e, "-la", &mut t);
    assert!(passed(&outs[0]));
    assert_eq!(outs[2].marked.as_deref(), Some("la"));

    // `/ ` leaves raw mode at once, with no effect at all.
    let (mut e, _, _) = raw_plain("/ ", None);
    let mut t = 10.0;
    assert!(type_str(&mut e, "a", &mut t)[0].consumed);
}

#[test]
fn raw_mode_ends_on_terminators_chords_navigation_and_focus() {
    type Case = (&'static str, u16, &'static str, u32);
    let cases: Vec<Case> = vec![
        ("tab", keycode::TAB, "\t", 0),
        ("escape", keycode::ESCAPE, "\u{1b}", 0),
        ("left", keycode::LEFT, "\u{F702}", 0),
        ("cmd-v", 9, "v", Modifiers::COMMAND),
    ];
    for (name, code, chars, mods) in cases {
        let (mut e, _s) = en();
        let mut t = 0.0;
        type_str(&mut e, "/ab", &mut t);
        let o = key(&mut e, code, chars, mods, &mut t);
        assert!(passed(&o), "{name}");
        assert!(type_str(&mut e, "a", &mut t)[0].consumed, "{name}: Idle");
    }
    // A mouse click and a focus change end it too.
    let (mut e, _s) = en();
    let mut t = 0.0;
    type_str(&mut e, "/ab", &mut t);
    e.note_navigation();
    assert!(type_str(&mut e, "a", &mut t)[0].consumed);
    e.commit_pending(t + 1.0);
    type_str(&mut e, "/ab", &mut t);
    e.on_context(&ctx("other", 2, t + 1.0));
    assert!(type_str(&mut e, "a", &mut t)[0].consumed);
}

#[test]
fn backspace_in_raw_passes_and_stays_raw() {
    let (mut e, _s) = en();
    let mut t = 0.0;
    type_str(&mut e, "/ab", &mut t);
    let o = key(&mut e, keycode::BACKSPACE, "\u{7f}", 0, &mut t);
    assert!(passed(&o));
    assert!(passed(&type_str(&mut e, "c", &mut t)[0]), "still raw");
}

#[test]
fn double_shift_in_raw_flips_the_sticky_mode_without_a_label() {
    let (mut e, s) = en();
    e.on_context(&ctx("app", 1, 1.0));
    let mut t = 1.0;
    type_str(&mut e, "/ab", &mut t);
    let before = e.mode();
    let o = double_shift(&mut e, &mut t);
    assert!(o.consumed);
    assert_eq!(o.mode, Some(before.flip()));
    assert_eq!(o.overlay, Some(Overlay::Manual));
    assert_eq!(o.marked, None, "nothing is marked in raw mode");
    assert!(passed(&type_str(&mut e, "c", &mut t)[0]), "still raw");
    let evs = events(&s);
    assert!(
        !evs.iter().any(|ev| matches!(ev, Event::Label { .. })),
        "no label"
    );
    assert!(evs.iter().any(|ev| matches!(
        ev,
        Event::ManualSwitch {
            scope: SwitchScope::AfterCommit,
            word_id: None,
            ..
        }
    )));
}

#[test]
fn double_shift_in_each_state() {
    // Composing: flips the word and locks it.
    let (mut e, _s) = en();
    let mut t = 0.0;
    type_str(&mut e, "has", &mut t);
    assert_eq!(
        double_shift(&mut e, &mut t).marked.as_deref(),
        Some("h\u{e1}")
    );
    // Pending: flips the pending word, boundary kept.
    let (mut e, _s) = en();
    let mut t = 0.0;
    type_str(&mut e, "has ", &mut t);
    assert_eq!(
        double_shift(&mut e, &mut t).marked.as_deref(),
        Some("h\u{e1} ")
    );
    // Idle before typing: flips the sticky mode, labelling the focus prior.
    let (mut e, s) = en();
    e.on_context(&ctx("app", 1, 1.0));
    let mut t = 1.0;
    let o = double_shift(&mut e, &mut t);
    assert_eq!(o.marked, None);
    assert_eq!(o.mode, Some(e.mode()));
    assert!(events(&s).iter().any(|ev| matches!(
        ev,
        Event::ManualSwitch {
            scope: SwitchScope::BeforeTyping,
            ..
        }
    )));
}

#[test]
fn backspace_walks_from_pending_back_into_the_word() {
    let (mut e, s) = vi();
    let mut t = 0.0;
    type_str(&mut e, "tieengs.", &mut t);
    let o = key(&mut e, keycode::BACKSPACE, "\u{7f}", 0, &mut t);
    assert!(o.consumed);
    assert_eq!(o.marked.as_deref(), Some("ti\u{1ebf}ng"));
    let o = key(&mut e, keycode::BACKSPACE, "\u{7f}", 0, &mut t);
    assert_eq!(o.marked.as_deref(), Some("ti\u{ea}ng"));
    let outs = type_str(&mut e, "s ", &mut t);
    assert!(commits(&outs).is_empty());
    assert_eq!(outs[1].marked.as_deref(), Some("ti\u{1ebf}ng "));
    // Backspace on the last letter empties the word: Idle.
    type_str(&mut e, "\u{8}", &mut t);
    let ids: Vec<_> = events(&s)
        .into_iter()
        .filter_map(|ev| match ev {
            Event::WordCommit { word_id, .. } => Some(word_id),
            _ => None,
        })
        .collect();
    assert!(ids.is_empty(), "the word was never committed twice");
}

#[test]
fn model_switch_and_focus_change_commit_both_states() {
    for text in ["has", "has,"] {
        let (mut e, _s) = vi();
        e.on_context(&ctx("a", 1, 1.0));
        let mut t = 1.0;
        type_str(&mut e, text, &mut t);
        let o = e.switch_model(nimk_engine::ModelKind::GruAction, t + 1.0);
        assert_eq!(
            o.commit.as_deref(),
            Some(if text == "has" { "h\u{e1}" } else { "h\u{e1}," })
        );
    }
}

#[test]
fn word_ids_do_not_depend_on_the_mode() {
    let text = "tieengs, has /cmd a_b 1x\u{8} http://x.com Vieetj ";
    let run = |p: f32| {
        let (mut e, storage) = const_engine(p, inline_cfg());
        let mut t = 0.0;
        for c in text.chars() {
            t += 0.1;
            if c == '\u{8}' {
                e.on_key(&code_key(keycode::BACKSPACE, "\u{7f}", 0, t));
            } else {
                e.on_key(&char_key(c, t));
            }
        }
        e.commit_pending(t + 1.0);
        events(&storage)
            .into_iter()
            .filter_map(|ev| match ev {
                Event::WordCommit { word_id, raw, .. } => Some((word_id, raw)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(0.9), run(0.1));
}

#[test]
fn logs_are_deterministic() {
    let run = || {
        let (mut e, storage) = const_engine(0.9, EngineConfig { ..inline_cfg() });
        e.on_context(&ctx("a", 1, 1.0));
        let mut t = 1.0;
        type_str(&mut e, "tieengs, /help Vieetj. ", &mut t);
        double_shift(&mut e, &mut t);
        ret(&mut e, &mut t);
        e.shutdown().unwrap();
        log_bytes(&storage)
    };
    assert_eq!(run(), run());
}

#[test]
fn random_events_never_panic_or_lose_text() {
    let (mut e, _s) = const_engine(0.5, inline_cfg());
    let mut seed = 0x1234_5678_9abc_def0_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let alphabet: Vec<&str> = vec![
        "a", "b", "s", "w", "x", " ", ",", ".", "/", "@", "!", "_", "-", "~", "1", "9", "\"",
    ];
    let mut t = 0.0;
    for i in 0..20_000_u32 {
        t += 0.05;
        match next() % 14 {
            0 => {
                e.on_key(&code_key(keycode::BACKSPACE, "\u{7f}", 0, t));
            }
            1 => {
                e.on_key(&code_key(
                    keycode::RETURN,
                    "\r",
                    next() as u32 % 2 * Modifiers::SHIFT,
                    t,
                ));
            }
            2 => {
                e.on_key(&code_key(keycode::TAB, "\t", 0, t));
            }
            3 => {
                e.on_key(&code_key(8, "c", Modifiers::COMMAND, t));
            }
            4 => {
                e.on_key(&code_key(keycode::LEFT, "\u{F702}", 0, t));
            }
            5 if i % 7 == 0 => {
                e.commit_pending(t);
            }
            6 if i % 11 == 0 => {
                e.on_context(&ctx("app", u64::from(next() as u8 % 3), t));
            }
            7 if i % 13 == 0 => {
                double_shift(&mut e, &mut t);
            }
            _ => {
                let c = alphabet[(next() % alphabet.len() as u64) as usize];
                e.on_key(&code_key(0, c, 0, t));
            }
        }
    }
    let diags = e.take_diagnostics();
    assert!(!diags.iter().any(|d| d.contains("panic")), "{diags:?}");
    e.commit_pending(t + 1.0);
    assert_eq!(e.commit_pending(t + 2.0), EngineOutput::default());
}
