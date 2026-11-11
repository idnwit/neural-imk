//! The fine-tuning button sequence through the C ABI: count the new data,
//! save, fine-tune, install. Uses synthetic data, so it lives outside the
//! crate's `src` (the FFI layer itself never touches synthetic data).
#![allow(clippy::unwrap_used, clippy::expect_used, unsafe_code)]

use std::ffi::CString;
use std::ptr;
use std::sync::Arc;

use nimk_ffi::{
    NIMK_TUNE_OK, NIMK_TUNE_TOO_LITTLE, NimkOut, NimkTunePending, NimkTuneReport, nimk_new,
    nimk_shutdown, nimk_tune_install, nimk_tune_pending, nimk_tune_prepare, nimk_tune_run,
};
use nimk_policy::ModelKind;
use nimk_storage::{PlainFsStorage, Storage};
use nimk_synth::{Corpus, PretrainOptions, SynthConfig, generate, pretrain, write_log};

const MIN: u64 = 1;
const LINEAR: u8 = 0;

fn blank_out() -> NimkOut {
    NimkOut {
        consumed: false,
        commit: ptr::null(),
        marked: ptr::null(),
        mode: 0,
        overlay: 0,
    }
}

#[test]
fn button_sequence_pending_prepare_run_install() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = CString::new(tmp.path().to_str().unwrap()).unwrap();
    let storage: Arc<dyn Storage> = Arc::new(PlainFsStorage::open(tmp.path()).unwrap());
    let synth = |seed, sessions| {
        generate(
            &SynthConfig {
                seed,
                sessions,
                words_per_session: 100,
                ..SynthConfig::default()
            },
            &Corpus::builtin(),
        )
    };
    let cfg = nimk_engine::EngineConfig {
        policy: ModelKind::LinearAction,
        ..nimk_engine::EngineConfig::default()
    };
    let opts = PretrainOptions {
        epochs: 1,
        eval_frac: 0.0,
        ..PretrainOptions::default()
    };
    pretrain(&synth(1, 3), cfg, opts, Arc::clone(&storage)).unwrap();
    let logged = synth(5, 3);
    let words: u64 = logged.iter().map(|s| s.words.len() as u64).sum();
    write_log(&logged, storage.as_ref()).unwrap();

    let mut p = NimkTunePending::default();
    // SAFETY: `dir` is NUL-terminated and outlives the call; null
    // `models_dir` is allowed; `p` is writable.
    assert!(unsafe { nimk_tune_pending(dir.as_ptr(), ptr::null(), &mut p) });
    assert_eq!((p.new_words, p.runs, p.last_unix_ms), (words, 0, 0));
    assert_eq!(p.total_words, words);

    // SAFETY: `dir` is NUL-terminated; null `models_dir` is allowed.
    let core = unsafe { nimk_new(dir.as_ptr(), ptr::null(), c"t".as_ptr()) };
    assert!(!core.is_null());
    let mut out = blank_out();
    // SAFETY: live core from `nimk_new`; `out` is writable.
    assert!(unsafe { nimk_tune_prepare(core, 1.0, &mut out) });

    let mut report = NimkTuneReport::default();
    // SAFETY: as for `nimk_tune_pending`; `report` is writable.
    let code = unsafe { nimk_tune_run(dir.as_ptr(), ptr::null(), LINEAR, MIN, 1000, &mut report) };
    assert_eq!(code, NIMK_TUNE_OK);
    assert_eq!(report.new_words, words);
    assert!(report.keys > 0);

    // SAFETY: live core; `out` is writable.
    assert!(unsafe { nimk_tune_install(core, 2.0, &mut out) });
    // The result is consumed: a second install has nothing to do.
    // SAFETY: as above.
    assert!(!unsafe { nimk_tune_install(core, 3.0, &mut out) });

    // SAFETY: as for the first `nimk_tune_pending`.
    assert!(unsafe { nimk_tune_pending(dir.as_ptr(), ptr::null(), &mut p) });
    assert_eq!((p.new_words, p.runs, p.last_unix_ms), (0, 1, 1000));
    // Nothing new: a second run is refused.
    // SAFETY: as for the first `nimk_tune_run`.
    let code = unsafe {
        nimk_tune_run(
            dir.as_ptr(),
            ptr::null(),
            LINEAR,
            MIN,
            2000,
            ptr::null_mut(),
        )
    };
    assert_eq!(code, NIMK_TUNE_TOO_LITTLE);
    // SAFETY: live core, freed here.
    assert!(unsafe { nimk_shutdown(core) });
}
