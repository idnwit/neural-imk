//! Unit tests of the trainer schedule and backends.
use nimk_policy::PolicyError;
use nimk_storage::{BlobStore, EventSink, EventSource};
use nimk_types::{Features, Lang};

use super::*;

struct Fixed;
impl Policy for Fixed {
    fn id(&self) -> &str {
        "fake"
    }
    fn predict(&self, _f: &Features) -> f32 {
        0.5
    }
}

#[derive(Default)]
struct Counting {
    batches: Vec<usize>,
}
impl Learner for Counting {
    fn id(&self) -> &str {
        "fake"
    }
    fn update(&mut self, batch: &[Sample]) -> f32 {
        self.batches.push(batch.len());
        0.25
    }
    fn snapshot(&self) -> Arc<dyn Policy> {
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

fn sample() -> Sample {
    Sample {
        features: Features::default(),
        label: Lang::Vi,
        weight: 0.3,
    }
}

fn core() -> Core {
    let save = SaveTarget {
        storage: Arc::new(NoStorage),
        key: "model/fake/latest".into(),
    };
    Core::new(Box::new(Counting::default()), params(), save)
}

fn kinds(out: &[TrainerOut]) -> String {
    out.iter()
        .map(|o| match o {
            TrainerOut::Update { n_samples, .. } => format!("u{n_samples}"),
            TrainerOut::Snapshot(_) => "s".to_owned(),
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
        c.on_sample(sample(), false, &mut out);
    }
    assert_eq!(kinds(&out), "u1,u1,u1,u1");
    out.clear();
    // 5th sample: replay minibatch of 2.
    c.on_sample(sample(), false, &mut out);
    assert_eq!(kinds(&out), "u1,u2");
    out.clear();
    // Correction: window of 3 trained twice, then publish.
    c.on_sample(sample(), true, &mut out);
    assert_eq!(kinds(&out), "u1,u6,s");
}

#[test]
fn inline_save_error_is_a_diagnostic() {
    let mut t = Trainer::new(core(), false);
    t.request_save();
    let mut out = Vec::new();
    t.drain(&mut out);
    assert_eq!(kinds(&out), "d");
    assert!(t.shutdown(&mut out).is_err());
}

#[test]
fn background_matches_inline() {
    let run = |bg: bool| {
        let mut t = Trainer::new(core(), bg);
        for i in 0..30 {
            t.submit(sample(), i % 7 == 0);
        }
        let mut out = Vec::new();
        let _ = t.shutdown(&mut out);
        kinds(&out)
    };
    assert_eq!(run(false), run(true));
}
