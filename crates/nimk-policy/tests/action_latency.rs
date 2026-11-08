//! Latency of `score()` and `update()` (budget: score p99 < 1 ms).
//! Run: `cargo test --release -p nimk-policy --test action_latency -- --ignored --nocapture`.
#![allow(clippy::print_stderr)]

mod common;

use std::hint::black_box;
use std::time::Instant;

use common::{dataset, split_words};
use nimk_policy::{ModelKind, build_action};

fn stats(mut us: Vec<f64>) -> (f64, f64, f64) {
    us.sort_by(f64::total_cmp);
    let mean = us.iter().sum::<f64>() / us.len() as f64;
    let at = |q: f64| us[((us.len() as f64 * q) as usize).min(us.len() - 1)];
    (mean, at(0.5), at(0.99))
}

#[test]
#[ignore]
fn latency() {
    let (words, _) = split_words();
    let data = dataset(&words, 200, 1);
    for kind in [ModelKind::LinearAction, ModelKind::GruAction] {
        let mut learner = build_action(kind, 1);
        let snap = learner.snapshot();
        for ex in data.iter().take(500) {
            black_box(snap.score(&ex.obs));
        }
        let mut t = Vec::with_capacity(5000);
        for ex in data.iter().cycle().take(5000) {
            let t0 = Instant::now();
            black_box(snap.score(black_box(&ex.obs)));
            t.push(t0.elapsed().as_secs_f64() * 1e6);
        }
        let (mean, p50, p99) = stats(t);
        eprintln!("{kind:?} score(): mean {mean:.1} us, p50 {p50:.1}, p99 {p99:.1}");
        if cfg!(not(debug_assertions)) {
            assert!(p99 < 1000.0, "{kind:?} score p99 {p99} us over budget");
        }

        let mut t = Vec::new();
        for chunk in data.chunks_exact(32).take(200) {
            let t0 = Instant::now();
            black_box(learner.update(chunk));
            t.push(t0.elapsed().as_secs_f64() * 1e6);
        }
        let (mean, p50, p99) = stats(t);
        eprintln!("{kind:?} update(32): mean {mean:.1} us, p50 {p50:.1}, p99 {p99:.1}");

        let t0 = Instant::now();
        for _ in 0..100 {
            black_box(learner.snapshot());
        }
        eprintln!(
            "{kind:?} snapshot(): {:.1} us",
            t0.elapsed().as_secs_f64() * 1e4
        );
    }
}
