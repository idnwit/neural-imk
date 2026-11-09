# 0009. Neural network backend: burn 0.22.1 (flex)

> **Status:** accepted · **Date:** 2026-11-08

## Context

[0007](0007-neural-action-model.md) needs a neural model that runs per key with p99 < 1 ms and learns online inside the IME process.
The backend must infer and train in pure Rust: no Python, no external runtime, no GPU.

A small GRU (embedding 32, hidden 64) was measured on an M4 Pro:

| Candidate | GRU inference | Adam step, batch 32 | Note |
| --- | --- | --- | --- |
| `burn` 0.22.1 `flex` | p99 108 us | about 2.5 ms | `Module`, autodiff, optimizer, record built in |
| `candle` | 96 / 127 us | about 2.0 ms | same order as burn |
| `tract` | - | - | inference only |
| `ort` | - | - | training needs Python-built artifacts |
| `tch` | - | - | libtorch too heavy to bundle |
| `dfdx` | - | - | unmaintained |

Only `burn` and `candle` are viable, and the gap is small against the 1 ms budget.

## Decision

- Use `burn = "=0.22.1"`, `default-features = false`, features `std`, `flex`, `autodiff`, `optim`, for `GruAction` in `nimk-policy`.
- No `rayon`: single-threaded kernels do not compete with the typing app for CPU. No GPU backend.
- Pin the exact version because burn's API changes between minor releases.
- The hand-written `LinearAction` stays as baseline and fallback; both sit behind `ImeModel`/`ImeLearner`.
- `GruAction` keeps its own seeded RNG, because burn's device RNG is process-global.
- Debug builds are about 40x slower, so the root `Cargo.toml` sets `[profile.dev.package."*"] opt-level = 3` ([07](../07-build-and-install.md)).

## Consequences

- Measured in the product: `score` mean 166 us (p99 about 170-186 us); `update` batch 32 takes 3.6 ms on the trainer thread.
- `GruAction` snapshots use burn's record format, so bytes are not deterministic; compare snapshots by score, not bytes. Adam state is not saved.
- Upgrading burn is a deliberate act that reruns the learning tests and benchmarks.
- If `flex` shows numeric or performance bugs, fall back to `candle` or to `LinearAction` without changing the engine.
- Lowering the trainer thread's QoS is not done yet (open).
- Build time and `libnimk_ffi.a` size grow; `GruAction` learning tests run only in release.
