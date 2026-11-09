//! Unit tests of the trainer schedule and backends.
use nimk_policy::PolicyError;
use nimk_storage::{BlobStore, EventSink, EventSource};
use nimk_types::{Action, ActionScores, EditAction, Lang, Observation};

use super::*;

struct Fixed;
impl ImeModel for Fixed {
    fn id(&self) -> &str {
        "fake"
    }
    fn score(&self, _o: &Observation) -> ActionScores {
        ActionScores {
            edit: [0.25; 4],
            p_vi: 0.5,
        }
    }
}

#[derive(Default)]
struct Counting {
    batches: Vec<usize>,
    corrections_only: bool,
}
impl ImeLearner for Counting {
    fn id(&self) -> &str {
        "fake"
    }
    fn update(&mut self, batch: &[Example]) -> f32 {
        self.batches.push(batch.len());
        0.25
    }
    fn learns_from_confirmations(&self) -> bool {
        !self.corrections_only
    }
    fn snapshot(&self) -> Arc<dyn ImeModel> {
        Arc::new(Fixed)
    }
    fn save(&self) -> Vec<u8> {
        format!("{:?}", self.batches).into_bytes()
    }
    fn load(&mut self, _b: &[u8]) -> Result<(), PolicyError> {
        Ok(())
    }
}

struct NoStorage;
impl BlobStore for NoStorage {
    fn put(&self, _k: &str, _b: &[u8]) -> io::Result<()> {
        Err(io::Error::other("read-only"))
    }
    fn get(&self, _k: &str) -> io::Result<Option<Vec<u8>>> {
        Ok(None)
    }
    fn list(&self, _p: &str) -> io::Result<Vec<String>> {
        Ok(Vec::new())
    }
    fn delete(&self, _k: &str) -> io::Result<bool> {
        Ok(false)
    }
}
impl Storage for NoStorage {
    fn event_sink(&self, _s: &str) -> io::Result<Box<dyn EventSink>> {
        Err(io::Error::other("none"))
    }
    fn event_source(&self) -> io::Result<Box<dyn EventSource + '_>> {
        Err(io::Error::other("none"))
    }
    fn blobs(&self) -> &dyn BlobStore {
        self
    }
}

fn params() -> TrainerParams {
    TrainerParams {
        correction_window: 3,
        correction_steps: 2,
        replay_capacity: 4,
        replay_every: 5,
        replay_batch: 2,
        seed: 7,
    }
}

fn sample() -> Example {
    Example {
        obs: Observation::default(),
        target: Action {
            edit: EditAction::Append,
            render: Lang::Vi,
        },
        edit_weight: 0.0,
        render_weight: 0.3,
    }
}

fn core() -> Core {
    let save = SaveTarget {
        storage: Arc::new(NoStorage),
        key: "model/fake/latest".into(),
    };
    Core::new(
        ModelKind::LinearAction,
        Box::new(Counting::default()),
        params(),
        save,
    )
}

fn kinds(out: &[TrainerOut]) -> String {
    out.iter()
        .map(|o| match o {
            TrainerOut::Update { n_samples, .. } => format!("u{n_samples}"),
            TrainerOut::Snapshot(_) => "s".to_owned(),
            TrainerOut::Score { .. } => "t".to_owned(),
            TrainerOut::Diagnostic(_) => "d".to_owned(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn schedule() {
    let mut c = core();
    let mut out = Vec::new();
    for _ in 0..4 {
        c.on_examples(vec![sample()], false, None, &mut out);
    }
    assert_eq!(kinds(&out), "u1,u1,u1,u1");
    out.clear();
    // 5th sample: replay minibatch of 2.
    c.on_examples(vec![sample()], false, None, &mut out);
    assert_eq!(kinds(&out), "u1,u2");
    out.clear();
    // Correction: window of 3 trained twice, then publish.
    c.on_examples(vec![sample()], true, None, &mut out);
    assert_eq!(kinds(&out), "u1,u6,s");
}

#[test]
fn corrections_only_learner_skips_confirmations() {
    let save = SaveTarget {
        storage: Arc::new(NoStorage),
        key: "model/fake/latest".into(),
    };
    let learner = Counting {
        corrections_only: true,
        ..Counting::default()
    };
    let mut c = Core::new(ModelKind::LinearAction, Box::new(learner), params(), save);
    let mut out = Vec::new();
    for _ in 0..6 {
        c.on_examples(vec![sample()], false, None, &mut out);
    }
    assert_eq!(kinds(&out), "", "confirmations neither train nor replay");
    // Teacher-forced examples (edit weight > 0) still train.
    let taught = Example {
        edit_weight: 1.0,
        ..sample()
    };
    c.on_examples(vec![taught], false, None, &mut out);
    assert_eq!(kinds(&out), "u1");
    out.clear();
    c.on_examples(vec![sample()], true, None, &mut out);
    assert_eq!(kinds(&out), "u1,u4,s", "corrections train and re-train");
}

#[test]
fn inline_save_error_is_a_diagnostic() {
    let mut t = Trainer::new(vec![core()], false);
    t.request_save();
    let mut out = Vec::new();
    t.drain(&mut out);
    assert_eq!(kinds(&out), "d");
    assert!(t.shutdown(&mut out).is_err());
}

#[test]
fn background_matches_inline() {
    let run = |bg: bool| {
        let mut t = Trainer::new(vec![core()], bg);
        for i in 0..30 {
            t.submit(vec![sample()], i % 7 == 0, None);
        }
        let mut out = Vec::new();
        let _ = t.shutdown(&mut out);
        kinds(&out)
    };
    assert_eq!(run(false), run(true));
}

#[test]
fn word_examples_are_one_update() {
    let mut c = core();
    let mut out = Vec::new();
    c.on_examples(vec![sample(), sample(), sample()], false, None, &mut out);
    assert_eq!(kinds(&out), "u3");
    out.clear();
    c.on_examples(Vec::new(), true, None, &mut out);
    assert!(out.is_empty(), "empty word is a no-op");
}

/// The `Score` outputs of `out`, as `(word_id, correct, hits, total, flips, amend)`.
fn scores(out: &[TrainerOut]) -> Vec<(u64, bool, u16, u16, u16, bool)> {
    out.iter()
        .filter_map(|o| match o {
            TrainerOut::Score {
                word_id,
                correct,
                display_hits,
                display_total,
                flips,
                amend,
                ..
            } => Some((
                *word_id,
                *correct,
                *display_hits,
                *display_total,
                *flips,
                *amend,
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn words_are_scored_and_late_corrections_rescore_them() {
    let mut c = core();
    let mut out = Vec::new();
    for lang in [Lang::Vi, Lang::En, Lang::Vi] {
        c.on_key(&KeyObs {
            word_id: 4,
            active_render: lang,
            margin: 0.1,
            obs: None,
        });
    }
    let info = |label, amends| WordInfo {
        word_id: 4,
        label,
        committed: Lang::Vi,
        amends,
        category: StatCategory::Ide,
    };
    c.on_examples(Vec::new(), false, Some(info(Lang::Vi, None)), &mut out);
    assert_eq!(scores(&out), vec![(4, true, 2, 3, 2, false)]);
    out.clear();
    // The truth becomes En: wrong, one keystroke matched.
    c.on_examples(
        Vec::new(),
        true,
        Some(info(Lang::En, Some(Lang::Vi))),
        &mut out,
    );
    assert_eq!(scores(&out), vec![(4, false, 1, 3, 2, true)]);
    out.clear();
    // Same truth again, or an unknown word: nothing.
    c.on_examples(
        Vec::new(),
        true,
        Some(info(Lang::En, Some(Lang::Vi))),
        &mut out,
    );
    let unknown = WordInfo {
        word_id: 99,
        ..info(Lang::Vi, Some(Lang::En))
    };
    c.on_examples(Vec::new(), true, Some(unknown), &mut out);
    assert!(scores(&out).is_empty());
}
