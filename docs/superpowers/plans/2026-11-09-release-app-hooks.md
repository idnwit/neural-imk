# Release App + Transform/Hook Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship NeuralIMK as a drag-to-Applications DMG whose companion app installs the IME and shows statistics, on top of an engine refactored into Transforms (buffer-changing) and Hooks (observe-only).

**Architecture:** Telex and the model decision become pure `Transform`s called by the engine. A new `nimk-hook` crate defines `Hook`s that consume the existing `Record` stream on a background thread owned by `engine/hookbus.rs` (which replaces `logger.rs`); the event log, per-model stats and a new usage counter are hooks. A SwiftUI app in `/Applications` embeds the IME bundle, installs it into `~/Library/Input Methods`, and reads hook outputs from the IME data dir.

**Tech Stack:** Rust 2024 workspace (burn, serde), cbindgen, Swift 5 / SwiftUI / Swift Charts (macOS 13+), InputMethodKit, Carbon TIS, `hdiutil` / optional `create-dmg`, `codesign`, `notarytool`.

**Spec:** `docs/superpowers/specs/2026-11-09-release-app-hooks-design.md` (read it before any task).

## Global Constraints

- `make check` (fmt, clippy `-D warnings`, tests, arch-lint two passes) passes at the end of every task.
- No `unwrap`/`expect`/`panic!` outside tests; no `unwrap_or` (arch-lint AL013) — use `match` / `if let`.
- Engine and hooks never read the wall clock or env; the only clock is event timestamps.
- `SCHEMA_VERSION` stays `1`; new event fields use `#[serde(default)]`.
- IME bundle id stays `com.giangndm.inputmethod.NeuralIMK`; the companion app is `com.giangndm.NeuralIMK`.
- Minimum macOS 13.0. Docs are English only; user-facing app strings are Vietnamese.
- Commit messages end with:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01L5KhG6PziZ9RixbCVtjqWJ
  ```
- Work on branch `release-app`; never push.

## Review Focus

1. Hook thread panics or a hook's storage write fails → typing continues, a diagnostic appears, other hooks keep running (Task 4 test `panicking_hook_is_disabled_others_continue`).
2. User `config.toml` that has tables, comments, or the keys inside a table → GUI rewrite only touches top-level `double_shift`/`ctrl_space`, preserves everything else, inserts before the first table header (Task 8 tests).
3. IME running while the app updates it → old process is terminated before the bundle is replaced; a failed copy leaves the old install intact (Task 10 `Installer` uses temp dir + rename; `VersionPolicy` test for missing/older/newer/equal).
4. Usage across midnight and timezone offsets → words land on the local day derived from `wall_unix_ms + utc_offset_min` (Task 6 test `day_bucket_uses_local_offset`).
5. Legacy `stats/<kind>` blob from existing users → migrated once, numbers unchanged in the menu (Task 5 test `legacy_stats_blob_is_migrated_once`).

---

### Task 1: Characterization fixture for the pipeline

Pin today's behaviour before refactoring: decision stream, raw event-log lines and model stats for a fixed synthetic session, per model kind.

**Files:**
- Create: `crates/nimk-replay/tests/pipeline_fixture.rs`
- Create (generated): `crates/nimk-replay/tests/fixtures/pipeline_linear_action.txt`, `crates/nimk-replay/tests/fixtures/pipeline_gru_action.txt`

**Interfaces:**
- Consumes: `nimk_synth::{generate, write_log, SynthConfig, Corpus}`, `nimk_engine::{Engine, EngineConfig, SessionInfo, Threading}`, `MemStorage`.
- Produces: `cargo test -p nimk-replay --release --test pipeline_fixture` — the gate every later engine task must keep green. `NIMK_BLESS=1` rewrites fixtures (only allowed in this task and, for added lines only, Task 5).

- [ ] **Step 1: Write the test**

```rust
//! Characterization: the decision stream, the raw log lines and the model
//! statistics of a fixed synthetic session must not change across the
//! Transform/Hook refactor. `NIMK_BLESS=1` rewrites the fixtures.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fmt::Write as _;
use std::sync::Arc;

use nimk_engine::{AppContext, Engine, EngineConfig, SessionInfo, Threading};
use nimk_policy::ModelKind;
use nimk_storage::{MemStorage, Storage};
use nimk_synth::{Corpus, SynthConfig, generate, write_log};
use nimk_types::{Boundary, Event, Record};

/// Record types whose raw bytes are pinned (session_start is excluded: it
/// gains `utc_offset_min` in Task 6; model_score is new in Task 5).
const PINNED: &[&str] = &[
    "\"type\":\"context\"", "\"type\":\"key\"", "\"type\":\"decision\"",
    "\"type\":\"manual_switch\"", "\"type\":\"word_commit\"", "\"type\":\"label\"",
    "\"type\":\"model_update\"",
];

fn source_records() -> Vec<Record> {
    let cfg = SynthConfig { seed: 7, sessions: 1, words_per_session: 120, ..SynthConfig::default() };
    let scripts = generate(&cfg, &Corpus::builtin());
    let src = MemStorage::new();
    write_log(&scripts, &src).unwrap();
    let source = src.event_source().unwrap();
    let mut out = Vec::new();
    for id in source.segments().unwrap() {
        for line in source.read_segment(&id).unwrap() {
            out.push(serde_json::from_slice::<Record>(&line.unwrap()).unwrap());
        }
    }
    out
}

fn run(kind: ModelKind) -> String {
    let storage = MemStorage::new();
    let cfg = EngineConfig { policy: kind, threading: Threading::Inline, hotkeys: false, ..EngineConfig::default() };
    let info = SessionInfo { id: "fixture".into(), wall_unix_ms: 1_794_111_700_000, app_version: "fixture".into() };
    let mut engine = Engine::new(cfg, Arc::new(storage.clone()) as Arc<dyn Storage>, info);
    for r in source_records() {
        match &r.event {
            Event::Context { bundle_id, field_id, caret } => {
                engine.on_context(&AppContext { bundle_id: bundle_id.clone(), field_id: *field_id, caret: *caret, timestamp: r.t });
            }
            Event::Key { key, .. } => { engine.on_key(key); }
            Event::Label { target, label, weight, source } => engine.inject_label(*target, *label, *weight, *source),
            Event::WordCommit { boundary: Boundary::External, .. } => { engine.commit_pending(r.t); }
            _ => {}
        }
    }
    let mut out = String::new();
    for k in ModelKind::ALL {
        writeln!(out, "stats {} {:?}", k.name(), engine.model_stats(*k)).unwrap();
    }
    engine.shutdown().unwrap();
    let source = storage.event_source().unwrap();
    for id in source.segments().unwrap() {
        for line in source.read_segment(&id).unwrap() {
            let line = String::from_utf8(line.unwrap()).unwrap();
            if PINNED.iter().any(|p| line.contains(p)) {
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out
}

fn check(kind: ModelKind) {
    let path = format!("{}/tests/fixtures/pipeline_{}.txt", env!("CARGO_MANIFEST_DIR"), kind.name());
    let got = run(kind);
    if std::env::var_os("NIMK_BLESS").is_some() {
        std::fs::create_dir_all(format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"))).unwrap();
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).expect("fixture missing; run with NIMK_BLESS=1");
    assert!(got == want, "pipeline output changed for {}; diff {path} against a blessed run", kind.name());
}

#[test]
fn linear_pipeline_is_unchanged() { check(ModelKind::LinearAction); }

#[test]
fn gru_pipeline_is_unchanged() { check(ModelKind::GruAction); }
```

If a name differs from the real API (e.g. `ModelKind::ALL` is a slice of values, `write_log` signature), adapt the test to the real API without changing what it pins. `nimk-replay` already dev-depends on `nimk-synth`.

- [ ] **Step 2: Bless and verify determinism**

Run: `NIMK_BLESS=1 cargo test -p nimk-replay --release --test pipeline_fixture && cargo test -p nimk-replay --release --test pipeline_fixture`
Expected: both PASS; run the second command twice more — it must pass every time (determinism). Fixture files must contain `decision` lines and two `stats` lines.

- [ ] **Step 3: Commit**

```bash
git add crates/nimk-replay/tests/pipeline_fixture.rs crates/nimk-replay/tests/fixtures
git commit -m "Test: characterization fixture for the engine pipeline"
```

---

### Task 2: Transforms (Telex, Model)

**Files:**
- Modify: `crates/nimk-types/src/lib.rs` (add module `transform`), Create: `crates/nimk-types/src/transform.rs`
- Modify: `crates/nimk-telex/src/lib.rs` (add `TelexTransform`, `BufferState`)
- Create: `crates/nimk-policy/src/transform.rs`, Modify: `crates/nimk-policy/src/lib.rs`
- Modify: `crates/nimk-engine/src/engine/flow.rs` (`refresh_telex`, `hysteresis`, `model_step`, focus prior near line 458), `crates/nimk-engine/src/engine.rs` (field `model: Arc<dyn ImeModel>` → keep, build `ModelTransform` per call or hold one)
- Modify: `arch-lint.toml` (rule `engine-uses-transforms`)
- Test: `crates/nimk-telex/src/lib.rs` tests, `crates/nimk-policy/tests/transform.rs`

**Interfaces:**
- Produces:
  ```rust
  // nimk_types
  pub trait Transform { type Input: ?Sized; type Output; fn apply(&self, input: &Self::Input) -> Self::Output; }
  // nimk_telex
  pub struct BufferState { pub vi: String, pub valid_vi: bool, pub viable_vi: bool }
  pub struct TelexTransform;
  impl Transform for TelexTransform { type Input = str; type Output = BufferState; }
  // nimk_policy
  pub struct ModelInput { pub obs: Observation, pub base: Lang, pub locked: bool, pub margin: f32,
                          pub structural: Option<EditAction>, pub prior: bool }
  pub struct ModelOutput { pub action: Action, pub p_vi: Option<f32> }
  pub struct ModelTransform { model: Arc<dyn ImeModel> }
  impl ModelTransform { pub fn new(model: Arc<dyn ImeModel>) -> Self; }
  impl Transform for ModelTransform { type Input = ModelInput; type Output = ModelOutput; }
  pub fn hysteresis(base: Lang, p: f32, margin: f32) -> (Option<f32>, Lang);
  ```

- [ ] **Step 1: Trait in nimk-types**

`crates/nimk-types/src/transform.rs`:

```rust
//! A pipeline stage that may change the buffer or the output: a pure
//! function of its input (state lives in the engine and is passed in).
//! Observers that must not change anything are hooks (`nimk-hook`).

/// One pure pipeline stage.
pub trait Transform {
    /// Data the stage reads.
    type Input: ?Sized;
    /// Data the stage produces.
    type Output;
    /// Runs the stage. Must not panic and must be deterministic.
    fn apply(&self, input: &Self::Input) -> Self::Output;
}
```

Export from `lib.rs`: `mod transform; pub use transform::Transform;`.

- [ ] **Step 2: Failing tests for the two transforms**

In `crates/nimk-telex/src/lib.rs` tests:

```rust
#[test]
fn telex_transform_matches_transform() {
    use nimk_types::Transform;
    for raw in ["vieejt", "has", "", "ddaay"] {
        let tr = transform(raw);
        let st = TelexTransform.apply(raw);
        assert_eq!((st.vi.as_str(), st.valid_vi, st.viable_vi), (tr.text.as_str(), tr.valid_vi, tr.viable_vi));
    }
}
```

`crates/nimk-policy/tests/transform.rs`:

```rust
#![allow(clippy::unwrap_used)]
use std::sync::Arc;
use nimk_policy::{ImeModel, ModelInput, ModelTransform, hysteresis};
use nimk_types::{ActionMask, ActionScores, EditAction, Lang, Observation, Transform};

struct P(f32);
impl ImeModel for P {
    fn id(&self) -> &str { "p@v0" }
    fn score(&self, _o: &Observation) -> ActionScores { ActionScores { p_vi: self.0, ..ActionScores::default() } }
}

fn input(base: Lang, locked: bool, structural: Option<EditAction>, prior: bool) -> ModelInput {
    let mut obs = Observation::default();
    obs.mask = ActionMask { append: true, pop: false, commit: false };
    ModelInput { obs, base, locked, margin: 0.1, structural, prior }
}

#[test]
fn hysteresis_holds_inside_margin() {
    assert_eq!(hysteresis(Lang::En, 0.55, 0.1), (Some(0.55), Lang::En));
    assert_eq!(hysteresis(Lang::En, 0.7, 0.1), (Some(0.7), Lang::Vi));
    assert_eq!(hysteresis(Lang::Vi, f32::NAN, 0.1), (None, Lang::Vi));
}

#[test]
fn lock_keeps_base_and_commit_uses_raw_p() {
    let t = ModelTransform::new(Arc::new(P(0.55)));
    let locked = t.apply(&input(Lang::En, true, Some(EditAction::Append), false));
    assert_eq!(locked.action.render, Lang::En);
    let commit = t.apply(&input(Lang::En, false, Some(EditAction::Commit), false));
    assert_eq!(commit.action.render, Lang::Vi); // commit step ignores hysteresis
    assert_eq!(commit.action.edit, EditAction::Commit);
}

#[test]
fn prior_uses_hysteresis_only() {
    let t = ModelTransform::new(Arc::new(P(0.9)));
    let out = t.apply(&input(Lang::En, false, None, true));
    assert_eq!(out.action.render, Lang::Vi);
    assert_eq!(out.action.edit, EditAction::Pass);
}
```

If `ActionScores`/`Observation` lack `Default`, build them field by field the way `crates/nimk-engine/tests/common/mod.rs` does.

- [ ] **Step 3: Run to see failure**

Run: `cargo test -p nimk-telex -p nimk-policy --test transform`
Expected: FAIL to compile (`TelexTransform`, `ModelTransform` not found).

- [ ] **Step 4: Implement**

`nimk-telex` (`lib.rs`):

```rust
/// Telex rendering of a raw buffer and its flags.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BufferState {
    /// Vietnamese rendering.
    pub vi: String,
    /// Complete valid Vietnamese syllable.
    pub valid_vi: bool,
    /// Could still become a valid syllable.
    pub viable_vi: bool,
}

/// The Telex stage: raw keys to Vietnamese rendering.
#[derive(Debug, Clone, Copy, Default)]
pub struct TelexTransform;

impl nimk_types::Transform for TelexTransform {
    type Input = str;
    type Output = BufferState;
    fn apply(&self, raw: &str) -> BufferState {
        let tr = transform(raw);
        BufferState { vi: tr.text, valid_vi: tr.valid_vi, viable_vi: tr.viable_vi }
    }
}
```

`nimk-policy/src/transform.rs` — move the logic of `Engine::hysteresis` and the model branch of `model_step` here verbatim:

```rust
//! The model stage: scores the observation and picks the action, with the
//! display hysteresis, the manual-switch lock and the commit-step rule.

use std::sync::Arc;

use nimk_types::{Action, EditAction, Lang, Observation, Transform};

use crate::{ImeModel, decide};

/// Input of [`ModelTransform`].
#[derive(Debug, Clone)]
pub struct ModelInput {
    pub obs: Observation,
    /// Rendering shown before this key (or the sticky mode for a prior).
    pub base: Lang,
    /// A manual switch locked the rendering for this word.
    pub locked: bool,
    /// Hysteresis margin in force.
    pub margin: f32,
    /// The only legal edit, when the mask leaves no choice.
    pub structural: Option<EditAction>,
    /// Focus prior: only the rendering matters; edit is `Pass`.
    pub prior: bool,
}

/// Output of [`ModelTransform`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelOutput {
    pub action: Action,
    /// Sanitized probability for the log; `None` if not finite.
    pub p_vi: Option<f32>,
}

/// Leave `base` only if `|p - 0.5| > margin`.
#[must_use]
pub fn hysteresis(base: Lang, p: f32, margin: f32) -> (Option<f32>, Lang) {
    if !p.is_finite() {
        return (None, base);
    }
    let p = p.clamp(0.0, 1.0);
    let chosen = if (p - 0.5).abs() > margin { Lang::from_p_vi(p) } else { base };
    (Some(p), chosen)
}

/// The model stage around one read-only snapshot.
#[derive(Clone)]
pub struct ModelTransform {
    model: Arc<dyn ImeModel>,
}

impl ModelTransform {
    #[must_use]
    pub fn new(model: Arc<dyn ImeModel>) -> Self {
        Self { model }
    }
}

impl Transform for ModelTransform {
    type Input = ModelInput;
    type Output = ModelOutput;

    fn apply(&self, input: &ModelInput) -> ModelOutput {
        let scores = self.model.score(&input.obs);
        let (p, held) = hysteresis(input.base, scores.p_vi, input.margin);
        if input.prior {
            return ModelOutput { action: Action { edit: EditAction::Pass, render: held }, p_vi: p };
        }
        let edit = match input.structural {
            Some(edit) => edit,
            None => decide(&scores, input.obs.mask).edit,
        };
        let render = if input.locked {
            input.base
        } else if edit == EditAction::Commit {
            match p { Some(p) => Lang::from_p_vi(p), None => input.base }
        } else {
            held
        };
        ModelOutput { action: Action { edit, render }, p_vi: p }
    }
}
```

Export from `nimk-policy/src/lib.rs`: `mod transform; pub use transform::{ModelInput, ModelOutput, ModelTransform, hysteresis};`. Add `Debug` manually if needed (`Arc<dyn ImeModel>` may not be `Debug`; skip derive).

Engine (`flow.rs`):
- `refresh_telex`: `let st = nimk_telex::TelexTransform.apply(self.composer.raw()); self.composer.set_vi(&st.vi); self.telex = TelexFlags { valid_vi: st.valid_vi, viable_vi: st.viable_vi };` (import `nimk_types::Transform`).
- Delete `Engine::hysteresis`.
- `model_step` `None =>` branch becomes:
  ```rust
  None => {
      let out = ModelTransform::new(Arc::clone(&self.model)).apply(&ModelInput {
          obs: obs.clone(), base: self.shown, locked: self.locked,
          margin: self.live.switch_margin, structural, prior: false,
      });
      (out.action, out.p_vi)
  }
  ```
  Keep `obs` available afterwards if later code uses it (clone only if needed; otherwise move it in).
- Focus prior (around line 458): replace `score` + `hysteresis` with `apply` on `ModelInput { prior: true, base: self.mode, locked: false, structural: None, .. }`, using `out.p_vi` and `out.action.render`.
- `ModelTransform::new` is a cheap `Arc` clone; no per-key allocation beyond today.

- [ ] **Step 5: arch-lint rule**

Append to `arch-lint.toml` after `hot-path-no-io`:

```toml
[[restrict-use]]
name = "engine-uses-transforms"
scope = "engine"
deny = ["nimk_telex::transform", "nimk_policy::decide"]
message = "The engine reaches Telex and the model only through TelexTransform / ModelTransform."
```

Update the `docs/08-coding-standard.md` rules table with one row for it.

- [ ] **Step 6: Verify**

Run: `cargo test -p nimk-telex -p nimk-policy && cargo test -p nimk-replay --release --test pipeline_fixture && make check`
Expected: all PASS, fixture unchanged (no bless).

- [ ] **Step 7: Commit**

```bash
git add -A crates arch-lint.toml docs/08-coding-standard.md
git commit -m "Engine: Telex and model decision as Transforms"
```

---

### Task 3: `nimk-hook` crate, event additions, arch-lint scope

**Files:**
- Create: `crates/nimk-hook/Cargo.toml`, `crates/nimk-hook/src/lib.rs`, `crates/nimk-hook/src/cx.rs`
- Modify: root `Cargo.toml` (workspace member + `nimk-hook = { path = "crates/nimk-hook" }` in `[workspace.dependencies]`)
- Modify: `crates/nimk-types/src/event.rs` (`SessionStart.utc_offset_min`, `Event::ModelScore`), move `StatCategory` from `crates/nimk-engine/src/trainer/stats.rs` to `crates/nimk-types/src/stat.rs` (re-export from `nimk-engine` as today so the public API is unchanged)
- Modify: `arch-lint.toml`, `docs/08-coding-standard.md`
- Test: `crates/nimk-hook/src/cx.rs` unit tests, `crates/nimk-types` serde test

**Interfaces:**
- Produces:
  ```rust
  pub trait Hook: Send {
      fn name(&self) -> &'static str;
      fn init(&mut self, cx: &mut HookCx<'_>) {}
      fn on_record(&mut self, rec: &Record, cx: &mut HookCx<'_>);
      fn flush(&mut self, cx: &mut HookCx<'_>) -> io::Result<()> { Ok(()) }
  }
  pub struct HookCx<'a> { .. }
  impl<'a> HookCx<'a> {
      pub fn new(name: &'static str, blobs: &'a dyn BlobStore, outbox: &'a mut Vec<HookMsg>) -> Self;
      pub fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>>;
      pub fn put(&self, key: &str, bytes: &[u8]) -> io::Result<()>;
      pub fn get_legacy(&self, key: &str) -> io::Result<Option<Vec<u8>>>;
      pub fn summary(&mut self, json: String);
      pub fn diagnostic(&mut self, msg: String);
  }
  pub enum HookMsg { Summary { hook: &'static str, json: String }, Diagnostic(String) }
  // nimk_types
  Event::SessionStart { .., #[serde(default)] utc_offset_min: i32 }
  Event::ModelScore { kind: String, word_id: u64, category: StatCategory, correct: bool,
                      display_hits: u16, display_total: u16, flips: u16, amend: bool,
                      #[serde(default)] served: bool }
  pub enum StatCategory { Terminal, Ide, Browser, Editor, Other }  // serde snake_case
  ```

- [ ] **Step 1: Failing tests**

`crates/nimk-hook/src/cx.rs` tests:

```rust
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use nimk_storage::{MemStorage, Storage};
    use super::*;

    #[test]
    fn keys_are_scoped_under_hooks_name() {
        let s = MemStorage::new();
        let mut out = Vec::new();
        let cx = HookCx::new("usage", s.blobs(), &mut out);
        cx.put("2026-11-09", b"{}").unwrap();
        assert_eq!(s.blobs().get("hooks/usage/2026-11-09").unwrap().as_deref(), Some(&b"{}"[..]));
        assert_eq!(cx.get("2026-11-09").unwrap().as_deref(), Some(&b"{}"[..]));
    }

    #[test]
    fn legacy_reads_any_key_and_summaries_queue() {
        let s = MemStorage::new();
        s.blobs().put("stats/gru_action", b"x").unwrap();
        let mut out = Vec::new();
        let mut cx = HookCx::new("model_stats", s.blobs(), &mut out);
        assert_eq!(cx.get_legacy("stats/gru_action").unwrap().as_deref(), Some(&b"x"[..]));
        cx.summary("{}".into());
        cx.diagnostic("d".into());
        assert_eq!(out.len(), 2);
    }
}
```

`crates/nimk-types` test (in `event.rs` tests or `tests/`):

```rust
#[test]
fn old_session_start_parses_with_zero_offset_and_model_score_round_trips() {
    let old = r#"{"v":1,"seq":0,"t":1.0,"session":"s","type":"session_start","wall_unix_ms":5,"app_version":"a","config_json":"{}","policy_id":"p"}"#;
    let r: Record = serde_json::from_str(old).unwrap();
    assert!(matches!(r.event, Event::SessionStart { utc_offset_min: 0, .. }));
    let ms = Event::ModelScore { kind: "gru_action".into(), word_id: 7, category: StatCategory::Terminal,
        correct: true, display_hits: 3, display_total: 4, flips: 1, amend: false, served: true };
    let json = serde_json::to_string(&ms).unwrap();
    assert!(json.contains(r#""type":"model_score""#) && json.contains(r#""category":"terminal""#));
    assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), ms);
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p nimk-hook -p nimk-types`
Expected: FAIL (crate / variants missing).

- [ ] **Step 3: Implement**

`crates/nimk-hook/Cargo.toml`:

```toml
[package]
name = "nimk-hook"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[lints]
workspace = true

[dependencies]
nimk-types.workspace = true
nimk-storage.workspace = true
serde.workspace = true
serde_json.workspace = true
```

`crates/nimk-hook/src/lib.rs`:

```rust
//! Hooks: observers of the engine's `Record` stream (docs/adr/0014).
//!
//! A hook never changes the buffer or the output, never sees the engine,
//! never spawns threads and never reads the wall clock. The engine's
//! `hookbus` owns the thread and calls hooks in registration order.

mod cx;

use std::io;

use nimk_types::Record;

pub use cx::{HookCx, HookMsg};

/// One observer of the record stream.
pub trait Hook: Send {
    /// Stable name; also the blob prefix `hooks/<name>/`.
    fn name(&self) -> &'static str;
    /// Called once before the first record, on the constructing thread.
    fn init(&mut self, _cx: &mut HookCx<'_>) {}
    /// Called for every record, in `seq` order.
    fn on_record(&mut self, rec: &Record, cx: &mut HookCx<'_>);
    /// Persists state (host cadence and shutdown).
    ///
    /// # Errors
    /// Storage failures; the host reports them and keeps running.
    fn flush(&mut self, _cx: &mut HookCx<'_>) -> io::Result<()> {
        Ok(())
    }
}
```

`crates/nimk-hook/src/cx.rs`:

```rust
//! What a hook may touch: its own blob prefix, read-only legacy keys, and
//! an outbox of summaries and diagnostics for the engine.

use std::io;

use nimk_storage::BlobStore;

/// Message from a hook to the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookMsg {
    /// Latest summary (JSON) of `hook`.
    Summary { hook: &'static str, json: String },
    /// Human-readable warning.
    Diagnostic(String),
}

/// Context handed to every hook call.
pub struct HookCx<'a> {
    name: &'static str,
    blobs: &'a dyn BlobStore,
    outbox: &'a mut Vec<HookMsg>,
}

impl<'a> HookCx<'a> {
    pub fn new(name: &'static str, blobs: &'a dyn BlobStore, outbox: &'a mut Vec<HookMsg>) -> Self {
        Self { name, blobs, outbox }
    }

    fn key(&self, key: &str) -> String {
        format!("hooks/{}/{key}", self.name)
    }

    /// Reads `hooks/<name>/<key>`.
    ///
    /// # Errors
    /// Storage failure.
    pub fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        self.blobs.get(&self.key(key))
    }

    /// Writes `hooks/<name>/<key>` atomically.
    ///
    /// # Errors
    /// Storage failure.
    pub fn put(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        self.blobs.put(&self.key(key), bytes)
    }

    /// Reads any key, for one-time migrations.
    ///
    /// # Errors
    /// Storage failure.
    pub fn get_legacy(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        self.blobs.get(key)
    }

    /// Publishes this hook's latest summary.
    pub fn summary(&mut self, json: String) {
        self.outbox.push(HookMsg::Summary { hook: self.name, json });
    }

    /// Reports a warning.
    pub fn diagnostic(&mut self, msg: String) {
        self.outbox.push(HookMsg::Diagnostic(format!("hook {}: {msg}", self.name)));
    }
}
```

`nimk-types`: move `StatCategory` (enum, `ALL`, `from_name`, `index`) into `crates/nimk-types/src/stat.rs`, add `#[derive(Serialize, Deserialize)] #[serde(rename_all = "snake_case")]`; in `nimk-engine/src/trainer/stats.rs` replace the definition with `pub use nimk_types::StatCategory;`. Add `utc_offset_min` (`#[serde(default)]`, doc: "Local time offset from UTC at start, minutes (host-supplied)") to `SessionStart` and the `ModelScore` variant (docs per field as in the spec). Fix every constructor of `SessionStart` (engine `with_models`: `utc_offset_min: session.utc_offset_min` — add `pub utc_offset_min: i32` to `SessionInfo`, and set it to `0` everywhere `SessionInfo` is built in tests/replay/synth; FFI fills it in Task 6). `nimk-replay` `feed`: `ModelScore` already falls into `_ => {}` — add an explicit arm comment `// Derived statistics; not replayed.`

- [ ] **Step 4: arch-lint**

In `arch-lint.toml`:
- header comment layer line: `ffi, replay, synth  ->  engine  ->  telex | policy | hook  ->  types` and `hook -> storage`.
- add scope:
  ```toml
  [[scopes]]
  name = "hook"
  paths = ["crates/nimk-hook/src/**"]
  ```
- add rule:
  ```toml
  [[restrict-use]]
  name = "leaf-hook"
  scope = "hook"
  deny = ["nimk_engine::**", "nimk_ffi::**", "nimk_policy::**", "nimk_telex::**", "nimk_synth::**", "nimk_replay::**",
          "std::fs::**", "std::net::**", "std::thread::**", "std::time::**", "std::env::**"]
  message = "Hooks observe Records; I/O only through the handles given (HookCx, EventSink); the host owns threads."
  ```
- append `"nimk_hook::**"` to the deny lists of `types-is-bottom`, `leaf-storage`, `leaf-telex`, `leaf-policy`, `ffi-thin`.
Update `docs/08-coding-standard.md` (layer diagram, scopes table, rules table).

- [ ] **Step 5: Verify**

Run: `cargo test -p nimk-hook -p nimk-types && cargo test -p nimk-replay --release --test pipeline_fixture && make check`
Expected: PASS; fixture unchanged.

Probe (do not commit): add `use std::thread;` to `crates/nimk-hook/src/lib.rs`, run `cargo arch-lint check` → expect a `leaf-hook` violation; revert.

- [ ] **Step 6: Commit**

```bash
git add -A Cargo.toml Cargo.lock crates arch-lint.toml docs/08-coding-standard.md
git commit -m "nimk-hook crate: Hook trait and HookCx; model_score event; arch-lint leaf-hook"
```

---

### Task 4: `hookbus` replaces `logger`; `event_log` hook

**Files:**
- Create: `crates/nimk-hook/src/event_log.rs` (hook wrapping an `EventSink`)
- Create: `crates/nimk-engine/src/hookbus.rs`; Delete: `crates/nimk-engine/src/logger.rs` (move `NullSink` into `hookbus.rs`)
- Modify: `crates/nimk-engine/src/lib.rs`, `engine.rs` (`logger: Logger` → `hooks: HookBus`), every `self.logger.log(` call site (rename to `self.hooks.log(`), `take_diagnostics`, `shutdown`, `flow.rs::drain_trainer` (also drain hook messages)
- Modify: `crates/nimk-engine/Cargo.toml` (+ `nimk-hook`), `arch-lint.toml` (`eng-logger` → `eng-hookbus`)
- Test: unit tests in `hookbus.rs`; `pipeline_fixture` unchanged

**Interfaces:**
- Consumes: `Hook`, `HookCx`, `HookMsg` (Task 3).
- Produces:
  ```rust
  // nimk_hook
  pub struct EventLogHook { sink: Box<dyn EventSink> }
  impl EventLogHook { pub fn new(sink: Box<dyn EventSink>) -> Self; }   // name "event_log"
  // nimk_engine::hookbus (crate-private)
  pub(crate) struct HookBus;
  impl HookBus {
      pub(crate) fn new(session: String, storage: Arc<dyn Storage>, hooks: Vec<Box<dyn Hook>>, background: bool) -> Self;
      pub(crate) fn log(&mut self, t: f64, event: Event);
      pub(crate) fn drain(&mut self) -> Vec<HookMsg>;          // summaries + diagnostics so far
      pub(crate) fn take_diagnostics(&mut self) -> Vec<String>;
      pub(crate) fn shutdown(self) -> io::Result<()>;          // flush all hooks, join thread
  }
  ```

- [ ] **Step 1: Failing tests in `hookbus.rs`**

```rust
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::sync::{Arc, Mutex};
    use nimk_hook::{Hook, HookCx};
    use nimk_storage::MemStorage;
    use nimk_types::{Event, Lang, Record, SwitchScope};
    use super::*;

    struct Count(Arc<Mutex<Vec<u64>>>);
    impl Hook for Count {
        fn name(&self) -> &'static str { "count" }
        fn on_record(&mut self, r: &Record, cx: &mut HookCx<'_>) {
            self.0.lock().unwrap().push(r.seq);
            cx.summary(format!("{}", r.seq));
        }
    }
    struct Boom;
    impl Hook for Boom {
        fn name(&self) -> &'static str { "boom" }
        fn on_record(&mut self, _r: &Record, _cx: &mut HookCx<'_>) { panic!("boom") }
    }
    fn ev() -> Event { Event::ManualSwitch { scope: SwitchScope::InWord, word_id: Some(1), to: Lang::Vi } }

    #[test]
    fn panicking_hook_is_disabled_others_continue() {
        for background in [false, true] {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let hooks: Vec<Box<dyn Hook>> = vec![Box::new(Boom), Box::new(Count(Arc::clone(&seen)))];
            let mut bus = HookBus::new("s".into(), Arc::new(MemStorage::new()), hooks, background);
            bus.log(1.0, ev());
            bus.log(2.0, ev());
            bus.shutdown_for_test();
            assert_eq!(*seen.lock().unwrap(), vec![0, 1]);
            let d = bus.take_diagnostics().join("\n");
            assert_eq!(d.matches("hook boom").count(), 1, "one diagnostic, then disabled: {d}");
        }
    }

    #[test]
    fn inline_and_background_deliver_same_summaries() {
        let summaries = |background: bool| {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let hooks: Vec<Box<dyn Hook>> = vec![Box::new(Count(Arc::clone(&seen)))];
            let mut bus = HookBus::new("s".into(), Arc::new(MemStorage::new()), hooks, background);
            bus.log(1.0, ev());
            bus.log(2.0, ev());
            bus.shutdown_for_test();
            bus.drain().into_iter().filter(|m| matches!(m, HookMsg::Summary { .. })).collect::<Vec<_>>()
        };
        assert_eq!(summaries(false), summaries(true));
        assert_eq!(summaries(false).len(), 2);
    }
}
```

`shutdown_for_test(&mut self)` flushes and joins the worker but keeps the bus to inspect diagnostics (`#[cfg(test)]` helper; implement `shutdown` on top of the same private `finish(&mut self)`). After `finish`, `drain()` must return every message the worker produced (join the thread, then collect what remains in the channel).

Move the existing `logger.rs` test `inline_and_background_write_same_bytes` into `crates/nimk-hook/src/event_log.rs` as a test of `EventLogHook` (driving `on_record` directly with records built like `Logger::log` did) and into `hookbus.rs` (full path, using a shared sink).

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p nimk-engine hookbus`
Expected: FAIL (module missing).

- [ ] **Step 3: Implement**

`crates/nimk-hook/src/event_log.rs`:

```rust
//! The event log as a hook: every record, serialized as one JSON line.

use std::io;

use nimk_storage::EventSink;
use nimk_types::Record;

use crate::{Hook, HookCx};

/// Appends every record to the session's event sink.
pub struct EventLogHook {
    sink: Box<dyn EventSink>,
}

impl EventLogHook {
    #[must_use]
    pub fn new(sink: Box<dyn EventSink>) -> Self {
        Self { sink }
    }
}

impl Hook for EventLogHook {
    fn name(&self) -> &'static str {
        "event_log"
    }

    fn on_record(&mut self, rec: &Record, cx: &mut HookCx<'_>) {
        let bytes = match serde_json::to_vec(rec) {
            Ok(b) => b,
            Err(e) => return cx.diagnostic(format!("log serialize failed: {e}")),
        };
        if let Err(e) = self.sink.append(&bytes) {
            cx.diagnostic(format!("log append failed (seq {}): {e}", rec.seq));
        }
    }

    fn flush(&mut self, _cx: &mut HookCx<'_>) -> io::Result<()> {
        self.sink.flush()
    }
}
```

Register in `lib.rs`: `mod event_log; pub use event_log::EventLogHook;`. Note: diagnostic text becomes `hook event_log: log append failed ...` — acceptable.

`crates/nimk-engine/src/hookbus.rs` — structure (mirrors `logger.rs`):

```rust
//! Hook host: wraps events into Records and runs every hook on them.
//!
//! `Inline` runs hooks on the caller (deterministic); `Background` hands
//! records to one worker thread. Each hook call is guarded by
//! `catch_unwind`: a panicking hook is disabled with one diagnostic. Hooks
//! flush after a `word_commit` once FLUSH_SECS of event time passed since
//! the last flush, and at shutdown.

const FLUSH_SECS: f64 = 30.0;

struct Slot { hook: Box<dyn Hook>, alive: bool }

/// Runs the hooks over one record; returns messages for the engine.
struct Runner { slots: Vec<Slot>, storage: Arc<dyn Storage>, last_flush_t: Option<f64>, outbox: Vec<HookMsg> }

impl Runner {
    fn init(&mut self) { /* for each slot: catch_unwind(|| hook.init(&mut HookCx::new(name, storage.blobs(), &mut outbox))) */ }
    fn record(&mut self, rec: &Record) {
        for slot in self.slots.iter_mut().filter(|s| s.alive) {
            let name = slot.hook.name();
            let mut cx = HookCx::new(name, self.storage.blobs(), &mut self.outbox);
            if catch_unwind(AssertUnwindSafe(|| slot.hook.on_record(rec, &mut cx))).is_err() {
                slot.alive = false;
                self.outbox.push(HookMsg::Diagnostic(format!("hook {name}: panicked; disabled")));
            }
        }
        let due = matches!(rec.event, Event::WordCommit { .. })
            && self.last_flush_t.is_none_or(|t0| rec.t - t0 >= FLUSH_SECS);
        if due { self.flush(rec.t); }
    }
    fn flush(&mut self, t: f64) { /* call flush on alive hooks under catch_unwind; Err -> Diagnostic("hook {name}: flush failed: {e}"); set last_flush_t = Some(t) */ }
}
```

`HookBus` holds `session`, `seq`, and a backend: `Inline(Runner)` or `Background { tx: Sender<Record>, msgs: Receiver<HookMsg>, handle: JoinHandle<io::Result<()>> }`. Worker loop: `runner.init()` happens **on the constructing thread before spawning** (so `init` summaries are available immediately — the model_stats summary must exist before the first key), then the runner moves into the thread; for each record `runner.record(&r)` and forward `runner.outbox.drain(..)` over the msgs channel; at channel close `runner.flush(last_t)` then forward. `log()` builds the `Record` exactly as `Logger::log` (`v: SCHEMA_VERSION`, `seq`, `t`, `session.clone()`). `drain()` returns inline outbox or `msgs.try_iter()`. `take_diagnostics()` = `drain()` filtered to `Diagnostic` strings plus internal ones — but summaries must not be lost: `drain()` returns everything and the engine routes them (see below); `take_diagnostics` only returns HookBus-internal diagnostics (spawn failures, "hook thread gone"). Spawn failure falls back to inline with a diagnostic, like `Logger`.

Engine wiring (`engine.rs::with_models`):

```rust
let hooks: Vec<Box<dyn Hook>> = vec![Box::new(EventLogHook::new(sink))];
let hooks_bus = HookBus::new(session.id.clone(), Arc::clone(&storage), hooks, background);
```

Add `summaries: HashMap<&'static str, String>` to `Engine`. In `drain_trainer` (rename the call sites' intent, keep the name) after draining the trainer:

```rust
for m in self.hooks.drain() {
    match m {
        HookMsg::Summary { hook, json } => { self.summaries.insert(hook, json); }
        HookMsg::Diagnostic(d) => self.diagnostics.push(d),
    }
}
```

Also call the same drain once at the end of `with_models` (picks up `init` summaries). Add `pub fn hook_summary(&self, name: &str) -> Option<&str>` to `Engine` (doc: latest summary JSON of hook `name`).

`take_diagnostics`: `d.extend(self.hooks.take_diagnostics())` instead of logger. `shutdown`: `let flushed = self.hooks.shutdown();`.

arch-lint: rename scope `eng-logger` → `eng-hookbus` with path `crates/nimk-engine/src/hookbus.rs`; `engine-no-threads.exclude` replace `logger.rs` by `hookbus.rs`; in the five `eng-*-independent` rules replace `crate::logger`/`super::logger` entries with `crate::hookbus`/`super::hookbus`, and rename rule `eng-logger-independent` → `eng-hookbus-independent`. Update `docs/08-coding-standard.md` scopes/rules rows and `docs/01-architecture.md` (threads table: "Hook thread" replaces "Log writer"; modules table: `hookbus`).

- [ ] **Step 4: Verify**

Run: `cargo test -p nimk-hook -p nimk-engine && cargo test -p nimk-replay --release --test pipeline_fixture && make check`
Expected: PASS; fixture bytes unchanged (proves `event_log` writes the same bytes).

- [ ] **Step 5: Commit**

```bash
git add -A crates arch-lint.toml docs
git commit -m "Engine: hookbus runs hooks; event log is the first hook"
```

---

### Task 5: `model_stats` hook

**Files:**
- Move: `crates/nimk-engine/src/trainer/stats.rs` → `crates/nimk-hook/src/model_stats/tracker.rs` (`StatsTracker`, `WordResult`, `WordAcc` stays in the trainer — it accumulates keystrokes there; `ModelStats` moves to `nimk-types/src/stat.rs` with `Serialize, Deserialize` so the summary JSON round-trips)
- Create: `crates/nimk-hook/src/model_stats.rs`
- Modify: `crates/nimk-engine/src/trainer.rs` (`record_word` emits `TrainerOut::Score(ScoreRow)` instead of mutating `StatsTracker`; remove stats load/save; `TrainerOut::Stats` removed), `engine.rs` (drop `load_stats`, `stats: [ModelStats; 2]`; `model_stats(kind)` reads summary), `flow.rs::drain_trainer` (log `Event::ModelScore`)
- Test: tracker unit tests (moved), `crates/nimk-hook/src/model_stats.rs` tests, engine test, fixture

**Interfaces:**
- Consumes: `Event::ModelScore`, `StatCategory`, `ModelStats` (nimk-types), `Hook`.
- Produces:
  ```rust
  pub struct ModelStatsHook;  impl ModelStatsHook { pub fn new() -> Self; }  // name "model_stats"
  // summary JSON: {"linear_action": ModelStats, "gru_action": ModelStats}  (serde of nimk_types::ModelStats)
  // blobs: hooks/model_stats/<kind>  (StatsTracker::to_bytes format, version 2)
  // trainer: TrainerOut::Score { kind: ModelKind, word_id: u64, category: StatCategory, correct: bool,
  //                              display_hits: u16, display_total: u16, flips: u16, amend: bool, served: bool }
  ```
  `served` (already on `Event::ModelScore` since Task 3) feeds `StatsTracker::served_words_total`.

`StatsTracker` input changes from `WordResult { acc: WordAcc, render, label, .. }` to precomputed counts. Define in the tracker:

```rust
pub struct Scored { pub word_id: u64, pub category: StatCategory, pub served: bool,
                    pub correct: bool, pub keys: u16, pub matched: u16, pub flips: u16 }
pub fn record(&mut self, w: Scored);
pub fn amend(&mut self, word_id: u64, correct: bool, matched: u16);
```

`amend` today recomputes `corrected` and `matched` from the stored `acc` and new label. The trainer still owns `acc` and the label, so it computes the new `correct`/`matched` and emits `ModelScore { amend: true, correct, display_hits: matched, .. }`; the tracker swaps old for new values (same arithmetic as today, using the stored `Recorded` entry's old `correct`/`matched`). The trainer keeps the last `RECORDED_KEPT` (8) `(word_id, WordAcc, render)` per core to compute amendments.

- [ ] **Step 1: Port tests first**

Move every test in `trainer/stats.rs` to `tracker.rs`, rewriting each `t.record(word(..))` to `t.record(Scored { .. })` with the counts the old helper derived (`keys = vi+en`, `matched = count of label`, `correct = render == label`). Keep assertions identical. Add to `model_stats.rs`:

```rust
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use nimk_storage::{MemStorage, Storage};
    use nimk_types::{Event, ModelStats, Record, StatCategory};
    use crate::{Hook, HookCx, HookMsg};
    use super::*;

    fn score(word_id: u64, correct: bool, amend: bool) -> Record {
        Record { v: 1, seq: word_id, t: 1.0, session: "s".into(), event: Event::ModelScore {
            kind: "gru_action".into(), word_id, category: StatCategory::Other, correct,
            display_hits: 2, display_total: 3, flips: 1, amend, served: true } }
    }

    fn summary(out: &[HookMsg]) -> serde_json::Value {
        match out.iter().rev().find(|m| matches!(m, HookMsg::Summary { .. })) {
            Some(HookMsg::Summary { json, .. }) => serde_json::from_str(json).unwrap(),
            _ => panic!("no summary"),
        }
    }

    #[test]
    fn counts_and_amends() {
        let s = MemStorage::new();
        let mut out = Vec::new();
        let mut h = ModelStatsHook::new();
        h.init(&mut HookCx::new("model_stats", s.blobs(), &mut out));
        for r in [score(0, true, false), score(1, false, false), score(1, true, true)] {
            h.on_record(&r, &mut HookCx::new("model_stats", s.blobs(), &mut out));
        }
        let gru: ModelStats = serde_json::from_value(summary(&out)["gru_action"].clone()).unwrap();
        assert_eq!((gru.words_total, gru.corrected_total), (2, 0));
        h.flush(&mut HookCx::new("model_stats", s.blobs(), &mut out)).unwrap();
        assert!(s.blobs().get("hooks/model_stats/gru_action").unwrap().is_some());
    }

    #[test]
    fn legacy_stats_blob_is_migrated_once() {
        let s = MemStorage::new();
        let mut legacy = StatsTracker::default();
        legacy.record(Scored { word_id: 0, category: StatCategory::Other, served: true, correct: false, keys: 3, matched: 1, flips: 0 });
        s.blobs().put("stats/gru_action", &legacy.to_bytes().unwrap()).unwrap();
        let mut out = Vec::new();
        let mut h = ModelStatsHook::new();
        h.init(&mut HookCx::new("model_stats", s.blobs(), &mut out));
        let gru: ModelStats = serde_json::from_value(summary(&out)["gru_action"].clone()).unwrap();
        assert_eq!((gru.words_total, gru.corrected_total), (1, 1));
        assert!(s.blobs().get("hooks/model_stats/gru_action").unwrap().is_some(), "written at init");
        // Second start reads the new key, not the legacy one.
        s.blobs().put("stats/gru_action", b"garbage").unwrap();
        let mut out2 = Vec::new();
        ModelStatsHook::new().init(&mut HookCx::new("model_stats", s.blobs(), &mut out2));
        assert!(!out2.iter().any(|m| matches!(m, HookMsg::Diagnostic(_))));
    }
}
```

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p nimk-hook model_stats`
Expected: FAIL.

- [ ] **Step 3: Implement**

`ModelStatsHook` holds `trackers: [StatsTracker; 2]` indexed by kind name (`"linear_action"` → 0, `"gru_action"` → 1; unknown kind → diagnostic, ignored). `init`: for each kind, `cx.get(kind)`; if `None`, `cx.get_legacy("stats/<kind>")` and, when present and parseable, adopt it and `cx.put(kind, bytes)`; unparseable → diagnostic `"<key> unusable (<e>); counting from zero"` (same text as today's `load_stats`). Emit the summary at the end of `init`. `on_record`: only `Event::ModelScore`; `amend` → `tracker.amend(word_id, correct, display_hits)`, else `tracker.record(Scored { keys: display_total, matched: display_hits, .. })`; emit the summary after each change. `flush`: `put` every dirty tracker, `mark_saved`. Summary JSON: `serde_json::json!({"linear_action": t0.summary(), "gru_action": t1.summary()}).to_string()`.

Trainer: in `record_word` compute what `StatsTracker::record` used to (`render`, `acc`) and push `TrainerOut::Score { .. }` with `correct = nimk_types::word_correct(render, label)`, `display_total = acc.keys()`, `display_hits = acc.matches(label)`, `flips = acc.flips`, `served = self.active`. For `info.amends`, look the word up in the core's last 8 scored words and emit `amend: true` with recomputed values; unknown word → nothing (as today). Remove `stats`, `with_stats`, `stats_summary`, stats save in snapshot, `stats_key`. Keep `WordAcc` in the trainer (make its accessor methods `pub(crate)`).

Engine: remove `stats`, `load_stats`, `TrainerOut::Stats` arm. `drain_trainer` maps `TrainerOut::Score` to `self.hooks.log(t, Event::ModelScore { kind: kind.name().into(), .. })`. Register `Box::new(ModelStatsHook::new())` after `EventLogHook`. `model_stats(kind)`: parse `self.summaries.get("model_stats")` → `serde_json::Value` → `[kind.name()]` → `ModelStats`; any failure → `ModelStats::default()`. Cache the parsed array when the summary is drained (parse once in the drain loop into `stats: [ModelStats; 2]`) so menu calls do not parse JSON. Shadow-only behaviour (`counts()`) is unchanged because the trainer still decides whether to emit.

Docs: `docs/05-storage-and-log.md` (event table row `model_score`, data dir tree `hooks/model_stats/<kind>`, legacy note), `docs/04-engine-ml.md` stats paragraph points to the hook.

- [ ] **Step 4: Bless only the added lines and verify**

The fixture's pinned types exclude `model_score`, and `stats` lines must be identical. Run: `cargo test -p nimk-hook -p nimk-engine && cargo test -p nimk-replay --release --test pipeline_fixture && make check`
Expected: PASS without blessing. If `stats` lines differ, the port is wrong — fix the code, never re-bless.

- [ ] **Step 5: Commit**

```bash
git add -A crates docs
git commit -m "model_stats hook: per-model statistics move out of the trainer"
```

---

### Task 6: `usage` hook, UTC offset, `nimk_hook_summary`

**Files:**
- Create: `crates/nimk-hook/src/usage.rs`
- Modify: `crates/nimk-engine/src/engine.rs` (register hook), `crates/nimk-ffi/src/handle.rs` (`utc_offset_min` from host clock), `crates/nimk-ffi/src/exports.rs` (`nimk_hook_summary`), `crates/nimk-ffi/src/abi.rs` (string buffer for the summary)
- Test: `usage.rs` unit tests; `crates/nimk-ffi` test for `nimk_hook_summary`

**Interfaces:**
- Produces: `UsageHook::new()` (name `"usage"`); blob `hooks/usage/<YYYY-MM-DD>` per spec; summary = today's blob JSON. FFI: `pub unsafe extern "C" fn nimk_hook_summary(core: *mut NimkCore, name: *const c_char) -> *const c_char` (null if none; string valid until the next call on `core`).

- [ ] **Step 1: Failing tests**

```rust
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use nimk_storage::{MemStorage, Storage};
    use nimk_types::{Boundary, Event, LabelSource, Lang, Record, Target};
    use crate::{Hook, HookCx};
    use super::*;

    fn rec(seq: u64, t: f64, event: Event) -> Record { Record { v: 1, seq, t, session: "s".into(), event } }
    fn start(wall_ms: u64, offset: i32) -> Event {
        Event::SessionStart { wall_unix_ms: wall_ms, app_version: "a".into(), config_json: "{}".into(),
                              policy_id: "p".into(), utc_offset_min: offset }
    }
    fn ctx(app: &str) -> Event { Event::Context { bundle_id: app.into(), field_id: 1, caret: None } }
    fn commit(id: u64, mode: Lang) -> Event {
        Event::WordCommit { word_id: id, raw: "x".into(), vi: "x".into(), en: "x".into(), mode, boundary: Boundary::Space }
    }

    fn feed(events: Vec<(f64, Event)>) -> MemStorage {
        let s = MemStorage::new();
        let mut out = Vec::new();
        let mut h = UsageHook::new();
        h.init(&mut HookCx::new("usage", s.blobs(), &mut out));
        for (i, (t, e)) in events.into_iter().enumerate() {
            h.on_record(&rec(i as u64, t, e), &mut HookCx::new("usage", s.blobs(), &mut out));
        }
        h.flush(&mut HookCx::new("usage", s.blobs(), &mut out)).unwrap();
        s
    }

    fn day(s: &MemStorage, d: &str) -> serde_json::Value {
        serde_json::from_slice(&s.blobs().get(&format!("hooks/usage/{d}")).unwrap().unwrap()).unwrap()
    }

    #[test]
    fn counts_words_per_app_and_corrections() {
        // 2026-11-09T10:00:00Z
        let s = feed(vec![
            (100.0, start(1_794_218_400_000, 0)),
            (100.0, ctx("com.apple.Terminal")),
            (101.0, commit(0, Lang::En)),
            (102.0, commit(1, Lang::Vi)),
            (103.0, Event::Label { target: Target::Word(1), label: Lang::En, weight: 1.0, source: LabelSource::Correction }),
            (104.0, Event::Label { target: Target::Word(0), label: Lang::En, weight: 0.3, source: LabelSource::Implicit }),
        ]);
        let d = day(&s, "2026-11-09");
        assert_eq!(d["apps"]["com.apple.Terminal"]["en"], 1);
        assert_eq!(d["apps"]["com.apple.Terminal"]["vi"], 1);
        assert_eq!(d["total"]["corrected"], 1);
    }

    #[test]
    fn day_bucket_uses_local_offset() {
        // 2026-11-09T23:30:00Z with +07:00 is 2026-11-10 local.
        let s = feed(vec![
            (0.0, start(1_794_267_000_000, 420)),
            (0.0, ctx("a")),
            (1.0, commit(0, Lang::Vi)),
        ]);
        assert_eq!(day(&s, "2026-11-10")["total"]["vi"], 1);
    }

    #[test]
    fn existing_day_is_extended_not_overwritten() {
        let s = feed(vec![(0.0, start(1_794_218_400_000, 0)), (0.0, ctx("a")), (1.0, commit(0, Lang::Vi))]);
        let mut out = Vec::new();
        let mut h = UsageHook::new();
        h.init(&mut HookCx::new("usage", s.blobs(), &mut out));
        for (i, e) in [start(1_794_218_500_000, 0), ctx("a"), commit(0, Lang::Vi)].into_iter().enumerate() {
            h.on_record(&rec(i as u64, 0.0, e), &mut HookCx::new("usage", s.blobs(), &mut out));
        }
        h.flush(&mut HookCx::new("usage", s.blobs(), &mut out)).unwrap();
        assert_eq!(day(&s, "2026-11-09")["total"]["vi"], 2);
    }
}
```

Compute the expected epoch values with `date -u -j -f "%Y-%m-%dT%H:%M:%S" 2026-11-09T10:00:00 +%s` and fix the constants if they are off; the assertions (dates) are what matters.

- [ ] **Step 2: Run to see failure**

Run: `cargo test -p nimk-hook usage`
Expected: FAIL.

- [ ] **Step 3: Implement**

```rust
//! Usage counters per local day: committed words by app and language, and
//! corrections. Counts only; no text is stored.

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Counts { pub vi: u64, pub en: u64, pub corrected: u64 }

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Day { pub version: u32, pub date: String,
                 pub apps: std::collections::BTreeMap<String, Counts>, pub total: Counts }

pub struct UsageHook {
    /// Session clock anchor: (t at session_start, wall ms, offset min).
    anchor: Option<(f64, u64, i32)>,
    app: String,
    /// Bundle id per committed word id, for attributing corrections (last 64).
    word_app: std::collections::VecDeque<(u64, String)>,
    day: Option<Day>,
    dirty: bool,
}
```

- `SessionStart`: set anchor `(rec.t, wall_unix_ms, utc_offset_min)`.
- `Context`: `app = bundle_id`.
- `WordCommit`: compute local date of `rec.t`; switch day if different (flush the old one first via `cx.put`; load the new one via `cx.get(date)` or default `Day { version: 1, date, .. }`); increment `apps[app]` and `total` by `mode`; remember `(word_id, app)`; `dirty = true`; emit summary (the day JSON).
- `Label { source: Correction, target: Word(id) }`: increment `corrected` for the remembered app of `id` (else the current app) and `total`.
- Without an anchor (no session_start), ignore commits (diagnostic once).
- Date from epoch ms (no `std::time`): civil-from-days algorithm (Howard Hinnant):

```rust
/// Local calendar date of `unix_ms + offset_min`.
fn local_date(unix_ms: i64, offset_min: i32) -> String {
    let days = (unix_ms / 1000 + i64::from(offset_min) * 60).div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}
```

Wall ms of a record: `wall + ((rec.t - t0) * 1000.0) as i64` (saturating; negative deltas clamp to 0).
- `flush`: `put(date, json)` if dirty.

Engine: register `Box::new(UsageHook::new())` after `ModelStatsHook`.

FFI `handle.rs`: `SessionInfo { utc_offset_min: local_utc_offset_min(), .. }` where

```rust
/// Local UTC offset in minutes from the C library (the FFI may read the clock).
fn local_utc_offset_min() -> i32 {
    // SAFETY: localtime_r writes into `tm`; time(null) has no preconditions.
    unsafe {
        let now = libc_time();
        let mut tm = std::mem::zeroed::<Tm>();
        if localtime_r(&now, &mut tm).is_null() { return 0; }
        (tm.tm_gmtoff / 60) as i32
    }
}
```

Declare the needed `extern "C" { fn time(t: *mut i64) -> i64; fn localtime_r(t: *const i64, tm: *mut Tm) -> *mut Tm; }` and a `#[repr(C)] struct Tm` matching macOS `struct tm` (`tm_sec..tm_isdst: i32` ×9, `tm_gmtoff: i64`, `tm_zone: *const c_char`). If the workspace already depends on `libc`, use it instead. Unsafe stays inside `nimk-ffi` (the only crate allowed).

`exports.rs`:

```rust
/// Latest summary JSON of hook `name` ("model_stats", "usage"), or null.
/// The string stays valid until the next call on `core`.
///
/// # Safety
/// `core` must be null or live from `nimk_new`; `name` null or a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nimk_hook_summary(core: *mut NimkCore, name: *const c_char) -> *const c_char {
    // SAFETY: null or live per contract, exclusive for this call.
    let Some(core) = (unsafe { core.as_mut() }) else { return std::ptr::null() };
    if name.is_null() { return std::ptr::null(); }
    // SAFETY: non-null C string per contract.
    let Ok(name) = (unsafe { CStr::from_ptr(name) }).to_str() else { return std::ptr::null() };
    let r = catch_unwind(AssertUnwindSafe(|| core.engine.hook_summary(name).map(str::to_owned)));
    match r {
        Ok(Some(s)) => core.out.set_summary(s),   // stores a CString in OutBuffers, returns its ptr
        _ => std::ptr::null(),
    }
}
```

Add `summary: Option<CString>` + `set_summary` to `OutBuffers` following how `nimk_policy_id` keeps its string. Add a test next to the existing FFI tests: build a core on a temp dir, feed a context + a word + space via `nimk_context`/`nimk_key`, then `nimk_hook_summary(core, c"usage")` is non-null and parses as JSON with `"total"`. Update `docs/01-architecture.md` C ABI table (20 functions) and regenerate the header with `make header`.

- [ ] **Step 4: Verify**

Run: `cargo test -p nimk-hook -p nimk-ffi -p nimk-engine && cargo test -p nimk-replay --release --test pipeline_fixture && make check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A crates docs
git commit -m "usage hook: per-day word and correction counts; nimk_hook_summary"
```

---

### Task 7: Config default and architecture docs

**Files:**
- Modify: `crates/nimk-engine/config/default.toml` (`ctrl_space = false`), `docs/11-config.md`, `docs/02-keyboard-io.md`, README "How to use" (Ctrl+Space is optional)
- Create: `docs/adr/0014-transform-hook-pipeline.md`
- Modify: `docs/01-architecture.md` (layer diagram with `hook`, crates table row `nimk-hook`, threads table), `docs/05-storage-and-log.md` (`utc_offset_min`, `hooks/` tree, `usage` blob), `docs/README.md` (ADR index)
- Test: any engine test asserting the Ctrl+Space default (`grep -rn ctrl_space crates/*/tests crates/*/src`)

- [ ] **Step 1: Change default, find dependent tests**

Run: `grep -rn "ctrl_space" crates --include=*.rs`
Tests that rely on Ctrl+Space being on by default must set `ctrl_space: true` explicitly in their config (do not weaken assertions).

- [ ] **Step 2: Write ADR 0014**

Sections in the house style of `docs/adr/0013-*.md` (Status, Context, Decision, Consequences): Transforms = pure stages that change buffer/output (Telex, Model); Hooks = observers of `Record`s on the hook thread; why `Record` is the single data model (replayable hooks); isolation (`catch_unwind`, disable); flush cadence; `model_score` additive event; arch-lint rules `leaf-hook`, `engine-uses-transforms`.

- [ ] **Step 3: Verify**

Run: `make check && cargo test -p nimk-replay --release --test pipeline_fixture`
Expected: PASS (fixture uses `hotkeys: false`, so the default change does not affect it).

- [ ] **Step 4: Commit**

```bash
git add -A crates docs README.md
git commit -m "Ctrl+Space off by default; ADR 0014 Transform/Hook pipeline; docs"
```

---

### Task 8: Swift pure logic for the app (tested with swiftc)

**Files:**
- Create: `macos/Shared/ConfigKeys.swift`, `macos/Shared/HotkeyPreset.swift`, `macos/Shared/VersionPolicy.swift`, `macos/Shared/UsageData.swift`, `macos/Shared/CorrectionHint.swift`
- Create: `macos/tests/app/main.swift`
- Modify: `Makefile` (`swift-test` target)

**Interfaces:**
- Produces:
  ```swift
  enum ConfigKeys { static func set(_ text: String, values: [(String, Bool)]) -> String }
  enum HotkeyPreset: String, CaseIterable { case doubleShift, ctrlSpace, both, off
      var values: [(String, Bool)]        // [("double_shift", _), ("ctrl_space", _)]
      static func from(doubleShift: Bool, ctrlSpace: Bool) -> HotkeyPreset }
  enum InstallAction: Equatable { case install, update, none }
  enum VersionPolicy { static func action(bundled: Int?, installed: Int?) -> InstallAction }
  struct UsageCounts: Decodable, Equatable { let vi: Int; let en: Int; let corrected: Int }
  struct UsageDay: Decodable, Equatable { let date: String; let apps: [String: UsageCounts]; let total: UsageCounts }
  enum UsageData { static func parse(_ data: Data) -> UsageDay?
                   static func topApps(_ days: [UsageDay], limit: Int) -> [(app: String, words: Int)] }
  struct CorrectionHint { static let maxShown = 3
      static func shouldShow(count: Int) -> Bool; static func next(count: Int) -> Int }
  ```

- [ ] **Step 1: Write the test runner**

`macos/tests/app/main.swift`:

```swift
import Foundation

func check<T: Equatable>(_ got: T, _ want: T, _ line: Int = #line) {
    if got != want { print("FAIL line \(line): got \(got), want \(want)"); exit(1) }
}

// ConfigKeys: insert at top before tables, replace in place, preserve the rest.
let user = "# mine\nswitch_margin = 0.05\n\n[[app_category]]\nname = \"terminal\"\nctrl_space = true\n"
check(ConfigKeys.set(user, values: [("double_shift", true), ("ctrl_space", false)]),
      "double_shift = true\nctrl_space = false\n# mine\nswitch_margin = 0.05\n\n[[app_category]]\nname = \"terminal\"\nctrl_space = true\n")
check(ConfigKeys.set("ctrl_space = true # old\nx = 1\n", values: [("ctrl_space", false)]),
      "ctrl_space = false\nx = 1\n")
check(ConfigKeys.set("", values: [("double_shift", false)]), "double_shift = false\n")
check(ConfigKeys.set("  double_shift=true\n[t]\n", values: [("double_shift", false)]), "double_shift = false\n[t]\n")

// Presets
check(HotkeyPreset.doubleShift.values.map { "\($0.0)=\($0.1)" }, ["double_shift=true", "ctrl_space=false"])
check(HotkeyPreset.from(doubleShift: true, ctrlSpace: true), .both)
check(HotkeyPreset.from(doubleShift: false, ctrlSpace: false), .off)

// Versions
check(VersionPolicy.action(bundled: 5, installed: nil), .install)
check(VersionPolicy.action(bundled: 5, installed: 4), .update)
check(VersionPolicy.action(bundled: 5, installed: 5), .none)
check(VersionPolicy.action(bundled: 5, installed: 6), .none)
check(VersionPolicy.action(bundled: nil, installed: 3), .none)

// Usage
let json = #"{"version":1,"date":"2026-11-09","apps":{"a":{"vi":3,"en":1,"corrected":0},"b":{"vi":0,"en":9,"corrected":2}},"total":{"vi":3,"en":10,"corrected":2}}"#
let day = UsageData.parse(Data(json.utf8))
check(day?.total, UsageCounts(vi: 3, en: 10, corrected: 2))
check(UsageData.topApps([day!], limit: 1).map { $0.app }, ["b"])
check(UsageData.parse(Data("nope".utf8)), nil)

// Correction hint
check(CorrectionHint.shouldShow(count: 0), true)
check(CorrectionHint.shouldShow(count: 3), false)
check(CorrectionHint.next(count: 2), 3)

print("app ok")
```

Makefile `swift-test` add:

```make
	swiftc macos/Shared/*.swift macos/tests/app/main.swift -o target/swift-test/app
	target/swift-test/app
```

- [ ] **Step 2: Run to see failure**

Run: `make swift-test`
Expected: FAIL to compile (types missing).

- [ ] **Step 3: Implement**

`ConfigKeys.swift`:

```swift
import Foundation

/// Rewrites top-level boolean keys of a TOML file, preserving everything
/// else byte for byte. Only lines before the first table header count as
/// top level; missing keys are inserted at the very top.
enum ConfigKeys {
    static func set(_ text: String, values: [(String, Bool)]) -> String {
        var lines = text.isEmpty ? [] : text.components(separatedBy: "\n")
        if lines.last == "" { lines.removeLast() }
        let firstTable = lines.firstIndex { $0.trimmingCharacters(in: .whitespaces).hasPrefix("[") } ?? lines.count
        var missing: [String] = []
        for (key, value) in values {
            let line = "\(key) = \(value)"
            let idx = lines[..<firstTable].firstIndex { raw in
                let t = raw.trimmingCharacters(in: .whitespaces)
                guard t.hasPrefix(key) else { return false }
                return t.dropFirst(key.count).trimmingCharacters(in: .whitespaces).hasPrefix("=")
            }
            if let idx { lines[idx] = line } else { missing.append(line) }
        }
        return (missing + lines).map { $0 + "\n" }.joined()
    }
}
```

`HotkeyPreset.swift`:

```swift
/// The four hotkey choices; each maps to the engine's two config bools.
enum HotkeyPreset: String, CaseIterable {
    case doubleShift, ctrlSpace, both, off

    var values: [(String, Bool)] {
        switch self {
        case .doubleShift: return [("double_shift", true), ("ctrl_space", false)]
        case .ctrlSpace: return [("double_shift", false), ("ctrl_space", true)]
        case .both: return [("double_shift", true), ("ctrl_space", true)]
        case .off: return [("double_shift", false), ("ctrl_space", false)]
        }
    }

    static func from(doubleShift: Bool, ctrlSpace: Bool) -> HotkeyPreset {
        switch (doubleShift, ctrlSpace) {
        case (true, false): return .doubleShift
        case (false, true): return .ctrlSpace
        case (true, true): return .both
        case (false, false): return .off
        }
    }
}
```

`VersionPolicy.swift`:

```swift
/// What the installer does given the bundled and installed CFBundleVersion.
enum InstallAction: Equatable { case install, update, none }

enum VersionPolicy {
    static func action(bundled: Int?, installed: Int?) -> InstallAction {
        guard let bundled else { return .none }
        guard let installed else { return .install }
        return bundled > installed ? .update : .none
    }
}
```

`UsageData.swift`:

```swift
import Foundation

struct UsageCounts: Decodable, Equatable { let vi: Int; let en: Int; let corrected: Int }
struct UsageDay: Decodable, Equatable { let date: String; let apps: [String: UsageCounts]; let total: UsageCounts }

/// Parsing and aggregation of the IME's `hooks/usage/<date>` blobs.
enum UsageData {
    static func parse(_ data: Data) -> UsageDay? { try? JSONDecoder().decode(UsageDay.self, from: data) }

    static func topApps(_ days: [UsageDay], limit: Int) -> [(app: String, words: Int)] {
        var words: [String: Int] = [:]
        for day in days { for (app, c) in day.apps { words[app, default: 0] += c.vi + c.en } }
        return words.sorted { $0.value != $1.value ? $0.value > $1.value : $0.key < $1.key }
            .prefix(limit).map { (app: $0.key, words: $0.value) }
    }
}
```

`CorrectionHint.swift`:

```swift
/// The "double-Shift again to undo" hint shows for the first corrections only.
struct CorrectionHint {
    static let maxShown = 3
    static func shouldShow(count: Int) -> Bool { count < maxShown }
    static func next(count: Int) -> Int { count + 1 }
}
```

- [ ] **Step 4: Verify**

Run: `make swift-test`
Expected: prints `app ok` (and the existing `debouncer`, `idlecommit`, `statstext` outputs).

- [ ] **Step 5: Commit**

```bash
git add macos/Shared macos/tests/app Makefile
git commit -m "App: tested pure logic (config keys, presets, versions, usage, hint)"
```

---

### Task 9: IME-side Swift changes

**Files:**
- Modify: `macos/NeuralIMK/main.swift` (observe `…reloadConfig`), `macos/NeuralIMK/InputController.swift` (post `…corrected` on manual overlay; correction overlay text; menu item "Mở NeuralIMK…"), `macos/NeuralIMK/Overlay.swift` (wider text variant if needed — `showText` already exists), `scripts/build-app.sh` (compile `macos/Shared/CorrectionHint.swift` into the IME)
- Modify: `docs/06-gui.md` (menu item, notifications, overlay text)

**Interfaces:**
- Consumes: `CorrectionHint` (Task 8), `Core.reloadConfig()`.
- Produces: distributed notifications `com.giangndm.inputmethod.NeuralIMK.reloadConfig` (observed) and `com.giangndm.inputmethod.NeuralIMK.corrected` (posted); `UserDefaults` key `correctionHintCount` (IME domain).

- [ ] **Step 1: Reload observer (`main.swift`, before `app.run()`)**

```swift
// The companion app rewrote config.toml (docs/06-gui.md).
let reloadObserver = DistributedNotificationCenter.default().addObserver(
    forName: Notification.Name("com.giangndm.inputmethod.NeuralIMK.reloadConfig"),
    object: nil, queue: .main) { _ in
    _ = Core.shared.reloadConfig()
}
```

Add `reloadObserver` to the `withExtendedLifetime` tuple.

- [ ] **Step 2: Correction notification and overlay (`InputController.swift`, where `out.overlay` is applied, ~line 294)**

```swift
if out.overlay == UInt8(NIMK_OVERLAY_MANUAL) {
    DistributedNotificationCenter.default().postNotificationName(
        Notification.Name("com.giangndm.inputmethod.NeuralIMK.corrected"),
        object: nil, userInfo: nil, deliverImmediately: true)
    let defaults = UserDefaults.standard
    let count = defaults.integer(forKey: "correctionHintCount")
    if CorrectionHint.shouldShow(count: count), let word = (out.marked ?? out.commit), !word.isEmpty {
        defaults.set(CorrectionHint.next(count: count), forKey: "correctionHintCount")
        Overlay.shared.showText("→ \(word.trimmingCharacters(in: .whitespaces)) · Double-Shift lần nữa để hoàn tác",
                                caret: caret)
        return  // replaces the VI/EN badge for this event
    }
}
```

Place it so the existing `showOverlay(...)` call is skipped only when the hint was shown. Check that `Overlay.showText` sizes the panel to the text; if it uses the fixed 34×22 size, make `showText` compute width from `label.intrinsicContentSize` (+16 px padding).

- [ ] **Step 3: Menu item (`menu()`, before "Open data folder")**

```swift
let openApp = NSMenuItem(title: "Mở NeuralIMK…", action: #selector(openCompanion(_:)), keyEquivalent: "")
openApp.target = self
menu.addItem(openApp)
```

```swift
@objc func openCompanion(_ sender: Any?) {
    log("menu: open companion")
    if let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.giangndm.NeuralIMK") {
        NSWorkspace.shared.openApplication(at: url, configuration: NSWorkspace.OpenConfiguration())
    }
}
```

Use whatever logging helper the file already uses for menu actions (`menu: ...` lines).

- [ ] **Step 4: Build**

In `scripts/build-app.sh`, the IME `swiftc` line compiles `"$SRC"/*.swift` — add `macos/Shared/CorrectionHint.swift`.
Run: `./scripts/build-app.sh && make swift-test`
Expected: builds `build/NeuralIMK.app` without warnings-as-errors failures; swift tests pass.

- [ ] **Step 5: Commit**

```bash
git add macos/NeuralIMK scripts/build-app.sh docs/06-gui.md
git commit -m "IME: reload on notification, correction hint overlay, open companion"
```

---

### Task 10: Companion app shell, installer, Overview; nested build

**Files:**
- Create: `macos/App/Info.plist`, `macos/App/main.swift` (SwiftUI `App`), `macos/App/AppModel.swift` (paths, refresh, watcher), `macos/App/Installer.swift`, `macos/App/InputSource.swift` (TIS wrappers), `macos/App/Strings.swift`, `macos/App/OverviewView.swift`, `macos/App/RootView.swift`
- Modify: `scripts/build-app.sh` (build IME into `build/ime/NeuralIMK.app`, then the app into `build/NeuralIMK.app` with the IME nested; `CFBundleVersion` from `git rev-list --count HEAD` in both plists), `scripts/install.sh` (dev path: copy nested IME as today and also the app to `/Applications` only if `INSTALL_APP=1`)
- Modify: `docs/07-build-and-install.md`

**Interfaces:**
- Consumes: `VersionPolicy`, `InstallAction` (Task 8).
- Produces:
  ```swift
  final class AppModel: ObservableObject {
      let imeBundleId: String            // from nested Info.plist
      let dataDir: URL; let logURL: URL; let configURL: URL; let nestedIME: URL; let installedIME: URL
      @Published var installState: InstallState   // .notInstalled / .installed(version) / .failed(String)
      @Published var imeSelected: Bool
      @Published var developerMode: Bool          // UserDefaults "developerMode"
      @Published var showPerAppUsage: Bool        // UserDefaults "showPerAppUsage", default true
      func refresh()
  }
  enum Installer { static func installIfNeeded(_ m: AppModel) throws -> InstallAction; static func uninstall(_ m: AppModel, deleteData: Bool) throws }
  enum InputSource { static func register(_ url: URL); static func enable(bundleId: String) -> Bool; static func isSelected(bundleId: String) -> Bool }
  enum Section: Int, CaseIterable { case overview = 1, statistics, settings, log }
  ```

- [ ] **Step 1: `Info.plist`**

Keys: `CFBundleIdentifier` `com.giangndm.NeuralIMK`, `CFBundleName`/`CFBundleDisplayName` `NeuralIMK`, `CFBundleExecutable` `NeuralIMK`, `CFBundlePackageType` `APPL`, `CFBundleShortVersionString` `0.1.0`, `CFBundleVersion` `1` (rewritten by build), `LSMinimumSystemVersion` `13.0`, `NSPrincipalClass` `NSApplication`, `CFBundleIconFile` `AppIcon`, `LSApplicationCategoryType` `public.app-category.utilities`.

- [ ] **Step 2: `InputSource.swift`**

```swift
import Carbon
import Foundation

/// Text Input Sources wrappers for the nested IME.
enum InputSource {
    static func register(_ url: URL) { TISRegisterInputSource(url as CFURL) }

    private static func sources(bundleId: String) -> [TISInputSource] {
        let filter = [kTISPropertyBundleID as String: bundleId] as CFDictionary
        guard let list = TISCreateInputSourceList(filter, true)?.takeRetainedValue() as? [TISInputSource] else { return [] }
        return list
    }

    /// Enables every input source of the IME; true if at least one is enabled.
    static func enable(bundleId: String) -> Bool {
        sources(bundleId: bundleId).reduce(false) { ok, src in TISEnableInputSource(src) == noErr || ok }
    }

    static func isSelected(bundleId: String) -> Bool {
        guard let cur = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue(),
              let ptr = TISGetInputSourceProperty(cur, kTISPropertyBundleID) else { return false }
        return (Unmanaged<CFString>.fromOpaque(ptr).takeUnretainedValue() as String) == bundleId
    }

    static func select(bundleId: String) {
        if let src = sources(bundleId: bundleId).first { TISSelectInputSource(src) }
    }
}
```

- [ ] **Step 3: `Installer.swift`**

```swift
import AppKit

enum InstallError: LocalizedError {
    case copyFailed(String)
    var errorDescription: String? { if case .copyFailed(let m) = self { return m }; return nil }
}

/// Copies the nested IME into ~/Library/Input Methods and registers it.
enum Installer {
    static func bundleVersion(_ url: URL) -> Int? {
        guard let info = NSDictionary(contentsOf: url.appendingPathComponent("Contents/Info.plist")),
              let v = info["CFBundleVersion"] as? String else { return nil }
        return Int(v)
    }

    static func installIfNeeded(_ m: AppModel) throws -> InstallAction {
        let action = VersionPolicy.action(bundled: bundleVersion(m.nestedIME), installed: bundleVersion(m.installedIME))
        guard action != .none else { return .none }
        let fm = FileManager.default
        let dir = m.installedIME.deletingLastPathComponent()
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        let tmp = dir.appendingPathComponent(".NeuralIMK.installing.app")
        try? fm.removeItem(at: tmp)
        do { try fm.copyItem(at: m.nestedIME, to: tmp) } catch { throw InstallError.copyFailed(error.localizedDescription) }
        terminateIME(bundleId: m.imeBundleId)
        if fm.fileExists(atPath: m.installedIME.path) {
            _ = try fm.replaceItemAt(m.installedIME, withItemAt: tmp)
        } else {
            try fm.moveItem(at: tmp, to: m.installedIME)
        }
        InputSource.register(m.installedIME)
        _ = InputSource.enable(bundleId: m.imeBundleId)
        return action
    }

    /// SIGTERM (the IME flushes its engine), wait up to 2 s.
    static func terminateIME(bundleId: String) {
        let running = NSRunningApplication.runningApplications(withBundleIdentifier: bundleId)
        running.forEach { $0.terminate() }
        let deadline = Date().addingTimeInterval(2)
        while running.contains(where: { !$0.isTerminated }) && Date() < deadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        }
    }

    static func uninstall(_ m: AppModel, deleteData: Bool) throws {
        terminateIME(bundleId: m.imeBundleId)
        try? FileManager.default.removeItem(at: m.installedIME)
        if deleteData { try? FileManager.default.removeItem(at: m.dataDir) }
    }
}
```

`NSRunningApplication.terminate()` sends a quit Apple event; the IME is `LSBackgroundOnly` and may ignore it — if `isTerminated` stays false after the deadline, send `kill(pid, SIGTERM)` for each remaining `processIdentifier` (the IME already handles SIGTERM in `main.swift`).

- [ ] **Step 4: `AppModel.swift`, `RootView.swift`, `OverviewView.swift`, `Strings.swift`, `main.swift`**

- `AppModel.init`: `nestedIME = Bundle.main.bundleURL.appendingPathComponent("Contents/Library/InputMethods/NeuralIMK.app")`; `imeBundleId` from its plist (fallback `com.giangndm.inputmethod.NeuralIMK`); `installedIME = <Library>/Input Methods/NeuralIMK.app` via `FileManager.url(for: .libraryDirectory, ...)`; `dataDir = <Application Support>/<imeBundleId>`; `logURL = <Library>/Logs/<imeBundleId>.log`; `configURL = dataDir/config.toml`. `refresh()` recomputes `installState` and `imeSelected`. A `Timer` every 1 s updates `imeSelected` while onboarding or Overview is visible. A `DispatchSource.makeFileSystemObjectSource` on `dataDir/hooks` (open with `O_EVTONLY`) triggers `objectWillChange` for statistics.
- `main.swift`:
  ```swift
  import SwiftUI

  @main
  struct NeuralIMKApp: App {
      @StateObject private var model = AppModel()
      var body: some Scene {
          WindowGroup("NeuralIMK") { RootView().environmentObject(model).frame(minWidth: 760, minHeight: 520) }
              .commands {
                  CommandGroup(replacing: .appSettings) {
                      Button(Strings.settings) { model.section = .settings }.keyboardShortcut(",", modifiers: .command)
                  }
                  CommandMenu(Strings.view) {
                      ForEach(Section.allCases.filter { $0 != .log || model.developerMode }, id: \.self) { s in
                          Button(s.title) { model.section = s }.keyboardShortcut(KeyEquivalent(Character("\(s.rawValue)")), modifiers: .command)
                      }
                      Button(Strings.refresh) { model.refresh() }.keyboardShortcut("r", modifiers: .command)
                  }
              }
      }
  }
  ```
  `@Published var section: Section = .overview` lives in `AppModel`. On first appear: `try Installer.installIfNeeded(model)` (errors → `installState = .failed(msg)`), and show onboarding when `UserDefaults` `onboardingDone` is false (Task 12 provides `OnboardingView`; until then a placeholder `Text`).
- `RootView`: `NavigationSplitView` with a `List(selection:)` over visible sections (Log only in developer mode) and a detail switch; Statistics/Settings/Log are placeholders replaced in Tasks 11–12.
- `OverviewView`: install state row (version, "Thử lại" button on failure), the warning `Strings.typingInABC` when installed && !imeSelected with a "Chọn NeuralIMK" button (`InputSource.select`), active model (read `dataDir/blobs/settings/model` JSON `{"model": ...}`, default `gru_action`) and its recent accuracy from `blobs/hooks/model_stats/<kind>`, then a hotkey cheat sheet (double-Shift = sửa từ, double-Shift lần nữa = hoàn tác, `/ @ !` đầu từ = gõ lệnh), and a button "Xem lại hướng dẫn" that reopens onboarding.
- `Strings.swift`: every Vietnamese string used by the app as `static let`.

Note: blob keys map to files under `dataDir/blobs/<key>` (see `docs/05-storage-and-log.md` tree), so `hooks/model_stats/gru_action` is `dataDir/blobs/hooks/model_stats/gru_action`.

- [ ] **Step 5: Build script**

Restructure `scripts/build-app.sh`:
1. IME build unchanged but into `IME_APP="build/ime/NeuralIMK.app"`.
2. Version: `BUILD_NUMBER="$(git rev-list --count HEAD 2>/dev/null || echo 1)"`; `plutil -replace CFBundleVersion -string "$BUILD_NUMBER"` on both plists after copying.
3. App: `swiftc -O -swift-version 5 -parse-as-library -target "$arch-apple-macos$MIN_MACOS" -module-name NeuralIMKApp macos/App/*.swift macos/Shared/*.swift -framework SwiftUI -framework Charts -framework Carbon -framework AppKit -o build/obj/NeuralIMKApp-$arch`; `lipo` for multiple archs like the IME.
4. Assemble `build/NeuralIMK.app/Contents/{MacOS/NeuralIMK, Info.plist, Resources/AppIcon.icns, Library/InputMethods/NeuralIMK.app}`. App icon: extend `scripts/make-icon.swift` usage or render a 1024 px PNG and convert with `iconutil` (same visual as the menu icon).
5. Sign inside-out ad hoc: `codesign -s - --force "$APP/Contents/Library/InputMethods/NeuralIMK.app"` then `codesign -s - --force "$APP"`. (Task 13 swaps in a real identity.)
`scripts/install.sh` copies `build/NeuralIMK.app/Contents/Library/InputMethods/NeuralIMK.app` (dev flow unchanged for `make install`).

- [ ] **Step 6: Verify**

Run: `make app && make swift-test && open build/NeuralIMK.app`
Expected: window opens; Overview shows installed version; `ls ~/Library/Input\ Methods` shows `NeuralIMK.app` with the new `CFBundleVersion`; Cmd+1..3 switch sections; Cmd+, opens Settings section.
Run `make check` (Rust untouched but must stay green).

- [ ] **Step 7: Commit**

```bash
git add macos/App scripts docs/07-build-and-install.md
git commit -m "Companion app: installer, input source checks, Overview; nested build"
```

---

### Task 11: Statistics and Log screens

**Files:**
- Create: `macos/App/StatisticsView.swift`, `macos/App/StatsStore.swift`, `macos/App/LogView.swift`
- Modify: `macos/App/RootView.swift`

**Interfaces:**
- Consumes: `UsageData`, `UsageDay` (Task 8), `AppModel.dataDir/logURL/showPerAppUsage/developerMode` (Task 10).
- Produces: `final class StatsStore: ObservableObject { @Published var days: [UsageDay]; @Published var models: [String: ModelStatsJSON]; func load(dataDir: URL) }` where `struct ModelStatsJSON: Decodable { let words_recent: Int; let corrected_recent: Int; let words_total: Int; let corrected_total: Int; let keys_recent: Int; let keys_match_recent: Int }` read from `blobs/hooks/model_stats/<kind>` — note this blob is the `StatsTracker` format (not `ModelStats`); read its `totals` object (check `to_bytes` in `crates/nimk-hook/src/model_stats/tracker.rs` for the exact shape and decode accordingly).

- [ ] **Step 1: `StatsStore.load`**: list `blobs/hooks/usage/*` (file names are dates), keep the last 30 by name, parse with `UsageData.parse`, sort by date; read both model_stats blobs. Missing files → empty arrays (empty state view: `Strings.noDataYet`).

- [ ] **Step 2: `StatisticsView`** (Swift Charts):
  - "Từ mỗi ngày": stacked `BarMark` per day, series VI/EN.
  - "Sửa mỗi ngày": `LineMark` of `total.corrected`.
  - "Độ chính xác model": per model, recent accuracy `1 - corrected_recent/words_recent` and lifetime, as grouped `BarMark`; skip models with 0 words.
  - "Theo ứng dụng" (only if `showPerAppUsage`): horizontal `BarMark` of `UsageData.topApps(days, limit: 8)`.
  - Each chart: `.accessibilityLabel(summary)` (e.g. "30 ngày, trung bình 840 từ mỗi ngày, 62% tiếng Việt") and a `Toggle(Strings.showAsTable)` that swaps the chart for a `Table`/`Grid` of the same numbers.
  - Refresh on `AppModel` watcher and Cmd+R.

- [ ] **Step 3: `LogView`** (developer mode only): read the last 500 lines of `logURL` (read the file tail: seek to max(0, size - 256 KB), split lines, keep last 500); `TextField` filter bound to `@State var filter`, focused by Cmd+F (`.keyboardShortcut("f")` on a hidden button setting `@FocusState`), `.onExitCommand { filter = "" }` for Esc; monospaced `List` of matching lines (plain `localizedCaseInsensitiveContains`); "Mở thư mục dữ liệu" button (`NSWorkspace.shared.open(dataDir)`).

- [ ] **Step 4: Verify**

Run: `make app && open build/NeuralIMK.app`, type a few words with NeuralIMK in any app, wait 30 s, type one more word (forces a usage flush), press Cmd+R in the app.
Expected: Statistics shows today's bars; toggling "Hiện dạng bảng" shows numbers; enabling Developer mode (Task 12; until then set `defaults write com.giangndm.NeuralIMK developerMode -bool true`) shows Log with Cmd+4 and filtering works.

- [ ] **Step 5: Commit**

```bash
git add macos/App
git commit -m "Companion app: statistics charts and developer log"
```

---

### Task 12: Settings and Onboarding

**Files:**
- Create: `macos/App/SettingsView.swift`, `macos/App/OnboardingView.swift`, `macos/App/SystemShortcuts.swift`
- Modify: `macos/App/RootView.swift`, `macos/App/main.swift` (present onboarding sheet)

**Interfaces:**
- Consumes: `ConfigKeys`, `HotkeyPreset`, `CorrectionHint` (Task 8), `Installer`, `InputSource`, `AppModel` (Task 10); notification names from Task 9.
- Produces: `enum SystemShortcuts { static func inputSourceShortcutEnabled() -> Bool }`; `UserDefaults` keys `onboardingDone`, `developerMode`, `showPerAppUsage`.

- [ ] **Step 1: `SystemShortcuts`**

```swift
import Foundation

/// Reads whether macOS's own "Select previous/next input source" shortcut
/// (symbolic hotkeys 60 and 61) is enabled; macOS then swallows Ctrl+Space.
enum SystemShortcuts {
    static func inputSourceShortcutEnabled() -> Bool {
        guard let dict = UserDefaults(suiteName: "com.apple.symbolichotkeys")?
                .dictionary(forKey: "AppleSymbolicHotKeys") else { return false }
        return ["60", "61"].contains { id in
            ((dict[id] as? [String: Any])?["enabled"] as? Bool) == true
        }
    }
}
```

(If `UserDefaults(suiteName:)` cannot read another domain, read `~/Library/Preferences/com.apple.symbolichotkeys.plist` with `NSDictionary(contentsOf:)`.)

- [ ] **Step 2: `SettingsView`**

- "Phím tắt sửa từ": `Picker` (segmented) over `HotkeyPreset.allCases` with Vietnamese labels (Double Shift / Ctrl+Space / Cả hai / Tắt). Initial value: parse current config by reading `config.toml` lines for `double_shift`/`ctrl_space` (missing → defaults true/false). On change: `let text = (try? String(contentsOf: configURL)) ?? ""`; `let new = ConfigKeys.set(text, values: preset.values)`; write atomically (`Data(new.utf8).write(to: configURL, options: .atomic)`, create `dataDir` if missing); post `DistributedNotificationCenter.default().postNotificationName(.init("com.giangndm.inputmethod.NeuralIMK.reloadConfig"), object: nil, userInfo: nil, deliverImmediately: true)`.
- `.off` selected → caption `Strings.noHotkeyUseMenu`.
- Preset includes Ctrl+Space and `SystemShortcuts.inputSourceShortcutEnabled()` → warning with button opening `x-apple.systempreferences:com.apple.preference.keyboard?Shortcuts`.
- "Thử ngay" `TextField` next to the picker (plain field; typing in it goes through the IME).
- Toggles: "Hiện thống kê theo ứng dụng" (`showPerAppUsage`), "Chế độ nhà phát triển" (`developerMode`).
- "Xoá dữ liệu thống kê": confirmation dialog; deletes `dataDir/blobs/hooks/usage`.
- "Gỡ cài đặt NeuralIMK…": confirmation sheet with a "Xoá cả dữ liệu học" toggle → `Installer.uninstall`; afterwards show `Strings.removeFromInputSources` and a button opening `x-apple.systempreferences:com.apple.Keyboard-Settings.extension`.
- All controls reachable by Tab (native controls only).

- [ ] **Step 3: `OnboardingView`** (sheet, 4 steps, "Tiếp"/"Quay lại" buttons with Return/Esc shortcuts, step indicator):
  1. Cài đặt: runs `Installer.installIfNeeded` (or shows it done); line `Strings.statsStayLocal`.
  2. Chọn NeuralIMK: live check (`imeSelected` from the 1 s timer) with a green checkmark; buttons "Bật NeuralIMK" (`InputSource.enable` + `select`) and "Mở cài đặt Bàn phím"; tips about the Globe/Fn key and "Use Caps Lock to switch to and from ABC" (`Strings.capsLockTip`).
  3. Luyện tập: prompt picked at random from 3 sentences in `Strings.practicePrompts` (e.g. "chao ban, hom nay toi hoc code"); `TextField`; instruction to make one word wrong and double-Shift it; a checkmark when the `…corrected` distributed notification arrives (observer added on appear, removed on disappear).
  4. Xong: cheat sheet (same view as Overview) and "Bắt đầu dùng" → `onboardingDone = true`.

- [ ] **Step 4: Verify**

Run: `defaults delete com.giangndm.NeuralIMK onboardingDone; make app && open build/NeuralIMK.app`
Expected: onboarding appears; step 2 turns green after selecting NeuralIMK in the input menu; step 3 ticks after a double-Shift correction in the practice field; changing the preset rewrites `config.toml` (inspect with `cat`) and the IME shows "Đã nạp config"; Cmd+, selects Settings.

- [ ] **Step 5: Commit**

```bash
git add macos/App
git commit -m "Companion app: settings (hotkey presets, uninstall) and onboarding"
```

---

### Task 13: DMG packaging, signing, notarization, docs

**Files:**
- Create: `scripts/package-dmg.sh`, `scripts/make-dmg-background.swift`
- Modify: `scripts/build-app.sh` (signing identity from `NIMK_SIGN_IDENTITY`), `Makefile` (`dmg` target), `docs/07-build-and-install.md` (Release section + manual checklist), `README.md` (end-user install path)

**Interfaces:**
- Consumes: `build/NeuralIMK.app` from Task 10.
- Produces: `build/NeuralIMK-<CFBundleShortVersionString>.dmg`; env vars `NIMK_SIGN_IDENTITY`, `NIMK_NOTARY_PROFILE`.

- [ ] **Step 1: Signing in `build-app.sh`**

```bash
SIGN_ID="${NIMK_SIGN_IDENTITY:--}"
sign() {
  if [[ "$SIGN_ID" == "-" ]]; then
    codesign -s - --force "$1"
  else
    codesign --force --options runtime --timestamp -s "$SIGN_ID" "$1"
  fi
}
sign "$APP/Contents/Library/InputMethods/NeuralIMK.app"
sign "$APP"
codesign --verify --strict --verbose=2 "$APP"
```

- [ ] **Step 2: Background image script**

`scripts/make-dmg-background.swift <out.png>`: draws a 640×400 PNG (light background, a large arrow from left (x≈160) to right (x≈480) at y≈200, caption "Kéo NeuralIMK vào Applications" at the bottom) with AppKit `NSImage` + `NSBitmapImageRep`, like `make-icon.swift`.

- [ ] **Step 3: `package-dmg.sh`**

```bash
#!/usr/bin/env bash
# Builds a universal app and packs it into a drag-to-Applications DMG.
# Env: NIMK_SIGN_IDENTITY (Developer ID Application; ad hoc if unset),
#      NIMK_NOTARY_PROFILE (notarytool keychain profile; skip notarization if unset).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ARCHS="arm64 x86_64" ./scripts/build-app.sh
APP="build/NeuralIMK.app"
VERSION="$(plutil -extract CFBundleShortVersionString raw "$APP/Contents/Info.plist")"
DMG="build/NeuralIMK-$VERSION.dmg"
STAGE="build/dmg-stage"
rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
swift scripts/make-dmg-background.swift build/dmg-background.png

if command -v create-dmg >/dev/null; then
  create-dmg --volname "NeuralIMK" --background build/dmg-background.png \
    --window-size 640 400 --icon-size 112 --icon "NeuralIMK.app" 160 190 \
    --app-drop-link 480 190 --hide-extension "NeuralIMK.app" "$DMG" "$STAGE"
else
  ln -s /Applications "$STAGE/Applications"
  hdiutil create -volname "NeuralIMK" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
  echo "note: create-dmg not installed; DMG has no background layout (brew install create-dmg)"
fi

if [[ -n "${NIMK_SIGN_IDENTITY:-}" ]]; then
  codesign --force --timestamp -s "$NIMK_SIGN_IDENTITY" "$DMG"
fi
if [[ -n "${NIMK_NOTARY_PROFILE:-}" ]]; then
  xcrun notarytool submit "$DMG" --keychain-profile "$NIMK_NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
elif [[ -z "${NIMK_SIGN_IDENTITY:-}" ]]; then
  echo "warning: ad hoc signed, not notarized: other Macs need System Settings > Privacy & Security > Open Anyway"
fi
echo "built $DMG"
```

`create-dmg` exits 2 when it only failed to set a custom icon; treat 0 and 2 as success (`|| [[ $? -eq 2 ]]`).

Makefile: add `dmg` to `.PHONY` and

```make
dmg:
	./scripts/package-dmg.sh
```

- [ ] **Step 4: Docs**

`docs/07-build-and-install.md`: Makefile table row `dmg`; "Release" section (requirements: Apple Developer Program, Developer ID Application cert, `xcrun notarytool store-credentials <profile>`; commands; what ad hoc means for users) and a "Manual release checklist" (fresh user account: open DMG, drag, open app, onboarding steps 1–4, type in TextEdit, re-drag a newer build and open once → updated, uninstall; Ctrl+Space warning appears only when the macOS shortcut is on). Update the build pipeline diagram (app wraps IME) and the install table (`make install` = dev; DMG = users). README quick start: add "For users: download the DMG, drag NeuralIMK to Applications, open it and follow the steps." Mention that Ctrl+Space is off by default.

- [ ] **Step 5: Verify**

Run: `make dmg && hdiutil attach build/NeuralIMK-0.1.0.dmg -nobrowse -mountpoint build/mnt && ls build/mnt && codesign --verify --deep --strict build/mnt/NeuralIMK.app; hdiutil detach build/mnt`
Expected: listing shows `NeuralIMK.app` and `Applications`; codesign verify succeeds; `lipo -archs build/NeuralIMK.app/Contents/MacOS/NeuralIMK` prints `x86_64 arm64` (requires `rustup target add x86_64-apple-darwin`; if missing, the script fails with the rustup hint — install the target, do not drop the arch).
Run: `make check && make swift-test`.

- [ ] **Step 6: Commit**

```bash
git add scripts Makefile docs README.md
git commit -m "Release: universal build, DMG packaging, optional signing and notarization"
```
