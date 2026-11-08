//! Shared helpers for engine integration tests.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use nimk_engine::{Engine, EngineConfig, EngineOutput, Threading};
use nimk_policy::{ImeLearner, ImeModel, PolicyError};
use nimk_storage::{MemStorage, Storage};
use nimk_types::{
    ActionScores, AppContext, Event, Example, KeyEvent, KeyKind, Modifiers, Observation, Record,
    keycode,
};

/// Scripted scoring function of the fake model.
pub type ScoreFn = dyn Fn(&Observation) -> ActionScores + Send + Sync;

/// Examples the fake learner was updated with.
pub type Seen = Arc<Mutex<Vec<Example>>>;

/// Model answering from a closure.
pub struct FakeModel(pub Arc<ScoreFn>);

impl ImeModel for FakeModel {
    fn id(&self) -> &str {
        "fake@v0"
    }
    fn score(&self, obs: &Observation) -> ActionScores {
        (self.0)(obs)
    }
}

/// Learner that never changes its script and records every example.
pub struct FakeLearner {
    pub script: Arc<ScoreFn>,
    pub seen: Seen,
}

impl ImeLearner for FakeLearner {
    fn id(&self) -> &str {
        "fake@v0"
    }
    fn update(&mut self, batch: &[Example]) -> f32 {
        self.seen.lock().unwrap().extend_from_slice(batch);
        0.5
    }
    fn snapshot(&self) -> Arc<dyn ImeModel> {
        Arc::new(FakeModel(Arc::clone(&self.script)))
    }
    fn save(&self) -> Vec<u8> {
        b"fake".to_vec()
    }
    fn load(&mut self, _bytes: &[u8]) -> Result<(), PolicyError> {
        Ok(())
    }
}

fn one_hot(i: usize) -> [f32; 4] {
    let mut e = [0.0; 4];
    e[i] = 1.0;
    e
}

/// Typing behaviour cloned by hand: letters append, Backspace pops, any
/// other key commits a non-empty word and otherwise passes. `p_vi` is a
/// function of the observation.
pub fn typist(p_vi: impl Fn(&Observation) -> f32 + Send + Sync + 'static) -> Arc<ScoreFn> {
    Arc::new(move |o: &Observation| {
        let p = p_vi(o);
        let Some(k) = &o.key else {
            return ActionScores {
                edit: [0.25; 4],
                p_vi: p,
            };
        };
        let letter = k.chars.chars().next().is_some_and(char::is_alphabetic);
        let edit = if k.keycode == keycode::BACKSPACE {
            1
        } else if letter && o.mask.append {
            0
        } else if !o.raw.is_empty() {
            2
        } else {
            3
        };
        ActionScores {
            edit: one_hot(edit),
            p_vi: p,
        }
    })
}

/// Typist with a constant `p(VI)`.
pub fn const_typist(p: f32) -> Arc<ScoreFn> {
    typist(move |_| p)
}

/// Engine with a scripted model over fresh in-memory storage.
pub fn scripted_engine(script: Arc<ScoreFn>, cfg: EngineConfig) -> (Engine, MemStorage, Seen) {
    let storage = MemStorage::new();
    let seen = Seen::default();
    let learner = FakeLearner {
        script,
        seen: Arc::clone(&seen),
    };
    let engine = Engine::with_model(cfg, Arc::new(storage.clone()), session(), Box::new(learner));
    (engine, storage, seen)
}

/// Engine with a constant-`p(VI)` typist.
pub fn const_engine(p: f32, cfg: EngineConfig) -> (Engine, MemStorage) {
    let (e, s, _) = scripted_engine(const_typist(p), cfg);
    (e, s)
}

/// Inline config (deterministic, no replay minibatches so example counts
/// are exact).
pub fn inline_cfg() -> EngineConfig {
    EngineConfig {
        threading: Threading::Inline,
        replay_every: 0,
        ..EngineConfig::default()
    }
}

/// Session info for tests.
pub fn session() -> nimk_engine::SessionInfo {
    nimk_engine::SessionInfo {
        id: "test-session".into(),
        wall_unix_ms: 1,
        app_version: "0.0.0".into(),
    }
}

/// Key-down for one character (letters keycode 0, space, comma).
pub fn char_key(c: char, t: f64) -> KeyEvent {
    let (code, mods) = match c {
        ' ' => (keycode::SPACE, 0),
        ',' => (43, 0),
        '.' => (47, 0),
        c if c.is_ascii_uppercase() => (0, Modifiers::SHIFT),
        _ => (0, 0),
    };
    KeyEvent {
        kind: KeyKind::KeyDown,
        keycode: code,
        chars: c.to_string(),
        mods: Modifiers(mods),
        timestamp: t,
    }
}

/// Simple keys by keycode.
pub fn code_key(code: u16, chars: &str, mods: u32, t: f64) -> KeyEvent {
    KeyEvent {
        kind: KeyKind::KeyDown,
        keycode: code,
        chars: chars.into(),
        mods: Modifiers(mods),
        timestamp: t,
    }
}

/// Modifier change.
pub fn flags(code: u16, mods: u32, t: f64) -> KeyEvent {
    KeyEvent {
        kind: KeyKind::FlagsChanged,
        keycode: code,
        chars: String::new(),
        mods: Modifiers(mods),
        timestamp: t,
    }
}

/// Types `text` one key every 0.1 s from `*t`; returns all outputs.
pub fn type_str(engine: &mut Engine, text: &str, t: &mut f64) -> Vec<EngineOutput> {
    text.chars()
        .map(|c| {
            *t += 0.1;
            engine.on_key(&char_key(c, *t))
        })
        .collect()
}

/// One lone Left-Shift tap (press and release 0.05 s apart); returns the
/// release output.
pub fn shift_tap(engine: &mut Engine, t: &mut f64) -> EngineOutput {
    *t += 0.1;
    let down = engine.on_key(&flags(keycode::LEFT_SHIFT, Modifiers::SHIFT, *t));
    assert!(!down.consumed, "shift down is not consumed");
    *t += 0.05;
    engine.on_key(&flags(keycode::LEFT_SHIFT, 0, *t))
}

/// Double Shift tap (0.1 s apart); returns the output of the second release.
pub fn double_shift(engine: &mut Engine, t: &mut f64) -> EngineOutput {
    let first = shift_tap(engine, t);
    assert!(!first.consumed, "a single tap is passed through");
    shift_tap(engine, t)
}

/// Context for app `bundle` / field `field`.
pub fn ctx(bundle: &str, field: u64, t: f64) -> AppContext {
    AppContext {
        bundle_id: bundle.into(),
        field_id: field,
        caret: None,
        timestamp: t,
    }
}

/// Committed texts in order.
pub fn commits(outs: &[EngineOutput]) -> Vec<String> {
    outs.iter().filter_map(|o| o.commit.clone()).collect()
}

/// Raw record bytes of every segment, in order.
pub fn log_bytes(storage: &dyn Storage) -> Vec<Vec<u8>> {
    let src = storage.event_source().unwrap();
    let mut out = Vec::new();
    for seg in src.segments().unwrap() {
        for rec in src.read_segment(&seg).unwrap() {
            out.push(rec.unwrap());
        }
    }
    out
}

/// Decoded events of every segment, in order.
pub fn events(storage: &dyn Storage) -> Vec<Event> {
    log_bytes(storage)
        .iter()
        .map(|b| serde_json::from_slice::<Record>(b).unwrap().event)
        .collect()
}
