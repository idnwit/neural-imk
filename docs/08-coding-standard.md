# Coding standard

> **Status:** stable · **Updated:** 2026-11-09 · **Code:** `Cargo.toml`, `clippy.toml`, `arch-lint.toml`, `Makefile`

`make check` (fmt, clippy, test, arch-lint) must pass before every commit.
Tools enforce every rule below except those marked as conventions.

## Gate

| Step | Command | Blocks |
| --- | --- | --- |
| `fmt` | `cargo fmt --all -- --check` | Unformatted code |
| `lint` | `cargo clippy --workspace --all-targets -- -D warnings` | Any rustc or clippy warning |
| `test` | `cargo test --workspace` | Failing tests |
| `arch` | `cargo arch-lint check`, then `--rules no-panic-in-lib` | Architecture violations |

## Lints

Lints live in the root `Cargo.toml` (`[workspace.lints]`) and `clippy.toml`. Crates inherit them with `[lints] workspace = true`.
`nimk-ffi` is the exception: it has its own `[lints]` table with the same clippy lints, and it may use `unsafe`.

| Lint | Level | Meaning |
| --- | --- | --- |
| `unsafe_code` | deny | No `unsafe` outside `nimk-ffi` (also catches `#[unsafe(no_mangle)]`) |
| `missing_docs` | warn (error via `-D warnings`) | Public items need docs |
| `clippy::unwrap_used`, `expect_used` | deny | No `unwrap()` or `expect()` |
| `clippy::print_stdout` | deny | No `println!`; `nimk-replay` and `nimk-synth` binaries allow it to print reports |
| `allow-unwrap-in-tests`, `allow-expect-in-tests` | true | Tests may use both (`clippy.toml`) |

Every crate except `nimk-ffi` also has `#![forbid(unsafe_code)]`. `nimk-ffi` has `#![allow(unsafe_code)]` and comments each `unsafe` block.
Use `match` or `if let` instead of `unwrap_or`: arch-lint rule AL013 flags every `unwrap_or`. Deliberate exceptions use `// arch-lint: allow(<rule>) reason="..."`.

## Architecture contract

`arch-lint.toml` (arch-lint 0.11, `preset = "recommended"`, `fail_on = "warning"`) locks layers and the key path by `use` path.

```text
 ffi, replay, synth  -->  engine  -->  telex | policy | hook  -->  types
                             \-->  storage   (true leaf, no nimk deps)
                                      hook  -->  storage
```

Dependencies point down only. `Cargo.toml` also enforces this, since a crate without the dependency cannot `use` it.

### Scopes

| Scope | Paths |
| --- | --- |
| `types`, `storage`, `telex`, `policy`, `hook`, `engine`, `ffi`, `replay`, `synth` | `crates/nimk-<name>/src/**` |
| `pure` | `nimk-types`, `nimk-telex`, `nimk-policy` |
| `eng-composer`, `eng-hotkey`, `eng-labeler`, `eng-trainer`, `eng-logger` | One engine module each (`eng-trainer` includes `trainer/**`) |
| `engine-no-threads` | Engine `src/**` except `trainer.rs`, `trainer/**`, `logger.rs` |
| `non-ffi` | `crates/*/src/**` except `nimk-ffi` |

`crates/*/examples/**` is excluded from arch-lint; clippy still checks it.

### Rules

| Rule | Scope | Forbids | Protects |
| --- | --- | --- | --- |
| `types-is-bottom` | `types` | all other nimk crates | `nimk-types` is the bottom layer |
| `leaf-storage` | `storage` | all other nimk crates (incl. hook) | Storage handles bytes only |
| `leaf-telex` | `telex` | every nimk crate except `types` | Telex depends only on `types` |
| `leaf-policy` | `policy` | storage, telex, hook, engine, ffi, synth, replay, `std::fs`, `std::thread` | Policy only maps `Observation` to action scores |
| `leaf-hook` | `hook` | engine, ffi, policy, telex, synth, replay, `std::fs/net/thread/time/env` | Hooks observe Records; I/O only through given handles; the host owns threads |
| `engine-not-up` | `engine` | ffi, synth, replay, `std::fs` | Engine writes files only via `Storage` |
| `ffi-thin` | `ffi` | policy, telex, hook, synth, replay, `std::fs` | FFI only adapts the engine to C |
| `replay-not-ffi`, `synth-not-ffi` | `replay`, `synth` | `nimk_ffi` | They call the engine directly |
| `pure-no-side-effects` | `pure` | `std::fs/io/net/thread/time/env`, `std::sync` (except `Arc`, `atomic`), `parking_lot` | No I/O, locks or clock in pure crates |
| `engine-event-clock` | `engine` | `std::time`, `std::env` | Event timestamps are the only clock |
| `hot-path-no-io` | `engine-no-threads` | `std::fs`, `std::net`, `std::thread`, `Mutex`, `RwLock`, `Condvar` | Key path has no I/O, locks or threads |
| `engine-uses-transforms` | `engine` | `nimk_telex::transform`, `nimk_policy::decide` | Engine reaches Telex and the model only through `TelexTransform` / `ModelTransform` |
| `eng-<module>-independent` (5 rules) | each engine module | `crate::` and `super::` paths of its siblings and `engine` | Only `engine.rs` wires modules together |
| `no-c-exports-outside-ffi` | `non-ffi` | `#[no_mangle]`, `#[export_name]` | Only `nimk-ffi` exports C symbols |
| `no-unwrap-expect` | repo | `unwrap`/`expect` outside tests | Backs up the clippy lints |
| `no-panic-in-lib` (AL011) | repo | `panic!` and similar in libraries | Runs as the second `make arch` pass |

### Known arch-lint 0.11 limits

| Limit | Workaround |
| --- | --- |
| It does not map `use other_crate::...` to scopes | Layering uses `restrict-use` deny lists on crate paths |
| `[[independent]]` and `[[deny-scope-dep]]` do not resolve `crate::` here | Sibling modules are denied by path text |
| `deny-attr` misses `#[unsafe(no_mangle)]` | The rustc `unsafe_code` lint catches it |
| AL011 is not in the `recommended` preset | `make arch` runs a second pass for it |
| A file scope does not cover its child directory | Add a glob such as `trainer/**` when a module becomes a directory |

### Adding a rule

1. Add the scope or rule to `arch-lint.toml` with a `message` that gives the reason.
2. Write a violating probe, run `make arch`, confirm it is blocked, then delete the probe.
3. Update the tables above and [01-architecture.md](01-architecture.md) in the same commit.

## Conventions

| Convention | Checked by |
| --- | --- |
| The Swift shim holds no decision logic; behavior changes go in Rust with a test | Review |
| An error on the key path is logged and returns `consumed = false`; a key is never lost | Review, golden tests |
| Hot path (`ImeModel::score`, `Encoded::new`) allocates little; p99 under 1 ms | Review, `tests/action_latency.rs` (ignored, run by hand) |
| No wall clock in the engine or pure crates | `engine-event-clock`, `pure-no-side-effects` |
| Files only via `Storage` | `engine-not-up`, `leaf-policy`, `ffi-thin` |
| API, event schema or default config changes update `docs/` in the same commit | Review |
| An incompatible schema change bumps `SCHEMA_VERSION` | Review |

## Tests

- Use fixed seeds and `Threading::Inline` for deterministic tests.
- Heavy learning tests (`GruAction`) run in release; dependencies get `opt-level = 3` so debug tests stay usable ([07](07-build-and-install.md)).
- An engine behavior change needs a golden or integration test in `crates/nimk-engine/tests/`.

| Crate | Test kinds |
| --- | --- |
| `nimk-types` | Unit |
| `nimk-storage` | Unit; one integration suite run on both `MemStorage` and `PlainFsStorage` |
| `nimk-telex` | Unit; `conformance` table; `roundtrip` (`transform(encode(w)) == w`) |
| `nimk-policy` | `action_encode`, `action_model`, `action_learn`, `action_latency` (ignored) |
| `nimk-engine` | Unit; `golden`, `boundary`, `pending`, `commit`, `fresh`, `model_switch`, `shadow`, `teach`, `robust` (random events must not panic) |
| `nimk-ffi` | Unit through the C ABI: null safety, `catch_unwind`, `NimkOut` |
| `nimk-replay` | Unit; integration on a small fixture log |
| `nimk-synth` | Unit (generate, pretrain, eval) and CLI tests |
| Swift | `make swift-test` for `Debouncer`, `IdleCommit`, `StatsText` |

## Experiment code

Offline experiments live in `crates/*/examples/` (for example `nimk-synth/examples/right_context.rs`). They are outside the product contract.
Results go in `docs/research/`.
