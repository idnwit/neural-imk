//! UX contract with FRESH (untrained) models of every kind and the default
//! config: Vietnamese Telex works out of the box, English survives VI mode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::*;
use nimk_engine::{Engine, EngineConfig};
use nimk_policy::PolicyKind;
use nimk_storage::MemStorage;
use nimk_types::{Event, Lang, Target};

const KINDS: [PolicyKind; 3] = [
    PolicyKind::Baseline,
    PolicyKind::Linear,
    PolicyKind::TinyMlp,
];

fn fresh(kind: PolicyKind) -> (Engine, MemStorage) {
    let storage = MemStorage::new();
    let cfg = EngineConfig {
        policy: kind,
        ..EngineConfig::default()
    };
    let engine = Engine::new(cfg, Arc::new(storage.clone()), session());
    (engine, storage)
}

#[test]
fn fresh_models_type_vietnamese() {
    for kind in KINDS {
        let (mut e, _s) = fresh(kind);
        assert_eq!(e.mode(), Lang::Vi, "{kind:?}: VI prior");
        let mut t = 1.0;
        let outs = type_str(&mut e, "tieengs Vieetj ", &mut t);
        assert_eq!(commits(&outs), vec!["tiếng", "Việt"], "{kind:?}");
        e.shutdown().unwrap();
    }
}

#[test]
fn fresh_models_with_focus_type_vietnamese() {
    for kind in KINDS {
        let (mut e, _s) = fresh(kind);
        e.on_context(&ctx("com.apple.TextEdit", 3, 1.0));
        let mut t = 1.0;
        let outs = type_str(&mut e, "tieengs Vieetj dduwowngf ", &mut t);
        assert_eq!(commits(&outs), vec!["tiếng", "Việt", "đường"], "{kind:?}");
        e.shutdown().unwrap();
    }
}

#[test]
fn english_words_survive_vi_mode() {
    for kind in KINDS {
        for word in ["string", "class", "the"] {
            let (mut e, _s) = fresh(kind);
            let mut t = 1.0;
            let outs = type_str(&mut e, &format!("{word} "), &mut t);
            assert_eq!(commits(&outs), vec![word.to_owned()], "{kind:?}");
            e.shutdown().unwrap();
        }
    }
}

#[test]
fn ambiguous_test_is_decided() {
    for kind in KINDS {
        let storage = MemStorage::new();
        let cfg = EngineConfig {
            policy: kind,
            ..inline_cfg()
        };
        let mut e = Engine::new(cfg, Arc::new(storage.clone()), session());
        let mut t = 1.0;
        let outs = type_str(&mut e, "test ", &mut t);
        let c = commits(&outs);
        assert!(c == ["tét"] || c == ["test"], "{kind:?}: {c:?}");
        let decided = events(&storage).iter().any(|ev| {
            matches!(
                ev,
                Event::Decision {
                    target: Target::Word(0),
                    ..
                }
            )
        });
        assert!(decided, "{kind:?}");
        assert!(!e.take_diagnostics().iter().any(|d| d.contains("panic")));
        e.shutdown().unwrap();
    }
}

#[test]
fn english_word_in_vi_sentence_is_word_scoped() {
    for kind in KINDS {
        let (mut e, _s) = fresh(kind);
        let mut t = 1.0;
        let outs = type_str(&mut e, "Tooi dungf class nayf ", &mut t);
        assert_eq!(
            commits(&outs),
            vec!["Tôi", "dùng", "class", "này"],
            "{kind:?}"
        );
        assert!(
            outs.iter().all(|o| o.overlay.is_none() && o.mode.is_none()),
            "{kind:?}: no sticky change"
        );
        assert_eq!(e.mode(), Lang::Vi, "{kind:?}");
        e.shutdown().unwrap();
    }
}

#[test]
fn manual_switch_to_en_keeps_telex_words_raw() {
    for kind in KINDS {
        let (mut e, _s) = fresh(kind);
        let mut t = 1.0;
        let o = shift_tap(&mut e, &mut t);
        assert_eq!(o.mode, Some(Lang::En), "{kind:?}");
        let outs = type_str(&mut e, "tieengs ", &mut t);
        assert_eq!(commits(&outs), vec!["tieengs"], "{kind:?}");
        assert_eq!(e.mode(), Lang::En, "{kind:?}");
        e.shutdown().unwrap();
    }
}
