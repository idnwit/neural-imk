//! Smoke tests with FRESH (untrained, neutral) real models and the default
//! config: no panics, one decision per model call, keys are never lost.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::*;
use nimk_engine::{Engine, EngineConfig};
use nimk_policy::ModelKind;
use nimk_storage::MemStorage;
use nimk_types::{Event, KeyKind, Lang};

fn fresh(kind: ModelKind) -> (Engine, MemStorage) {
    let storage = MemStorage::new();
    let cfg = EngineConfig {
        policy: kind,
        ..inline_cfg()
    };
    let engine = Engine::new(cfg, Arc::new(storage.clone()), session());
    (engine, storage)
}

fn smoke(kind: ModelKind) {
    let (mut e, storage) = fresh(kind);
    assert_eq!(e.mode(), Lang::Vi);
    e.on_context(&ctx("com.apple.TextEdit", 3, 1.0));
    let mut t = 1.0;
    let text = "tieengs Vieetj, has string ";
    let mut outs = type_str(&mut e, text, &mut t);
    outs.push(double_shift(&mut e, &mut t));
    outs.extend(type_str(&mut e, "dduwowngf ", &mut t));
    outs.push(e.commit_pending(t + 1.0));
    // Every character typed either was consumed into the buffer or went to
    // the app: nothing may be dropped silently (consumed => marked text).
    for o in &outs {
        if o.consumed && o.mode.is_none() {
            assert!(
                o.marked.is_some(),
                "{kind:?}: consumed key without marked text"
            );
        }
    }
    let evs = events(&storage);
    let keys = evs
        .iter()
        .filter(
            |ev| matches!(ev, Event::Key { key, word_id: Some(_) } if key.kind == KeyKind::KeyDown),
        )
        .count();
    let decisions = evs
        .iter()
        .filter(|ev| {
            matches!(
                ev,
                Event::Decision {
                    target: nimk_types::Target::Word(_),
                    ..
                }
            )
        })
        .count();
    assert_eq!(keys, decisions, "{kind:?}: one decision per model call");
    let diags = e.take_diagnostics();
    assert!(
        !diags.iter().any(|d| d.contains("panic")),
        "{kind:?}: {diags:?}"
    );
    e.shutdown().unwrap();
}

#[test]
fn fresh_linear_action_smoke() {
    smoke(ModelKind::LinearAction);
}

#[test]
fn fresh_gru_action_smoke() {
    smoke(ModelKind::GruAction);
}

#[test]
fn fresh_models_decide_deterministically() {
    for kind in [ModelKind::LinearAction, ModelKind::GruAction] {
        let run = || {
            let (mut e, storage) = fresh(kind);
            let mut t = 1.0;
            let outs = type_str(&mut e, "tieengs ab ", &mut t);
            e.shutdown().unwrap();
            let decisions: Vec<_> = events(&storage)
                .into_iter()
                .filter_map(|ev| match ev {
                    Event::Decision { p_vi, chosen, .. } => Some((p_vi.map(f32::to_bits), chosen)),
                    _ => None,
                })
                .collect();
            (outs, decisions)
        };
        assert_eq!(run(), run(), "{kind:?}");
    }
}
