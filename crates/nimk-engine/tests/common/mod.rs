//! Shared helpers for engine integration tests.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use nimk_engine::{Engine, EngineConfig, EngineOutput, Threading};
use nimk_policy::{FeatureExtractor, HashedExtractor, Learner, Model, Policy, PolicyError};
use nimk_storage::{MemStorage, Storage};
use nimk_types::{
    AppContext, Event, Features, KeyEvent, KeyKind, Modifiers, Record, Sample, keycode,
};

/// Policy returning a constant `p(VI)`.
pub struct ConstPolicy(pub f32);

impl Policy for ConstPolicy {
    fn id(&self) -> &str {
        "const@v0"
    }
    fn predict(&self, _features: &Features) -> f32 {
        self.0
    }
}

/// Learner that never changes its constant prediction.
pub struct ConstLearner(pub f32);

impl Learner for ConstLearner {
    fn id(&self) -> &str {
        "const@v0"
    }
    fn update(&mut self, _batch: &[Sample]) -> f32 {
        0.5
    }
    fn snapshot(&self) -> Arc<dyn Policy> {
        Arc::new(ConstPolicy(self.0))
    }
    fn save(&self) -> Vec<u8> {
        b"const".to_vec()
    }
    fn load(&mut self, _bytes: &[u8]) -> Result<(), PolicyError> {
        Ok(())
    }
}

/// Model with a constant `p(VI)`.
pub fn const_model(p: f32) -> Model {
    let extractor: Arc<dyn FeatureExtractor> = Arc::new(HashedExtractor);
    Model {
        extractor,
        learner: Box::new(ConstLearner(p)),
    }
}

/// Inline config (deterministic).
pub fn inline_cfg() -> EngineConfig {
    EngineConfig {
        threading: Threading::Inline,
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

/// Engine with a constant policy over fresh in-memory storage.
pub fn const_engine(p: f32, cfg: EngineConfig) -> (Engine, MemStorage) {
    let storage = MemStorage::new();
    let engine = Engine::with_model(cfg, Arc::new(storage.clone()), session(), const_model(p));
    (engine, storage)
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

/// Lone Left-Shift tap; returns the release output.
pub fn shift_tap(engine: &mut Engine, t: &mut f64) -> EngineOutput {
    *t += 0.1;
    let down = engine.on_key(&flags(keycode::LEFT_SHIFT, Modifiers::SHIFT, *t));
    assert!(!down.consumed, "shift down is not consumed");
    *t += 0.1;
    engine.on_key(&flags(keycode::LEFT_SHIFT, 0, *t))
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
