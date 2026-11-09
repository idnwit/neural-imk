//! Shadow training and per-model statistics (ADR 0013): scripted models
//! with known decisions.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use common::*;
use nimk_engine::{Engine, EngineConfig, LabelSource, Lang, ModelKind, Target, Threading};
use nimk_policy::{ImeLearner, ImeModel, PolicyError};
use nimk_storage::{MemStorage, Storage};
use nimk_types::{ActionScores, Example, Observation};

const LIN: ModelKind = ModelKind::LinearAction;
const GRU: ModelKind = ModelKind::GruAction;

/// Threads that called `score`, in call order.
type Calls = Arc<Mutex<Vec<ThreadId>>>;

struct Probe {
    script: Arc<ScoreFn>,
    calls: Calls,
    examples: Arc<Mutex<usize>>,
}

struct ProbeModel {
    script: Arc<ScoreFn>,
    calls: Calls,
}

impl ImeModel for ProbeModel {
    fn id(&self) -> &str {
        "probe@v0"
    }
    fn score(&self, obs: &Observation) -> ActionScores {
        self.calls.lock().unwrap().push(std::thread::current().id());
        (self.script)(obs)
    }
}

impl ImeLearner for Probe {
    fn id(&self) -> &str {
        "probe@v0"
    }
    fn update(&mut self, batch: &[Example]) -> f32 {
        *self.examples.lock().unwrap() += batch.len();
        0.5
    }
    fn snapshot(&self) -> Arc<dyn ImeModel> {
        Arc::new(ProbeModel {
            script: Arc::clone(&self.script),
            calls: Arc::clone(&self.calls),
        })
    }
    fn save(&self) -> Vec<u8> {
        // The checkpoint changes whenever the learner saw more examples.
        format!("seen={}", self.examples.lock().unwrap()).into_bytes()
    }
    fn load(&mut self, _bytes: &[u8]) -> Result<(), PolicyError> {
        Ok(())
    }
}

/// Handles to one scripted model.
struct Handle {
    calls: Calls,
    examples: Arc<Mutex<usize>>,
}

impl Handle {
    fn calls(&self) -> Vec<ThreadId> {
        self.calls.lock().unwrap().clone()
    }
    fn examples(&self) -> usize {
        *self.examples.lock().unwrap()
    }
}

fn probe(script: Arc<ScoreFn>) -> (Box<dyn ImeLearner>, Handle) {
    let h = Handle {
        calls: Calls::default(),
        examples: Arc::default(),
    };
    let learner = Probe {
        script,
        calls: Arc::clone(&h.calls),
        examples: Arc::clone(&h.examples),
    };
    (Box::new(learner), h)
}

/// Linear (active) always says Vietnamese; Gru (shadow) says Vietnamese
/// until the buffer holds two letters, i.e. English at the commit step of a
/// two-letter word.
type Learners = Vec<(ModelKind, Box<dyn ImeLearner>)>;

fn disagreeing_pair() -> (Learners, Handle, Handle) {
    let (a, ha) = probe(const_typist(0.9));
    let (b, hb) = probe(typist(
        |o| if o.raw.chars().count() >= 2 { 0.1 } else { 0.9 },
    ));
    (vec![(LIN, a), (GRU, b)], ha, hb)
}

fn cfg(threading: Threading) -> EngineConfig {
    EngineConfig {
        threading,
        hotkeys: false,
        ..inline_cfg()
    }
}

fn engine(cfg: EngineConfig, storage: &MemStorage, learners: Learners) -> Engine {
    Engine::with_models(cfg, Arc::new(storage.clone()), session(), learners)
}

fn label(e: &mut Engine, id: u64, lang: Lang) {
    e.inject_label(Target::Word(id), lang, 1.0, LabelSource::Correction);
}

fn blob(s: &MemStorage, key: &str) -> Option<Vec<u8>> {
    s.blobs().get(key).unwrap()
}

#[test]
fn each_model_counts_its_own_predictions_per_word() {
    let storage = MemStorage::new();
    let (learners, _, _) = disagreeing_pair();
    let mut e = engine(cfg(Threading::Inline), &storage, learners);
    let mut t = 1.0;
    e.on_context(&ctx("com.apple.TextEdit", 1, t));
    type_str(&mut e, "ab ", &mut t);
    label(&mut e, 0, Lang::Vi);
    type_str(&mut e, "cd ", &mut t);
    label(&mut e, 1, Lang::En);

    // Active: committed Vi both times; the second was corrected by the user.
    let a = e.model_stats(LIN);
    assert_eq!(
        (a.words_total, a.corrected_total, a.served_words_total),
        (2, 1, 2)
    );
    assert_eq!((a.keys_total, a.keys_match_total, a.flips_total), (6, 3, 0));
    assert_eq!(a.corrected_by_category, [0, 0, 0, 1, 0], "editor");
    assert_eq!((a.words_recent, a.corrected_recent), (2, 1));

    // Shadow: wanted English at the commit step of both words, so it would
    // have been corrected on the first only; it flips once per word.
    let b = e.model_stats(GRU);
    assert_eq!(
        (b.words_total, b.corrected_total, b.served_words_total),
        (2, 1, 0)
    );
    assert_eq!((b.keys_total, b.keys_match_total, b.flips_total), (6, 3, 2));
    assert_eq!(b.corrected_by_category, [0, 0, 0, 1, 0]);
    assert_eq!(
        (b.keys_recent, b.keys_match_recent, b.flips_recent),
        (6, 3, 2)
    );
}

#[test]
fn rolling_window_keeps_the_last_500_words() {
    let storage = MemStorage::new();
    let (learners, _, _) = disagreeing_pair();
    let mut e = engine(cfg(Threading::Inline), &storage, learners);
    let mut t = 1.0;
    e.on_context(&ctx("com.apple.TextEdit", 1, t));
    for id in 0..510u64 {
        type_str(&mut e, "ab ", &mut t);
        label(&mut e, id, Lang::Vi);
    }
    let b = e.model_stats(GRU);
    assert_eq!(b.words_total, 510);
    assert_eq!(b.words_recent, 500);
    assert_eq!(b.corrected_recent, 500);
    assert_eq!(e.model_stats(LIN).corrected_total, 0);
}

#[test]
fn shadow_learns_while_inactive_and_saves_its_own_checkpoint() {
    let storage = MemStorage::new();
    let (learners, ha, hb) = disagreeing_pair();
    let mut e = engine(cfg(Threading::Inline), &storage, learners);
    let mut t = 1.0;
    e.on_context(&ctx("com.apple.TextEdit", 1, t));
    type_str(&mut e, "ab ", &mut t);
    label(&mut e, 0, Lang::Vi);
    assert!(
        ha.examples() > 0 && hb.examples() > 0,
        "both learners trained"
    );
    assert_eq!(
        ha.examples(),
        hb.examples(),
        "same examples to each learner"
    );
    e.shutdown().unwrap();
    let first = blob(&storage, "model/gru_action/latest").unwrap();
    assert!(blob(&storage, "model/linear_action/latest").is_some());

    // More labels, a second run: the shadow's checkpoint changed.
    let (learners, _, hb2) = disagreeing_pair();
    let mut e = engine(cfg(Threading::Inline), &storage, learners);
    let mut t = 1.0;
    e.on_context(&ctx("com.apple.TextEdit", 1, t));
    type_str(&mut e, "ab cd ", &mut t);
    label(&mut e, 0, Lang::Vi);
    label(&mut e, 1, Lang::Vi);
    assert!(hb2.examples() > 0);
    e.shutdown().unwrap();
    // The scripted learner restarts from zero, so compare with its own count.
    assert_eq!(
        blob(&storage, "model/gru_action/latest").unwrap(),
        format!("seen={}", hb2.examples()).into_bytes()
    );
    assert_ne!(blob(&storage, "model/gru_action/latest").unwrap(), first);
}

#[test]
fn online_training_off_freezes_a_model() {
    let storage = MemStorage::new();
    let (learners, ha, hb) = disagreeing_pair();
    let mut c = cfg(Threading::Inline);
    c.models.set_online_training(GRU, false);
    let mut e = engine(c, &storage, learners);
    let mut t = 1.0;
    e.on_context(&ctx("com.apple.TextEdit", 1, t));
    type_str(&mut e, "ab ", &mut t);
    label(&mut e, 0, Lang::Vi);
    assert!(ha.examples() > 0);
    assert_eq!(hb.examples(), 0, "frozen: no update");
    assert_eq!(e.model_stats(GRU).words_total, 0, "frozen: not measured");
    assert!(hb.calls().is_empty(), "frozen: not scored in shadow");
    e.shutdown().unwrap();
    assert!(
        blob(&storage, "model/gru_action/latest").is_none(),
        "frozen: not saved"
    );
    assert!(blob(&storage, "model/linear_action/latest").is_some());
}

#[test]
fn toggle_applies_live_and_survives_restart() {
    let storage = MemStorage::new();
    let (learners, _, hb) = disagreeing_pair();
    let mut e = engine(cfg(Threading::Inline), &storage, learners);
    assert!(e.online_training(GRU));
    e.set_online_training(GRU, false);
    assert!(!e.online_training(GRU));
    let mut t = 1.0;
    e.on_context(&ctx("com.apple.TextEdit", 1, t));
    type_str(&mut e, "ab ", &mut t);
    label(&mut e, 0, Lang::Vi);
    assert_eq!(hb.examples(), 0);
    e.set_online_training(GRU, true);
    type_str(&mut e, "cd ", &mut t);
    label(&mut e, 1, Lang::Vi);
    assert!(hb.examples() > 0, "learns again once switched on");
    e.set_online_training(LIN, false);
    e.shutdown().unwrap();

    let e = Engine::new(cfg(Threading::Inline), Arc::new(storage.clone()), session());
    assert!(!e.online_training(LIN), "menu choice wins over config");
    assert!(e.online_training(GRU));
}

#[test]
fn stats_persist_and_reset_with_the_model() {
    let storage = MemStorage::new();
    let (learners, _, _) = disagreeing_pair();
    let mut e = engine(cfg(Threading::Inline), &storage, learners);
    let mut t = 1.0;
    e.on_context(&ctx("com.apple.TextEdit", 1, t));
    type_str(&mut e, "ab ", &mut t);
    label(&mut e, 0, Lang::Vi);
    let (a, b) = (e.model_stats(LIN), e.model_stats(GRU));
    assert_eq!((a.words_total, b.words_total), (1, 1));
    e.shutdown().unwrap();
    assert!(blob(&storage, "stats/linear_action").is_some());

    // A new engine starts from the saved counters.
    let mut e = Engine::new(cfg(Threading::Inline), Arc::new(storage.clone()), session());
    assert_eq!(e.model_stats(LIN), a);
    assert_eq!(e.model_stats(GRU), b);

    // Resetting the active model restarts only its counters.
    e.reset_model(5.0);
    assert_eq!(e.model_stats(LIN).words_total, 0);
    assert!(blob(&storage, "stats/linear_action").is_none());
    assert_eq!(e.model_stats(GRU), b, "the other model keeps its counters");
    e.shutdown().unwrap();
    assert!(
        blob(&storage, "stats/linear_action").is_none(),
        "nothing counted since"
    );
}

#[test]
fn hot_path_never_scores_the_shadow() {
    let main = std::thread::current().id();
    for threading in [Threading::Inline, Threading::Background] {
        let storage = MemStorage::new();
        let (learners, ha, hb) = disagreeing_pair();
        let mut e = engine(cfg(threading), &storage, learners);
        let mut t = 1.0;
        e.on_context(&ctx("com.apple.TextEdit", 1, t));
        for _ in 0..4 {
            type_str(&mut e, "ab ", &mut t);
        }
        e.shutdown().unwrap();
        // One call per typed key, with the observation pushed to the trainer.
        assert_eq!(hb.calls().len(), 12, "{threading:?}");
        assert!(!ha.calls().is_empty());
        assert!(ha.calls().iter().all(|id| *id == main), "active: hot path");
        match threading {
            Threading::Background => assert!(
                hb.calls().iter().all(|id| *id != main),
                "shadow scored off the hot path"
            ),
            Threading::Inline => assert!(hb.calls().iter().all(|id| *id == main)),
        }
    }
}

#[test]
fn inline_runs_are_deterministic() {
    let run = || {
        let storage = MemStorage::new();
        let (learners, _, _) = disagreeing_pair();
        let mut e = engine(cfg(Threading::Inline), &storage, learners);
        let mut t = 1.0;
        e.on_context(&ctx("com.apple.TextEdit", 1, t));
        for id in 0..5u64 {
            type_str(&mut e, "ab ", &mut t);
            label(&mut e, id, if id % 2 == 0 { Lang::Vi } else { Lang::En });
        }
        let stats = (e.model_stats(LIN), e.model_stats(GRU));
        e.shutdown().unwrap();
        (
            stats,
            blob(&storage, "stats/gru_action"),
            blob(&storage, "model/gru_action/latest"),
        )
    };
    assert_eq!(run(), run());
}
