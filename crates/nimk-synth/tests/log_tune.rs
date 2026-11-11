//! Real-log scripts and on-demand fine-tuning (`nimk-tune`), exercised with
//! synthetic logs written by `write_log`.
#![allow(clippy::unwrap_used)]

use std::io;
use std::sync::Arc;

use nimk_engine::EngineConfig;
use nimk_policy::ModelKind;
use nimk_storage::{MemStorage, Storage};
use nimk_synth::{
    Corpus, PretrainOptions, Script, SynthConfig, TUNE_STATE_KEY, TuneOptions, TuneOutput,
    generate, pretrain, scripts_from_log, tune, tune_pending, tune_result_key, write_log,
};

fn scripts(seed: u64, sessions: usize, words: usize) -> Vec<Script> {
    let cfg = SynthConfig {
        seed,
        sessions,
        words_per_session: words,
        ..SynthConfig::default()
    };
    generate(&cfg, &Corpus::builtin())
}

fn words(s: &[Script]) -> u64 {
    s.iter().map(|s| s.words.len() as u64).sum()
}

#[test]
fn round_trips_a_synthetic_log() {
    let scripts = scripts(11, 3, 150);
    let storage = MemStorage::new();
    write_log(&scripts, &storage).unwrap();
    let got = scripts_from_log(storage.event_source().unwrap().as_ref()).unwrap();
    assert_eq!((got.sessions, got.skipped, got.malformed), (3, 0, 0));
    assert_eq!(got.scripts.len(), scripts.len());
    assert_eq!(got.meta.len(), got.scripts.len());
    for want in &scripts {
        let i = got
            .scripts
            .iter()
            .position(|s| s.session == format!("log-{}-0.0", want.session))
            .unwrap_or_else(|| {
                let names: Vec<_> = got.scripts.iter().map(|s| &s.session).collect();
                panic!("no script for {}: {names:?}", want.session)
            });
        let s = &got.scripts[i];
        assert_eq!(got.meta[i].session, want.session);
        assert_eq!(s.bundle_id, want.bundle_id);
        assert_eq!(s.keys.len(), want.keys.len(), "key count");
        // JSON round-trips f64 to within an ulp: compare time loosely.
        for (n, (a, b)) in s.keys.iter().zip(&want.keys).enumerate() {
            let mut same = a.clone();
            same.timestamp = b.timestamp;
            assert_eq!(&same, b, "key {n}");
            assert!((a.timestamp - b.timestamp).abs() < 1e-6, "key {n} time");
        }
        assert_eq!(s.targets, want.targets);
        let langs = |x: &Script| {
            x.words
                .iter()
                .map(|w| (w.lang, w.raw.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(langs(s), langs(want));
    }
}

/// Storage with a pre-trained linear snapshot and `first` logged.
fn setup(first: &[Script]) -> Arc<dyn Storage> {
    let storage: Arc<dyn Storage> = Arc::new(MemStorage::new());
    let cfg = EngineConfig {
        policy: ModelKind::LinearAction,
        ..EngineConfig::default()
    };
    let opts = PretrainOptions {
        epochs: 1,
        eval_frac: 0.0,
        ..PretrainOptions::default()
    };
    pretrain(&scripts(1, 4, 120), cfg, opts, Arc::clone(&storage)).unwrap();
    write_log(first, storage.as_ref()).unwrap();
    storage
}

#[test]
fn everything_is_new_before_the_first_run() {
    let first = scripts(2, 3, 120);
    let storage = setup(&first);
    let p = tune_pending(storage.as_ref()).unwrap();
    assert_eq!(p.new_words, words(&first));
    assert_eq!(p.total_words, words(&first));
    assert_eq!(p.last_unix_ms, None);
    assert_eq!(p.runs, 0);
}

#[test]
fn a_run_consumes_the_data_and_new_data_counts_again() {
    let first = scripts(2, 3, 120);
    let storage = setup(&first);
    let r = tune(
        &storage,
        ModelKind::LinearAction,
        TuneOptions::default(),
        1234,
        TuneOutput::Result,
    )
    .unwrap();
    assert_eq!(r.new_words, words(&first));
    assert_eq!(r.replay_words, 0);
    assert!(r.keys > 0);
    let key = tune_result_key(ModelKind::LinearAction);
    assert!(storage.blobs().get(&key).unwrap().is_some());
    let p = tune_pending(storage.as_ref()).unwrap();
    assert_eq!((p.new_words, p.new_corrections), (0, 0));
    assert_eq!(p.total_words, words(&first));
    assert_eq!((p.last_unix_ms, p.runs), (Some(1234), 1));

    // More typing later: only that counts, and older words are replayed.
    let second = scripts(9, 2, 120);
    write_log(&second, storage.as_ref()).unwrap();
    let p = tune_pending(storage.as_ref()).unwrap();
    assert_eq!(p.new_words, words(&second));
    assert_eq!(p.total_words, words(&first) + words(&second));
    let r = tune(
        &storage,
        ModelKind::LinearAction,
        TuneOptions::default(),
        5678,
        TuneOutput::Result,
    )
    .unwrap();
    assert_eq!(r.new_words, words(&second));
    assert!(r.replay_words > 0);
    assert_eq!(tune_pending(storage.as_ref()).unwrap().runs, 2);
}

#[test]
fn too_little_new_data_is_refused_and_changes_nothing() {
    let first = scripts(2, 1, 120);
    let storage = setup(&first);
    let opts = TuneOptions {
        min_new_words: words(&first) + 1,
        ..TuneOptions::default()
    };
    let err = tune(
        &storage,
        ModelKind::LinearAction,
        opts,
        1,
        TuneOutput::Result,
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert!(storage.blobs().get(TUNE_STATE_KEY).unwrap().is_none());
    assert_eq!(
        tune_pending(storage.as_ref()).unwrap().new_words,
        words(&first)
    );
}

#[test]
fn a_missing_snapshot_is_an_error() {
    let storage: Arc<dyn Storage> = Arc::new(MemStorage::new());
    write_log(&scripts(2, 1, 120), storage.as_ref()).unwrap();
    let opts = TuneOptions {
        min_new_words: 1,
        ..TuneOptions::default()
    };
    let err = tune(
        &storage,
        ModelKind::LinearAction,
        opts,
        1,
        TuneOutput::Result,
    )
    .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}
