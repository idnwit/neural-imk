# Release app, Transform/Hook pipeline: design

> **Status:** approved (product owner) · **Date:** 2026-11-09 · **Branch:** `release-app`

## Goal

Ordinary Mac users install NeuralIMK from a DMG by dragging one app to Applications, open it once, and are typing within a minute. The same app shows what the IME is doing (statistics, logs). Internally, the engine is modelled as a data pipeline: **Transforms** change the buffer (the model lives there), **Hooks** only observe data. New features (usage statistics now, spell check later) are added as hooks or transforms without touching the core loop.

Success criteria:

1. A fresh macOS user installs from the DMG, completes onboarding and types Vietnamese with no Terminal command.
2. Re-dragging a newer DMG build and opening the app once updates the IME.
3. After the refactor, every existing test and the golden tests pass unchanged, and replay of a fixed synthetic log gives byte-identical decisions.
4. `make check` passes, including the updated `arch-lint.toml`.

## Non-goals (MVP cuts)

- Model controls in the GUI (switch model, online learning, restore). They stay in the IME menu; the GUI only displays model state.
- Sparkle or any auto-update; free-form shortcut recorder; per-app hotkeys; regex log search; VoiceOver audio graphs.
- Spell check (a later hook or transform).
- A dynamic chain of transforms: there are exactly two, with concrete types.

## Concepts

| Concept | May change the buffer or the output | Runs | Examples |
| --- | --- | --- | --- |
| Transform | Yes | Main thread, on the key path; pure function of its input | Telex, model VI/EN decision |
| Hook | No: receives copies of `Record`s | Hook thread (background) or the caller (`Threading::Inline`) | Event log, model statistics, usage statistics |

The runtime pipeline carries only data. State (the 4-state machine, `shown`, `locked`, the composer) stays in the engine and is passed to transforms as input.

## Part 1: Engine

### 1.1 Transforms

`nimk-types` gains:

```rust
pub trait Transform {
    type Input;
    type Output;
    fn apply(&self, input: &Self::Input) -> Self::Output;
}
```

| Transform | Crate | Input | Output | Replaces |
| --- | --- | --- | --- | --- |
| `TelexTransform` | `nimk-telex` | `str` (raw buffer) | `BufferState { vi: String, valid_vi: bool, viable_vi: bool }` | `Engine::refresh_telex` body |
| `ModelTransform` | `nimk-policy` | `ModelInput { obs: Observation, base: Lang, locked: bool, margin: f32, structural: Option<EditAction> }` | `ModelOutput { action: Action, p_vi: Option<f32> }` | Model branch of `model_step`: `score`, `decide`, hysteresis, lock, commit-step rule |

- `ModelTransform` holds an `Arc<dyn ImeModel>`; the engine rebuilds it when it swaps the model snapshot.
- The teacher-forced branch (`Engine::teach`) stays in the engine: it bypasses the model by definition.
- Behaviour must be bit-identical. Before refactoring, a test records the decision stream (`chosen`, `edit`, `p_vi`) of a fixed synthetic session per model kind as a fixture; the same test must pass after.

### 1.2 Hooks: crate `nimk-hook`

New library crate on the same layer as `telex` and `policy`. Depends on `nimk-types`, `nimk-storage`, `serde`, `serde_json`.

```rust
pub trait Hook: Send {
    /// Stable name; also the blob prefix `hooks/<name>/`.
    fn name(&self) -> &'static str;
    /// Called once before the first record, on the constructing thread.
    fn init(&mut self, cx: &mut HookCx<'_>);
    fn on_record(&mut self, rec: &Record, cx: &mut HookCx<'_>);
    /// Persist state. Called by the host on its flush cadence and at shutdown.
    fn flush(&mut self, cx: &mut HookCx<'_>) -> io::Result<()>;
}

pub struct HookCx<'a> { /* blobs, name, outbox */ }
impl HookCx<'_> {
    pub fn get(&self, key: &str) -> io::Result<Option<Vec<u8>>>;   // key under hooks/<name>/
    pub fn put(&self, key: &str, bytes: &[u8]) -> io::Result<()>;  // key under hooks/<name>/
    pub fn get_legacy(&self, key: &str) -> io::Result<Option<Vec<u8>>>; // any key, read-only (migration)
    pub fn summary(&mut self, json: String);                       // latest summary of this hook
    pub fn diagnostic(&mut self, msg: String);
}
```

Rules:

- A hook never sees the engine, never spawns threads, never reads the wall clock or env. Wall time is derived from data: `session_start.wall_unix_ms + (t - t_session_start) * 1000`.
- Local dates use `session_start.utc_offset_min` (new field, `#[serde(default)]` = 0; filled by `nimk-ffi` from the host clock).

`nimk-engine/src/hookbus.rs` replaces `logger.rs`:

- Owns the `Record` envelope (`v`, `seq`, `t`, `session`), exactly as `Logger` does today.
- `Background`: one thread runs all hooks in registration order; records arrive over an mpsc channel. `Inline`: hooks run on the caller.
- Every hook call is wrapped in `catch_unwind`; a panicking hook is disabled and reported in diagnostics. The key path is never affected.
- Flush cadence (event time, deterministic): after a `word_commit` when at least 30 s passed since the last flush, and at shutdown. The `event_log` hook additionally appends every record immediately (sink buffering unchanged).
- Summaries and diagnostics come back over a channel; the engine drains them at word boundaries (same place it drains trainer outputs) and caches the latest summary per hook.
- `Engine::hook_summary(name) -> Option<&str>`; FFI `nimk_hook_summary(core, name) -> *const c_char` (valid until the next call, like `NimkOut` strings).

### 1.3 Built-in hooks

| Hook | Consumes | Writes | Summary |
| --- | --- | --- | --- |
| `event_log` | every record | `events/*.jsonl` through the session `EventSink` (format unchanged; same bytes as today's logger) | none |
| `model_stats` | `model_score` | `hooks/model_stats/<kind>` (JSON, `StatsTracker` format v2) | `{"<kind>": ModelStats, ...}` |
| `usage` | `session_start`, `context`, `word_commit`, `label` | `hooks/usage/<YYYY-MM-DD>` (local date) | today's totals |

`model_stats`:

- `StatsTracker` (rolling window of 500, lifetime totals, `amend`) moves from `nimk-engine/src/trainer/stats.rs` to `nimk-hook`. `StatCategory` moves to `nimk-types` (it appears in an event).
- The trainer keeps computing per-word results (it alone sees shadow decisions) and emits `TrainerOut::Score`; the engine logs it as the new event below.
- Migration: on `init`, if `hooks/model_stats/<kind>` is absent and the legacy `stats/<kind>` exists, it is read once and written to the new key. The legacy blob is left in place.
- `Engine::model_stats(kind)` and `nimk_model_stats` keep their signatures and read the cached `model_stats` summary.

New event (additive; `SCHEMA_VERSION` stays 1, `nimk-replay` ignores it):

```json
{"type":"model_score","kind":"gru_action","word_id":7,"category":"terminal","correct":true,
 "display_hits":3,"display_total":4,"flips":1,"amend":false}
```

`amend: true` re-scores a word already counted (late correction), mirroring `StatsTracker::amend`.

`usage` blob per local day:

```json
{"version":1,"date":"2026-11-09",
 "apps":{"com.apple.Terminal":{"vi":120,"en":310,"corrected":4}},
 "total":{"vi":900,"en":1200,"corrected":21}}
```

- `vi`/`en` count committed words by `word_commit.mode`, attributed to the bundle id of the latest `context`.
- `corrected` counts `label` records with `source = correction`.
- Counts only; no text is stored.

### 1.4 Config default

`ctrl_space = false` in `crates/nimk-engine/config/default.toml` (it collides with the macOS input-source shortcut and IDE autocomplete). Docs 11 and 02 updated.

### 1.5 arch-lint

| Change | Detail |
| --- | --- |
| Scope `hook` | `crates/nimk-hook/src/**` |
| Rule `leaf-hook` | scope `hook`; deny `nimk_engine::**`, `nimk_ffi::**`, `nimk_policy::**`, `nimk_telex::**`, `nimk_synth::**`, `nimk_replay::**`, `std::fs::**`, `std::net::**`, `std::thread::**`, `std::time::**`, `std::env::**`. Message: hooks observe Records; I/O only through the handles given; the host owns threads |
| Rule `engine-uses-transforms` | scope `engine`; deny `nimk_telex::transform`, `nimk_policy::decide`. The engine reaches Telex and the model only through `TelexTransform` / `ModelTransform` |
| Deny `nimk_hook::**` | add to `types-is-bottom`, `leaf-storage`, `leaf-telex`, `leaf-policy`, `ffi-thin` |
| Engine module scopes | `eng-logger` becomes `eng-hookbus` (`hookbus.rs`); `engine-no-threads` excludes `hookbus.rs` instead of `logger.rs`; the five `eng-*-independent` deny lists swap `logger` for `hookbus` |
| Header comment | layer diagram includes `hook` |

Docs: `docs/01-architecture.md` (layers, threads, crates, C ABI count), `docs/05-storage-and-log.md` (new event, `hooks/` keys, `utc_offset_min`), `docs/08-coding-standard.md` (scopes, rules), new `docs/adr/0014-transform-hook-pipeline.md`.

## Part 2: Companion app

### 2.1 Bundle

```text
/Applications/NeuralIMK.app                      com.giangndm.NeuralIMK, SwiftUI, macOS 13+
  Contents/MacOS/NeuralIMK                       GUI (regular app, Dock icon while open)
  Contents/Library/InputMethods/NeuralIMK.app    the IME bundle, unchanged bundle id
```

- Sources in `macos/App/*.swift`; pure logic in files with no AppKit/SwiftUI imports so `make swift-test` can test them (existing pattern in `macos/tests/`).
- `scripts/build-app.sh` builds the IME as today into `build/NeuralIMK Input/NeuralIMK.app` (path internal to the build), then the GUI, then nests the IME and signs both.
- The GUI resolves the IME data dir as `<Application Support>/com.giangndm.inputmethod.NeuralIMK` and the log as `<Library>/Logs/com.giangndm.inputmethod.NeuralIMK.log` via `FileManager`, with the IME bundle id read from the nested bundle's `Info.plist`.

### 2.2 Install, update, uninstall (`Installer.swift`)

On every launch:

1. Read `CFBundleVersion` of the nested IME and of `~/Library/Input Methods/NeuralIMK.app` (if any).
2. If missing or older: copy the nested bundle to a temp dir inside `~/Library/Input Methods`, send SIGTERM to running `NeuralIMK` IME processes (matched by bundle id via `NSRunningApplication`), wait up to 2 s, replace by rename, then `TISRegisterInputSource` and `TISEnableInputSource` for the IME's input source id.
3. Errors are shown in Overview with a "Try again" button and logged to the GUI's own log.

Uninstall (Settings, confirmation sheet): terminate the IME, remove the bundle, optionally delete the data dir; then tell the user to remove it from Input Sources if TIS still lists it.

Version policy: `CFBundleVersion` is a monotonically increasing integer set by `build-app.sh` from `git rev-list --count HEAD`; `CFBundleShortVersionString` stays semantic.

### 2.3 Screens

| Section | Key | Content |
| --- | --- | --- |
| Overview | Cmd+1 | Install state and version; active model and its recent accuracy (`settings/model`, `hooks/model_stats`); hotkey cheat sheet; the warning "NeuralIMK is enabled but you are typing in ABC" when `TISCopyCurrentKeyboardInputSource` is not NeuralIMK |
| Statistics | Cmd+2 | Swift Charts: words per day by VI/EN (last 30 days), corrections per day, model accuracy (recent/lifetime per model), per-app words (toggle "Show per-app usage", default on). Each chart has a "Show as table" toggle and an accessibility label summarising it |
| Settings | Cmd+3 or Cmd+, | Hotkey preset; Developer mode; "Delete usage data"; Uninstall |
| Log | Cmd+4 | Only with Developer mode: tail of the diagnostics log (last 500 lines) with a plain-text filter (Cmd+F, Esc clears) |

Cmd+R refreshes. Data also refreshes automatically via a `DispatchSource` watcher on `hooks/`. All controls are reachable with Tab; Reduce Motion is respected.

UI strings are Vietnamese (the app's users), kept in one `Strings.swift`.

### 2.4 Keyboard design (agreed with the UI/UX review)

1. Double-Shift = "fix this word"; a second double-Shift undoes it. Esc is never taken from the app (it is a commit key).
2. Hotkey presets map exactly to existing config: Double Shift (`double_shift=true, ctrl_space=false`, default), Ctrl+Space, Both, Off. "Off" shows "No hotkey; use the input menu". A "Try it" text field sits next to the picker.
3. Ctrl+Space conflict warning only when `com.apple.symbolichotkeys` (ids 60, 61) shows the input-source shortcut enabled; the copy never states the macOS default. Button opens Keyboard Shortcuts settings.
4. Onboarding (first launch, re-openable from Overview), 4 steps:
   1. Install and enable, with the line "Usage statistics stay on this Mac."
   2. Select NeuralIMK: live green check polling TIS once per second; tips on the Globe/Fn key and on "Use Caps Lock to switch to and from ABC".
   3. Practice: type a prompt (randomly chosen from a small list), deliberately get one word wrong, double-Shift to fix it; a tick appears when the IME posts the distributed notification `com.giangndm.inputmethod.NeuralIMK.corrected` (posted on every hotkey flip; see 2.5). Practice corrections train the model normally.
   4. Done, with the cheat sheet.
5. Correction overlay (IME, `Overlay::Manual` from a hotkey flip): shows the word's new rendering (e.g. `→ việt`, taken from the `NimkOut` text) plus "Double-Shift lần nữa để hoàn tác" for the first 5 corrections or until 3 corrections were made, whichever comes first (counter in the IME's `UserDefaults`). After that the plain VI/EN overlay as today. No sounds.

### 2.5 Config writes and reload

- The GUI writes only the top-level keys `double_shift` and `ctrl_space` of the IME's `config.toml`: existing lines for those keys are replaced in place; missing keys are inserted at the top of the file, before any table header. Other content is preserved byte for byte. Write is atomic (temp + rename).
- Then it posts the distributed notification `com.giangndm.inputmethod.NeuralIMK.reloadConfig`. The IME observes it and calls the existing `Core.reloadConfig()` on the main thread.
- The IME posts `com.giangndm.inputmethod.NeuralIMK.corrected` (no payload) whenever a hotkey flip is applied (`overlay = MANUAL`). Hook flushes are too slow for live UI feedback.
- The IME menu gains "Mở NeuralIMK…", which opens `com.giangndm.NeuralIMK` via `NSWorkspace`.

## Part 3: Packaging

`scripts/package-dmg.sh` (Makefile target `dmg`):

1. `ARCHS="arm64 x86_64" ./scripts/build-app.sh` (universal).
2. Signing:
   - If `NIMK_SIGN_IDENTITY` is set: sign the nested IME, then the outer app, each with `codesign --force --options runtime --timestamp -s "$NIMK_SIGN_IDENTITY"` (inside-out, no `--deep`).
   - Otherwise ad hoc (`-s -`), and the script prints that the DMG will need "Open Anyway" on other Macs.
3. DMG: stage `NeuralIMK.app`, a symlink `Applications -> /Applications`, and a background image with an arrow (generated by a Swift script, like the icon). If `create-dmg` is installed it lays out icons and background; otherwise plain `hdiutil create -format UDZO` with the same staged folder (no layout). Output `build/NeuralIMK-<short version>.dmg`.
4. If `NIMK_NOTARY_PROFILE` is set (a `notarytool store-credentials` profile): sign the DMG, `xcrun notarytool submit --wait`, then `xcrun stapler staple` the DMG.

`docs/07-build-and-install.md` gains a "Release" section; README quick start gains the end-user path ("download DMG, drag to Applications, open").

## Part 4: Testing

| Area | Test |
| --- | --- |
| Transform refactor | Decision-stream fixture test recorded before refactor (per model kind); all existing tests and golden tests unchanged |
| `event_log` | Same records written byte-identical to the old logger for a scripted session (compare against a fixture captured before the change) |
| `model_stats` | Ported `StatsTracker` unit tests; engine-level test that `model_stats(kind)` equals the pre-refactor numbers for a scripted session; legacy migration test |
| `usage` | Unit tests: day bucketing with `utc_offset_min`, per-app attribution, corrections counted only from `source = correction`, persistence round trip |
| `hookbus` | Panicking hook is disabled and others keep running; Inline determinism; flush cadence; summaries drained |
| arch-lint | `make arch` passes; a deliberate violation probe (local, not committed) confirms `leaf-hook` and `engine-uses-transforms` fire |
| Swift pure logic | `make swift-test` additions: version compare, config key rewrite (insert/replace/preserve), hotkey preset mapping, usage JSON parsing for charts, tip counter |
| Manual (checklist in docs 07) | Fresh user account: DMG install, onboarding, typing, update by re-drag, uninstall; Ctrl+Space warning on/off |

## Decisions log

| Decision | Reason |
| --- | --- |
| Hooks consume `Record`s (one data model) | Hooks can be rerun over old logs and tested through replay; no second event model |
| Hooks run in the IME, GUI only reads | No IPC; data exists even when the GUI is closed |
| Model controls stay in the IME menu | Avoids a command channel GUI to IME in the MVP; the only command is reload-config |
| Presets instead of a shortcut recorder | Maps to existing config bools; no unusable bindings |
| Ctrl+Space default off | Swallowed by macOS when the input-source shortcut is on; conflicts with IDE autocomplete |
| Per-app usage on by default, toggleable, deletable | Local only; the event log already stores bundle ids |
