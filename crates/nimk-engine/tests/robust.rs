//! Determinism, persistence and robustness with the real policies.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::io;
use std::sync::Arc;

use common::*;
use nimk_engine::{Engine, EngineConfig, Threading};
use nimk_policy::PolicyKind;
use nimk_storage::{BlobStore, EventSink, EventSource, MemStorage, Storage};
use nimk_types::{Event, KeyEvent, KeyKind, LabelSource, Lang, Modifiers, Target, keycode};

/// A short session with words, punctuation, hotkeys and focus changes.
fn scripted(e: &mut Engine) {
    let mut t = 10.0;
    e.on_context(&ctx("com.apple.TextEdit", 1, t));
    type_str(e, "tieengs Vieetj has ", &mut t);
    shift_tap(e, &mut t);
    type_str(e, "for test, ", &mut t);
    type_str(e, "dduwowngf", &mut t);
    shift_tap(e, &mut t);
    type_str(e, " ok. ", &mut t);
    e.on_context(&ctx("com.microsoft.VSCode", 2, t + 1.0));
    t += 1.0;
    shift_tap(e, &mut t);
    type_str(e, "deploy commit ", &mut t);
    e.commit_pending(t + 20.0);
}

fn run_scripted(kind: PolicyKind) -> Vec<Vec<u8>> {
    let storage = MemStorage::new();
    let cfg = EngineConfig {
        policy: kind,
        ..inline_cfg()
    };
    let mut e = Engine::new(cfg, Arc::new(storage.clone()), session());
    scripted(&mut e);
    e.shutdown().unwrap();
    log_bytes(&storage)
}

#[test]
fn replay_is_byte_identical() {
    for kind in [
        PolicyKind::Baseline,
        PolicyKind::Linear,
        PolicyKind::TinyMlp,
    ] {
        let a = run_scripted(kind);
        let b = run_scripted(kind);
        assert!(a.len() > 50, "{kind:?}: {} records", a.len());
        assert_eq!(a, b, "{kind:?}");
    }
}

#[test]
fn log_records_are_sequenced() {
    let storage = MemStorage::new();
    let mut e = Engine::new(inline_cfg(), Arc::new(storage.clone()), session());
    scripted(&mut e);
    e.shutdown().unwrap();
    let recs: Vec<nimk_types::Record> = log_bytes(&storage)
        .iter()
        .map(|b| serde_json::from_slice(b).unwrap())
        .collect();
    for (i, r) in recs.iter().enumerate() {
        assert_eq!(r.seq, i as u64);
        assert_eq!(r.session, "test-session");
    }
    match &recs[0].event {
        Event::SessionStart { config_json, .. } => {
            let cfg: EngineConfig = serde_json::from_str(config_json).unwrap();
            assert_eq!(cfg.threading, Threading::Inline);
        }
        other => panic!("first record is {other:?}"),
    }
    assert!(recs.iter().any(|r| matches!(
        r.event,
        Event::Label {
            source: LabelSource::Correction,
            ..
        }
    )));
    assert!(recs.iter().any(|r| matches!(
        r.event,
        Event::Decision {
            target: Target::Focus(1),
            ..
        }
    )));
}

#[test]
fn snapshot_round_trip_and_corrupt_fallback() {
    let storage = MemStorage::new();
    let shared: Arc<dyn Storage> = Arc::new(storage.clone());
    let mut e = Engine::new(inline_cfg(), Arc::clone(&shared), session());
    assert!(e.take_diagnostics().is_empty());
    scripted(&mut e);
    e.shutdown().unwrap();
    let saved = storage
        .blobs()
        .get("model/tiny_mlp/latest")
        .unwrap()
        .unwrap();
    let fresh = {
        let other = MemStorage::new();
        Engine::new(inline_cfg(), Arc::new(other.clone()), session())
            .shutdown()
            .unwrap();
        other.blobs().get("model/tiny_mlp/latest").unwrap().unwrap()
    };
    assert_ne!(saved, fresh, "training changed the weights");

    // Reload without new samples: saving again yields the same weights.
    let mut e2 = Engine::new(inline_cfg(), Arc::clone(&shared), session());
    assert!(e2.take_diagnostics().is_empty());
    e2.shutdown().unwrap();
    assert_eq!(
        storage
            .blobs()
            .get("model/tiny_mlp/latest")
            .unwrap()
            .unwrap(),
        saved,
        "restored weights"
    );

    storage
        .blobs()
        .put("model/tiny_mlp/latest", b"garbage")
        .unwrap();
    let mut e3 = Engine::new(inline_cfg(), shared, session());
    let d = e3.take_diagnostics();
    assert!(d.iter().any(|m| m.contains("starting fresh")), "{d:?}");
}

/// Storage whose event sink cannot be opened.
struct NoSink(MemStorage);

impl Storage for NoSink {
    fn event_sink(&self, _session: &str) -> io::Result<Box<dyn EventSink>> {
        Err(io::Error::other("disk full"))
    }
    fn event_source(&self) -> io::Result<Box<dyn EventSource + '_>> {
        self.0.event_source()
    }
    fn blobs(&self) -> &dyn BlobStore {
        self.0.blobs()
    }
}

#[test]
fn missing_sink_falls_back_to_null() {
    let mut e = Engine::new(inline_cfg(), Arc::new(NoSink(MemStorage::new())), session());
    let d = e.take_diagnostics();
    assert!(
        d.iter().any(|m| m.contains("event sink unavailable")),
        "{d:?}"
    );
    let mut t = 0.0;
    assert_eq!(commits(&type_str(&mut e, "ab ", &mut t)), vec!["ab"]);
    e.shutdown().unwrap();
}

#[test]
fn background_threading_works() {
    let storage = MemStorage::new();
    let cfg = EngineConfig {
        threading: Threading::Background,
        ..EngineConfig::default()
    };
    let mut e = Engine::new(cfg, Arc::new(storage.clone()), session());
    scripted(&mut e);
    e.shutdown().unwrap();
    let evs = events(&storage);
    assert!(matches!(evs.first(), Some(Event::SessionStart { .. })));
    assert!(evs.iter().any(|e| matches!(e, Event::ModelUpdate { .. })));
    assert!(
        storage
            .blobs()
            .get("model/tiny_mlp/latest")
            .unwrap()
            .is_some()
    );
}

/// splitmix64 for reproducible garbage.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn garbage_never_panics() {
    const CHARS: &[&str] = &[
        "a", "e", "o", "w", "d", "s", "f", "r", "x", "j", "z", "A", "W", " ", ",", "1", "é",
        "\u{7f}", "\r", "\u{F702}", "", "ab", "\u{0}", "đ", "🙂",
    ];
    const CODES: &[u16] = &[
        0,
        1,
        2,
        13,
        keycode::SPACE,
        keycode::RETURN,
        keycode::TAB,
        keycode::BACKSPACE,
        keycode::ESCAPE,
        keycode::LEFT_SHIFT,
        keycode::RIGHT_SHIFT,
        keycode::LEFT,
        keycode::UP,
        55,
        58,
        59,
        117,
        u16::MAX,
    ];
    for kind in [PolicyKind::Baseline, PolicyKind::TinyMlp] {
        for hotkeys in [true, false] {
            let storage = MemStorage::new();
            let cfg = EngineConfig {
                policy: kind,
                hotkeys,
                ..inline_cfg()
            };
            let mut e = Engine::new(cfg, Arc::new(storage), session());
            let mut rng = Rng(0xBAD5EED ^ u64::from(hotkeys));
            let mut t = 0.0f64;
            for i in 0..5_000u32 {
                t += match rng.below(50) {
                    0 => -5.0,
                    1 => 30.0,
                    2 => f64::NAN,
                    _ => 0.05,
                };
                if !t.is_finite() {
                    t = f64::from(i);
                }
                match rng.below(40) {
                    0 => {
                        e.on_context(&ctx(["a", "b", ""][rng.below(3) as usize], rng.below(3), t));
                    }
                    1 => {
                        e.commit_pending(t);
                    }
                    2 => {
                        let target = if rng.below(2) == 0 {
                            Target::Word(rng.below(100))
                        } else {
                            Target::Focus(rng.below(5))
                        };
                        let label = if rng.below(2) == 0 {
                            Lang::Vi
                        } else {
                            Lang::En
                        };
                        e.inject_label(target, label, 1.0, LabelSource::Correction);
                    }
                    _ => {
                        let kind = if rng.below(4) == 0 {
                            KeyKind::FlagsChanged
                        } else {
                            KeyKind::KeyDown
                        };
                        let key = KeyEvent {
                            kind,
                            keycode: CODES[rng.below(CODES.len() as u64) as usize],
                            chars: CHARS[rng.below(CHARS.len() as u64) as usize].to_owned(),
                            mods: Modifiers((rng.next() as u32) & (rng.next() as u32)),
                            timestamp: t,
                        };
                        let out = e.on_key(&key);
                        if let Some(m) = &out.marked {
                            assert!(m.chars().count() <= 64);
                        }
                    }
                }
            }
            let d = e.take_diagnostics();
            assert!(!d.iter().any(|m| m.contains("panic")), "{kind:?}: {d:?}");
            e.shutdown().unwrap();
        }
    }
}
