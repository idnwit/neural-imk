//! Determinism, persistence and robustness.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::hash::{DefaultHasher, Hash, Hasher};
use std::io;
use std::sync::Arc;

use common::*;
use nimk_engine::{Engine, EngineConfig, Threading};
use nimk_policy::ModelKind;
use nimk_storage::{BlobStore, EventSink, EventSource, MemStorage, Storage};
use nimk_types::{
    ActionScores, Event, KeyEvent, KeyKind, LabelSource, Lang, Modifiers, Observation, Target,
    keycode,
};

/// A short session with words, punctuation, hotkeys and focus changes.
fn scripted(e: &mut Engine) {
    let mut t = 10.0;
    e.on_context(&ctx("com.apple.TextEdit", 1, t));
    type_str(e, "tieengs Vieetj has ", &mut t);
    double_shift(e, &mut t);
    type_str(e, "for test, ", &mut t);
    type_str(e, "dduwowngf", &mut t);
    double_shift(e, &mut t);
    type_str(e, " ok. ", &mut t);
    e.on_context(&ctx("com.microsoft.VSCode", 2, t + 1.0));
    t += 1.0;
    double_shift(e, &mut t);
    type_str(e, "deploy commit ", &mut t);
    e.commit_pending(t + 20.0);
}

fn run_scripted(kind: ModelKind) -> Vec<Vec<u8>> {
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
    for kind in [ModelKind::LinearAction, ModelKind::GruAction] {
        let a = run_scripted(kind);
        let b = run_scripted(kind);
        assert!(a.len() > 50, "{kind:?}: {} records", a.len());
        assert_eq!(a, b, "{kind:?}");
    }
}

#[test]
fn log_records_are_sequenced() {
    let storage = MemStorage::new();
    let cfg = EngineConfig {
        policy: ModelKind::LinearAction,
        ..inline_cfg()
    };
    let mut e = Engine::new(cfg, Arc::new(storage.clone()), session());
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
            assert_eq!(cfg.policy, ModelKind::LinearAction);
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
    let key = "model/linear_action/latest";
    let cfg = || EngineConfig {
        policy: ModelKind::LinearAction,
        ..inline_cfg()
    };
    let storage = MemStorage::new();
    let shared: Arc<dyn Storage> = Arc::new(storage.clone());
    let mut e = Engine::new(cfg(), Arc::clone(&shared), session());
    assert!(e.take_diagnostics().is_empty());
    scripted(&mut e);
    e.shutdown().unwrap();
    let saved = storage.blobs().get(key).unwrap().unwrap();
    let fresh = {
        let other = MemStorage::new();
        Engine::new(cfg(), Arc::new(other.clone()), session())
            .shutdown()
            .unwrap();
        other.blobs().get(key).unwrap().unwrap()
    };
    assert_ne!(saved, fresh, "training changed the weights");

    let mut e2 = Engine::new(cfg(), Arc::clone(&shared), session());
    assert!(e2.take_diagnostics().is_empty());
    e2.shutdown().unwrap();
    assert_eq!(
        storage.blobs().get(key).unwrap().unwrap(),
        saved,
        "restored weights"
    );

    storage.blobs().put(key, b"garbage").unwrap();
    let mut e3 = Engine::new(cfg(), shared, session());
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
    let learner = FakeLearner {
        script: const_typist(0.9),
        seen: Seen::default(),
    };
    let mut e = Engine::with_model(
        inline_cfg(),
        Arc::new(NoSink(MemStorage::new())),
        session(),
        Box::new(learner),
    );
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
        policy: ModelKind::LinearAction,
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
            .get("model/linear_action/latest")
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

/// Deterministic pseudo-random model: any edit distribution, any p(VI),
/// sometimes non-finite, to exercise every action on every observation.
fn chaotic() -> Arc<ScoreFn> {
    Arc::new(|o: &Observation| {
        let mut h = DefaultHasher::new();
        o.raw.hash(&mut h);
        o.key
            .as_ref()
            .map(|k| (k.keycode, k.chars.clone()))
            .hash(&mut h);
        let mut r = Rng(h.finish());
        let mut edit = [0.0f32; 4];
        for e in &mut edit {
            *e = (r.below(1000) as f32) / 1000.0;
        }
        let p_vi = match r.below(20) {
            0 => f32::NAN,
            1 => f32::INFINITY,
            _ => (r.below(1001) as f32) / 1000.0,
        };
        ActionScores { edit, p_vi }
    })
}

fn garbage(mut e: Engine, seed: u64, label: &str) {
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
    let mut rng = Rng(seed);
    let mut t = 0.0f64;
    for i in 0..20_000u32 {
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
                let lang = if rng.below(2) == 0 {
                    Lang::Vi
                } else {
                    Lang::En
                };
                e.inject_label(target, lang, 1.0, LabelSource::Correction);
            }
            _ => {
                let kind = if rng.below(3) == 0 {
                    KeyKind::FlagsChanged
                } else {
                    KeyKind::KeyDown
                };
                let mods = if kind == KeyKind::FlagsChanged && rng.below(2) == 0 {
                    // Make double Shift taps happen now and then.
                    Modifiers(if rng.below(2) == 0 {
                        Modifiers::SHIFT
                    } else {
                        0
                    })
                } else {
                    Modifiers((rng.next() as u32) & (rng.next() as u32))
                };
                let key = KeyEvent {
                    kind,
                    keycode: CODES[rng.below(CODES.len() as u64) as usize],
                    chars: CHARS[rng.below(CHARS.len() as u64) as usize].to_owned(),
                    mods,
                    timestamp: t,
                };
                let out = e.on_key(&key);
                if let Some(m) = &out.marked {
                    assert!(m.chars().count() <= 128);
                }
                if let Some(r) = &out.replace {
                    assert!(r.delete_before > 0);
                }
            }
        }
    }
    let d = e.take_diagnostics();
    assert!(!d.iter().any(|m| m.contains("panic")), "{label}: {d:?}");
    e.shutdown().unwrap();
}

#[test]
fn garbage_never_panics_with_chaotic_model() {
    for hotkeys in [true, false] {
        let cfg = EngineConfig {
            hotkeys,
            ..inline_cfg()
        };
        let (e, _s, _) = scripted_engine(chaotic(), cfg);
        garbage(e, 0xBAD5EED ^ u64::from(hotkeys), "chaotic");
    }
}

#[test]
fn garbage_never_panics_with_linear_action() {
    let cfg = EngineConfig {
        policy: ModelKind::LinearAction,
        ..inline_cfg()
    };
    let e = Engine::new(cfg, Arc::new(MemStorage::new()), session());
    garbage(e, 0xFEED, "linear_action");
}
