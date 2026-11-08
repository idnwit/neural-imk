//! Hot-path checks: `extract` (after warm-up) and `predict` must not allocate,
//! and TinyMlp predict latency is reported (`cargo test --release -p
//! nimk-policy --test alloc -- --nocapture`).
//!
//! A counting global allocator needs `unsafe impl GlobalAlloc`; it only
//! forwards to `System`, so the workspace `unsafe_code` lint is relaxed here.
#![allow(unsafe_code, clippy::print_stdout)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::time::Instant;

use nimk_policy::{
    DENSE_LEN, FeatureExtractor, HashedExtractor, Learner, Linear, SPARSE_BUCKETS, TinyMlp,
};
use nimk_types::{CaretRel, DecisionCtx, Features, Lang, Sample};

struct Counting;

thread_local! {
    /// Per-thread so parallel tests do not disturb each other.
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: every call forwards unchanged to `System`; the counter is a
// thread-local `Cell` with a const initializer, which never allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.with(|c| c.set(c.get() + 1));
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was allocated by `System` via `alloc` above.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.with(|c| c.set(c.get() + 1));
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocs() -> usize {
    ALLOCS.with(Cell::get)
}

fn ctx<'a>(raw: &'a str, prev: &'a [Lang]) -> DecisionCtx<'a> {
    DecisionCtx {
        bundle_id: "com.apple.TextEdit",
        caret: CaretRel::new(0.5, 0.5),
        raw,
        valid_vi: true,
        has_telex: true,
        prev_langs: prev,
        current: Lang::Vi,
        secs_since_switch: 3.0,
    }
}

#[test]
fn extract_and_predict_do_not_allocate() {
    let words = ["tieengs", "has", "", "Vieejt", "nguowif", "x"];
    let prev = [Lang::Vi, Lang::En];
    let mut f = Features::default();
    // Warm-up with the longest word sizes the buffers.
    HashedExtractor.extract(
        &ctx("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", &prev),
        &mut f,
    );

    let mut mlp = TinyMlp::new(SPARSE_BUCKETS, DENSE_LEN, 1);
    let mut lin = Linear::new(SPARSE_BUCKETS, DENSE_LEN);
    let batch = [Sample {
        features: f.clone(),
        label: Lang::Vi,
        weight: 1.0,
    }];
    mlp.update(&batch);
    lin.update(&batch);
    let policies = [
        mlp.snapshot(),
        lin.snapshot(),
        nimk_policy::Baseline::new().snapshot(),
    ];

    let before = allocs();
    let mut acc = 0.0;
    for w in words {
        HashedExtractor.extract(&ctx(w, &prev), &mut f);
        for p in &policies {
            acc += p.predict(&f);
        }
    }
    let after = allocs();
    black_box(acc);
    assert_eq!(
        after - before,
        0,
        "hot path allocated {} times",
        after - before
    );
}

#[test]
fn tiny_mlp_predict_latency() {
    let mut f = Features::default();
    HashedExtractor.extract(&ctx("tieengs", &[Lang::Vi]), &mut f);
    let policy = TinyMlp::new(SPARSE_BUCKETS, DENSE_LEN, 1).snapshot();
    let iters = 20_000u32;
    let start = Instant::now();
    let mut acc = 0.0;
    for _ in 0..iters {
        acc += policy.predict(black_box(&f));
    }
    black_box(acc);
    let us = start.elapsed().as_secs_f64() * 1e6 / f64::from(iters);
    println!(
        "tiny_mlp predict: {us:.2} us/predict ({} sparse ids)",
        f.sparse.len()
    );
    // Generous bound so debug builds pass; the target is < 100 us in release.
    assert!(us < 5_000.0);
}
