//! Teacher forcing (`Engine::teach`) for behaviour-cloning pre-training.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use nimk_engine::EngineOutput;
use nimk_types::{Action, EditAction, Event, KeyEvent, Lang, Modifiers, Observation, keycode};

const TEXT: &str = "tieengs Vieetj, abx\u{7f}\u{7f} c ";

/// The edit a typist wants for `key` with `len` keys buffered.
fn want(key: &KeyEvent, len: usize) -> EditAction {
    let letter = key.chars.chars().next().is_some_and(char::is_alphabetic);
    if key.keycode == keycode::BACKSPACE {
        if len > 0 {
            EditAction::Pop
        } else {
            EditAction::Pass
        }
    } else if letter {
        EditAction::Append
    } else if len > 0 {
        EditAction::Commit
    } else {
        EditAction::Pass
    }
}

fn keys() -> Vec<KeyEvent> {
    TEXT.chars()
        .enumerate()
        .map(|(i, c)| {
            let t = 1.0 + i as f64 * 0.1;
            if c == '\u{7f}' {
                code_key(keycode::BACKSPACE, "\u{7f}", 0, t)
            } else {
                char_key(c, t)
            }
        })
        .collect()
}

#[test]
fn teach_matches_on_key_and_queues_examples() {
    let seen_obs: Arc<Mutex<Vec<Observation>>> = Arc::default();
    let sink = Arc::clone(&seen_obs);
    let inner = const_typist(0.9);
    let spy: Arc<ScoreFn> = Arc::new(move |o: &Observation| {
        sink.lock().unwrap().push(o.clone());
        inner(o)
    });
    let (mut a, sa, _) = scripted_engine(spy, inline_cfg());

    // The teaching engine's own model must never be consulted.
    let panicky: Arc<ScoreFn> = Arc::new(|_o: &Observation| unreachable!("teach must not score"));
    let (mut b, sb, seen_b) = scripted_engine(panicky, inline_cfg());

    let mut len = 0usize;
    let mut outs_a: Vec<EngineOutput> = Vec::new();
    let mut outs_b: Vec<EngineOutput> = Vec::new();
    for k in keys() {
        outs_a.push(a.on_key(&k));
        let edit = want(&k, len);
        len = match edit {
            EditAction::Append => len + 1,
            EditAction::Pop => len - 1,
            EditAction::Commit => 0,
            EditAction::Pass => len,
        };
        let target = Action {
            edit,
            render: Lang::Vi,
        };
        outs_b.push(b.teach(&k, target, 1.0, 0.5));
    }
    assert_eq!(outs_a, outs_b, "identical buffer behaviour");

    let seen_b = seen_b.lock().unwrap();
    let obs_a = seen_obs.lock().unwrap();
    // Keys handled by the pending word (the second boundary character of
    // "Vieetj, ") never reach the model, so they leave no example.
    assert_eq!(seen_b.len(), keys().len() - 1, "one example per model call");
    assert_eq!(obs_a.len(), seen_b.len());
    for (x, o) in seen_b.iter().zip(obs_a.iter()) {
        assert_eq!(&x.obs, o, "observation built exactly as on_key does");
        assert!((x.edit_weight - 1.0).abs() < 1e-6);
        assert!((x.render_weight - 0.5).abs() < 1e-6);
        assert_eq!(x.target.render, Lang::Vi);
    }
    assert_eq!(seen_b[0].target.edit, EditAction::Append);
    assert_eq!(seen_b[7].target.edit, EditAction::Commit);

    // Same log shape (Key / Decision / WordCommit), p_vi only differs.
    let shape = |s: &nimk_storage::MemStorage| {
        events(s)
            .into_iter()
            .map(|e| match e {
                Event::Decision {
                    target,
                    at_key,
                    chosen,
                    ..
                } => format!("D {target:?} {at_key} {chosen:?}"),
                Event::Key { word_id, .. } => format!("K {word_id:?}"),
                Event::WordCommit { word_id, raw, .. } => format!("W {word_id} {raw}"),
                _ => String::new(),
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
    };
    assert_eq!(shape(&sa), shape(&sb));
    assert!(events(&sb).iter().any(|e| matches!(
        e,
        Event::Decision {
            p_vi: None,
            rule: None,
            ..
        }
    )));
}

#[test]
fn teach_render_and_mask_fallback() {
    let (mut e, _s, seen) = scripted_engine(const_typist(0.9), inline_cfg());
    let append = |render| Action {
        edit: EditAction::Append,
        render,
    };
    // Render comes from the target, not the model (which says Vi).
    let o = e.teach(&char_key('a', 1.0), append(Lang::En), 1.0, 1.0);
    assert_eq!(o.marked.as_deref(), Some("a"));
    let o = e.teach(&char_key('s', 1.1), append(Lang::En), 1.0, 1.0);
    assert_eq!(o.marked.as_deref(), Some("as"));
    let o = e.teach(&char_key('s', 1.2), append(Lang::Vi), 1.0, 1.0);
    assert_eq!(o.marked.as_deref(), Some("as"), "ass -> as");
    // Append on Cmd+A is masked: Pass, and the example records Pass.
    let o = e.teach(
        &code_key(0, "a", Modifiers::COMMAND, 1.3),
        append(Lang::Vi),
        1.0,
        1.0,
    );
    assert!(!o.consumed);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 4);
    assert_eq!(seen[3].target.edit, EditAction::Pass);
    assert_eq!(seen[1].target.render, Lang::En);
}

#[test]
fn teach_words_still_take_injected_labels() {
    let cfg = nimk_engine::EngineConfig {
        hotkeys: false,
        ..inline_cfg()
    };
    let (mut e, storage, _) = scripted_engine(const_typist(0.9), cfg);
    let a = Action {
        edit: EditAction::Append,
        render: Lang::Vi,
    };
    let c = Action {
        edit: EditAction::Commit,
        render: Lang::Vi,
    };
    e.teach(&char_key('a', 1.0), a, 0.0, 1.0);
    let o = e.teach(&char_key(' ', 1.1), c, 0.0, 1.0);
    assert_eq!(o.marked.as_deref(), Some("a "));
    // The next word's first key commits the pending one.
    let o = e.teach(&char_key('b', 1.2), a, 0.0, 1.0);
    assert_eq!(o.commit.as_deref(), Some("a "));
    e.inject_label(
        nimk_types::Target::Word(0),
        Lang::En,
        1.0,
        nimk_types::LabelSource::Correction,
    );
    assert!(e.take_diagnostics().is_empty());
    assert!(
        events(&storage)
            .iter()
            .any(|ev| matches!(ev, Event::Label { .. }))
    );
}

#[test]
fn teach_batches_examples_and_flushes_at_shutdown() {
    let cfg = nimk_engine::EngineConfig {
        teach_batch: 3,
        ..inline_cfg()
    };
    let (mut e, _s, seen) = scripted_engine(const_typist(0.9), cfg);
    let a = Action {
        edit: EditAction::Append,
        render: Lang::Vi,
    };
    for (i, c) in "abcd".chars().enumerate() {
        e.teach(&char_key(c, 1.0 + i as f64 * 0.1), a, 1.0, 1.0);
        let want = if i < 2 { 0 } else { 3 };
        assert_eq!(seen.lock().unwrap().len(), want, "after key {i}");
    }
    e.shutdown().unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        4,
        "remainder trained at shutdown"
    );
}
