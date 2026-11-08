use std::sync::Arc;

use nimk_engine::EngineConfig;
use nimk_policy::ModelKind;
use nimk_storage::MemStorage;

use super::*;
use crate::{Corpus, MixProfile, SynthConfig, generate, pretrain};

/// Variant-typed `theme` (thêm) is Vietnamese after Vietnamese context and
/// English after English context, once the model is trained on variants.
#[test]
fn theme_follows_its_context() {
    let cfg = EngineConfig {
        policy: ModelKind::LinearAction,
        ..EngineConfig::default()
    };
    let opts = PretrainOptions {
        epochs: 2,
        eval_frac: 0.1,
        ..PretrainOptions::default()
    };
    let synth = SynthConfig {
        seed: 7,
        sessions: 40,
        words_per_session: 300,
        mix: MixProfile {
            p_variant: 0.5,
            ..MixProfile::default()
        },
        ..SynthConfig::default()
    };
    let storage: Arc<dyn Storage> = Arc::new(MemStorage::new());
    pretrain(
        &generate(&synth, &Corpus::builtin()),
        cfg.clone(),
        opts,
        Arc::clone(&storage),
    )
    .unwrap();
    let vi = probe_word(
        cfg.clone(),
        opts,
        &storage,
        &[(Lang::Vi, "Tôi"), (Lang::Vi, "muốn")],
        "theme",
    )
    .unwrap();
    let en = probe_word(
        cfg,
        opts,
        &storage,
        &[(Lang::En, "change"), (Lang::En, "the"), (Lang::En, "dark")],
        "theme",
    )
    .unwrap();
    assert!(vi.p_vi.is_some() && en.p_vi.is_some());
    assert!(vi.p_vi > en.p_vi, "vi {vi:?} en {en:?}");
}
