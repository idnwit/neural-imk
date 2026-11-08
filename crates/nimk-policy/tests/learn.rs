//! Each learner must learn small toy tasks and round-trip through save/load.

use nimk_policy::{
    Baseline, DENSE_LEN, FeatureExtractor, HashedExtractor, Learner, Linear, PolicyError,
    SPARSE_BUCKETS, TinyMlp,
};
use nimk_types::{CaretRel, DecisionCtx, Features, Lang, Sample};

fn features(bundle: &str, raw: &str, current: Lang) -> Features {
    let ctx = DecisionCtx {
        bundle_id: bundle,
        caret: CaretRel::new(0.3, 0.6),
        raw,
        valid_vi: true,
        has_telex: false,
        prev_langs: &[],
        current,
        secs_since_switch: f32::INFINITY,
    };
    let mut f = Features::default();
    HashedExtractor.extract(&ctx, &mut f);
    f
}

fn sample(bundle: &str, raw: &str, label: Lang) -> Sample {
    Sample {
        features: features(bundle, raw, Lang::En),
        label,
        weight: 1.0,
    }
}

/// "has" is EN in app A and VI in app B, plus filler words per app.
fn app_task() -> Vec<Sample> {
    let mut v = Vec::new();
    for w in ["has", "the", "and", "with"] {
        v.push(sample("com.app.a", w, Lang::En));
    }
    for w in ["has", "the", "and", "with"] {
        v.push(sample("com.app.b", w, Lang::Vi));
    }
    v
}

/// Words with Telex doubles ("aa", "ee", "oo", "dd") are VI, others EN; same app.
fn ngram_task() -> Vec<Sample> {
    let vi = [
        "caan", "tieeng", "nhaa", "ddi", "booo", "vieet", "saang", "deep", "muoon", "ddaau",
    ];
    let en = [
        "can", "ting", "nah", "dig", "boat", "viet", "sang", "dept", "moon", "data",
    ];
    let mut v: Vec<Sample> = vi
        .iter()
        .map(|w| sample("com.app.c", w, Lang::Vi))
        .collect();
    v.extend(en.iter().map(|w| sample("com.app.c", w, Lang::En)));
    v
}

fn accuracy(learner: &dyn Learner, data: &[Sample]) -> f32 {
    let policy = learner.snapshot();
    let ok = data
        .iter()
        .filter(|s| Lang::from_p_vi(policy.predict(&s.features)) == s.label)
        .count();
    ok as f32 / data.len() as f32
}

/// Trains in mini-batches of 4 for `epochs`; returns final accuracy.
fn train(learner: &mut dyn Learner, data: &[Sample], epochs: usize) -> f32 {
    let mut first = None;
    let mut last = 0.0;
    for _ in 0..epochs {
        let mut sum = 0.0;
        for b in data.chunks(4) {
            sum += learner.update(b);
        }
        last = sum;
        first.get_or_insert(sum);
    }
    assert!(
        last <= first.unwrap_or(f32::INFINITY),
        "loss should not grow"
    );
    accuracy(learner, data)
}

fn learners() -> Vec<Box<dyn Learner>> {
    vec![
        Box::new(Linear::new(SPARSE_BUCKETS, DENSE_LEN)),
        Box::new(TinyMlp::new(SPARSE_BUCKETS, DENSE_LEN, 0x5eed)),
    ]
}

#[test]
fn learners_learn_app_task() {
    for mut l in learners() {
        let acc = train(l.as_mut(), &app_task(), 50);
        assert!(acc >= 0.95, "{} app task acc {acc}", l.id());
    }
}

#[test]
fn learners_learn_ngram_task() {
    for mut l in learners() {
        let acc = train(l.as_mut(), &ngram_task(), 50);
        assert!(acc >= 0.95, "{} ngram task acc {acc}", l.id());
    }
}

#[test]
fn baseline_remembers_last_label_per_app() {
    let mut b = Baseline::new();
    let acc = train(&mut b, &app_task(), 1);
    assert!(acc >= 0.95, "baseline acc {acc}");
    let p = b.snapshot();
    assert!(p.predict(&features("com.unknown", "x", Lang::Vi)) > 0.5);
    assert!(p.predict(&features("com.unknown", "x", Lang::En)) < 0.5);
    assert_eq!(p.id(), "baseline@v1");
}

#[test]
fn update_returns_weighted_loss_before_update() {
    let mut b = Baseline::new();
    let mut s = sample("com.app.a", "has", Lang::Vi);
    s.weight = 0.0;
    assert_eq!(b.update(std::slice::from_ref(&s)), 0.0);
    s.weight = 0.3;
    let loss = b.update(&[s]);
    assert!((loss - -(0.1f32.ln())).abs() < 1e-5, "loss {loss}");
    assert_eq!(b.update(&[]), 0.0);
}

fn all_learners() -> Vec<(Box<dyn Learner>, Box<dyn Learner>)> {
    vec![
        (Box::new(Baseline::new()), Box::new(Baseline::new())),
        (
            Box::new(Linear::new(SPARSE_BUCKETS, DENSE_LEN)),
            Box::new(Linear::new(SPARSE_BUCKETS, DENSE_LEN)),
        ),
        (
            Box::new(TinyMlp::new(SPARSE_BUCKETS, DENSE_LEN, 1)),
            Box::new(TinyMlp::new(SPARSE_BUCKETS, DENSE_LEN, 2)),
        ),
    ]
}

#[test]
fn save_load_round_trip_is_identical() {
    let data = ngram_task();
    for (mut a, mut b) in all_learners() {
        for chunk in data.chunks(4) {
            a.update(chunk);
        }
        let bytes = a.save();
        assert_eq!(&bytes[..4], b"NIMK");
        b.load(&bytes).expect("load");
        let (pa, pb) = (a.snapshot(), b.snapshot());
        for s in &data {
            assert_eq!(
                pa.predict(&s.features),
                pb.predict(&s.features),
                "{}",
                a.id()
            );
        }
        assert_eq!(b.save(), bytes);
    }
}

#[test]
fn load_rejects_wrong_kind_version_and_length() {
    let mlp = TinyMlp::new(SPARSE_BUCKETS, DENSE_LEN, 1).save();
    let lin = Linear::new(SPARSE_BUCKETS, DENSE_LEN).save();
    let base = Baseline::new().save();
    for (mut l, _) in all_learners() {
        let own = l.save();
        for other in [&mlp, &lin, &base] {
            if *other != own {
                assert!(
                    matches!(l.load(other), Err(PolicyError::Corrupt(_))),
                    "{}",
                    l.id()
                );
            }
        }
        let mut bad_version = own.clone();
        let ver_at = 4 + 1 + usize::from(own[4]);
        bad_version[ver_at] ^= 1;
        assert!(matches!(l.load(&bad_version), Err(PolicyError::Corrupt(_))));
        let mut extra = own.clone();
        extra.push(0);
        assert!(matches!(l.load(&extra), Err(PolicyError::Corrupt(_))));
        assert!(matches!(
            l.load(&own[..own.len() - 1]),
            Err(PolicyError::Corrupt(_))
        ));
        assert!(matches!(l.load(b"nope"), Err(PolicyError::Corrupt(_))));
        assert!(l.load(&own).is_ok());
    }
}

#[test]
fn dimension_mismatch_is_corrupt() {
    let small = TinyMlp::new(64, DENSE_LEN, 1).save();
    let mut big = TinyMlp::new(SPARSE_BUCKETS, DENSE_LEN, 1);
    assert!(matches!(big.load(&small), Err(PolicyError::Corrupt(_))));
    let small = Linear::new(64, DENSE_LEN).save();
    let mut big = Linear::new(SPARSE_BUCKETS, DENSE_LEN);
    assert!(matches!(big.load(&small), Err(PolicyError::Corrupt(_))));
}

#[test]
fn ids_are_stable() {
    assert_eq!(Linear::new(8, 2).id(), "linear@v1");
    assert_eq!(TinyMlp::new(8, 2, 0).id(), "tiny_mlp@v1");
    assert_eq!(TinyMlp::new(8, 2, 0).snapshot().id(), "tiny_mlp@v1");
    assert_eq!(
        nimk_policy::build(nimk_policy::PolicyKind::Linear, 0)
            .learner
            .id(),
        "linear@v1"
    );
}

/// Random features: 0..40 sparse ids anywhere in the bucket range, dense in [-1, 1].
fn random_features(state: &mut u64, bundle_hash: u32) -> Features {
    let mut next = || {
        *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    let n = (next() % 41) as usize;
    let sparse = (0..n)
        .map(|_| (next() % u64::from(SPARSE_BUCKETS)) as u32)
        .collect();
    let dense = (0..DENSE_LEN)
        .map(|_| (next() % 2001) as f32 / 1000.0 - 1.0)
        .collect();
    let current = if next() % 2 == 0 { Lang::Vi } else { Lang::En };
    Features {
        app: bundle_hash,
        current,
        sparse,
        dense,
    }
}

#[test]
fn untrained_models_are_neutral() {
    let lin = Linear::new(SPARSE_BUCKETS, DENSE_LEN).snapshot();
    let mut state = 42u64;
    let mut inputs: Vec<Features> = (0..1000).map(|i| random_features(&mut state, i)).collect();
    inputs.push(features("com.app.a", "tiee", Lang::Vi));
    for seed in [0, 1, 0x5eed] {
        let mlp = TinyMlp::new(SPARSE_BUCKETS, DENSE_LEN, seed).snapshot();
        for f in &inputs {
            for p in [mlp.predict(f), lin.predict(f)] {
                assert!((p - 0.5).abs() <= 0.05, "untrained p = {p}");
            }
        }
    }
}
