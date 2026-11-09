//! Behaviour-cloning pre-training of the neural action model through an
//! inline engine, plus held-out evaluation (`pretrain`).

use std::io;
use std::sync::Arc;

use nimk_engine::{Engine, EngineConfig, EngineError, SessionInfo, Threading};
use nimk_policy::{ModelKind, build_action};
use nimk_storage::{BlobStore, EventSink, EventSource, MemStorage, Storage};
use nimk_types::{Action, EditAction, Event, Lang, Record, Target};

use crate::eval_report::{Decided, EvalReport};
use crate::log::{APP_VERSION, synthetic_context};
use crate::{Script, ScriptWord};

/// Engine session id used while pre-training (its events are discarded).
const PRETRAIN_SESSION: &str = "nimk-synth-pretrain";
/// Engine session id of the evaluation engine.
const EVAL_SESSION: &str = "nimk-synth-eval";

/// Pre-training knobs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PretrainOptions {
    /// Passes over the training scripts. 1 is enough for `LinearAction`;
    /// the GRU benefits from 2-3.
    pub epochs: usize,
    /// Fraction of scripts (the last ones) held out for evaluation, in
    /// `[0, 1)`. With 2 or more scripts and a positive fraction at least one
    /// script is held out and at least one trains.
    pub eval_frac: f64,
    /// Teacher-forced examples per learner update
    /// (`EngineConfig::teach_batch`).
    pub batch: usize,
}

impl Default for PretrainOptions {
    fn default() -> Self {
        Self {
            epochs: 2,
            eval_frac: 0.1,
            batch: 128,
        }
    }
}

/// Outcome of a pre-training run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PretrainReport {
    /// Passes made.
    pub epochs: usize,
    /// Scripts trained on per epoch.
    pub train_scripts: u64,
    /// Teacher-forced keys over all epochs.
    pub train_keys: u64,
    /// Evaluation of the saved model on the held-out scripts.
    pub eval: EvalReport,
}

/// Storage view that keeps the model snapshot (blobs) but discards events,
/// so pre-training never mixes synthetic records into a real event log.
struct ModelOnly(Arc<dyn Storage>);

struct DiscardSink;

impl EventSink for DiscardSink {
    fn append(&mut self, _record: &[u8]) -> io::Result<()> {
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Storage for ModelOnly {
    fn event_sink(&self, _session: &str) -> io::Result<Box<dyn EventSink>> {
        Ok(Box::new(DiscardSink))
    }

    fn event_source(&self) -> io::Result<Box<dyn EventSource + '_>> {
        self.0.event_source()
    }

    fn blobs(&self) -> &dyn BlobStore {
        self.0.blobs()
    }
}

/// Snapshot blob key the engine uses for `kind`.
fn snapshot_key(kind: ModelKind) -> String {
    format!("model/{}/latest", kind.name())
}

fn invalid_data(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn engine_err(e: EngineError) -> io::Error {
    match e {
        EngineError::Storage(io) => io,
    }
}

fn session(id: &str) -> SessionInfo {
    SessionInfo {
        id: id.to_owned(),
        wall_unix_ms: 0,
        app_version: APP_VERSION.to_owned(),
        utc_offset_min: 0,
    }
}

/// Number of scripts held out of `n` for `eval_frac`.
fn eval_count(n: usize, eval_frac: f64) -> usize {
    if n < 2 || eval_frac.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return 0;
    }
    let k = (n as f64 * eval_frac.min(1.0)).round() as usize;
    k.clamp(1, n - 1)
}

/// The scripts [`pretrain`] holds out for evaluation (the last ones, per
/// `opts.eval_frac`).
#[must_use]
pub fn held_out(scripts: &[Script], eval_frac: f64) -> &[Script] {
    &scripts[scripts.len() - eval_count(scripts.len(), eval_frac)..]
}

/// Behaviour-clones `scripts` into a fresh `cfg.policy` model, saves it as
/// the engine snapshot (`model/<kind>/latest`) in `storage`, and evaluates
/// the saved model on held-out scripts.
///
/// Training: the first `n - k` scripts (`k` from [`PretrainOptions::eval_frac`])
/// run through one inline engine for `opts.epochs` passes, each key via
/// `Engine::teach` with the script's target action and both loss weights
/// 1.0; every script starts with a focus change to a new synthetic field.
/// `cfg` is used with `hotkeys = false`, `threading = Inline` and periodic
/// snapshots off (one snapshot is saved at the end). Events are discarded.
///
/// Evaluation: a fresh engine loads the saved snapshot and the last `k`
/// scripts are fed through `Engine::on_key`, so the model decides; no labels
/// are injected, so nothing is learned. Each decision is compared with the
/// target (edit over all keys; chosen rendering at `Commit` keys, also
/// restricted to ambiguous words) and the engine's committed text with the
/// expected text.
///
/// # Errors
/// `InvalidData` if the engine reports diagnostics (dropped labels, bad
/// snapshot) or the evaluation log does not line up with the keys; any I/O
/// error from saving or loading the snapshot.
pub fn pretrain(
    scripts: &[Script],
    cfg: EngineConfig,
    opts: PretrainOptions,
    storage: Arc<dyn Storage>,
) -> io::Result<PretrainReport> {
    let cfg = pretrain_config(cfg, opts);
    let k = eval_count(scripts.len(), opts.eval_frac);
    let (train, held_out) = scripts.split_at(scripts.len() - k);

    let learner = build_action(cfg.policy, cfg.seed);
    let model_only: Arc<dyn Storage> = Arc::new(ModelOnly(Arc::clone(&storage)));
    let mut engine =
        Engine::with_model(cfg.clone(), model_only, session(PRETRAIN_SESSION), learner);
    let mut train_keys = 0u64;
    let mut field_id = 0u64;
    for _ in 0..opts.epochs {
        for script in train {
            train_keys += teach_script(&mut engine, field_id, script)?;
            field_id += 1;
        }
    }
    engine.shutdown().map_err(engine_err)?;

    let eval = evaluate(held_out, &cfg, &storage)?;
    Ok(PretrainReport {
        epochs: opts.epochs,
        train_scripts: train.len() as u64,
        train_keys,
        eval,
    })
}

/// Teacher-forces one script; returns the number of keys.
fn teach_script(engine: &mut Engine, field_id: u64, script: &Script) -> io::Result<u64> {
    if script.keys.len() != script.targets.len() {
        return Err(invalid_data(format!(
            "{}: {} keys but {} targets",
            script.session,
            script.keys.len(),
            script.targets.len()
        )));
    }
    let t0 = script.keys.first().map_or(0.0, |k| k.timestamp);
    engine.on_context(&synthetic_context(&script.bundle_id, field_id, t0));
    for (key, target) in script.keys.iter().zip(&script.targets) {
        engine.teach(key, *target, 1.0, 1.0);
    }
    check_diagnostics(engine, &script.session)?;
    Ok(script.keys.len() as u64)
}

pub(crate) fn check_diagnostics(engine: &mut Engine, session: &str) -> io::Result<()> {
    let diagnostics = engine.take_diagnostics();
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(invalid_data(format!(
            "{session}: engine diagnostics: {}",
            diagnostics.join("; ")
        )))
    }
}

/// The text the user intends for `word` (committed in its target language).
struct Expected {
    text: String,
    ambiguous: bool,
    lang: Lang,
}

fn expected(word: &ScriptWord) -> Expected {
    let tr = nimk_telex::transform(&word.raw);
    let ambiguous = tr.text != word.raw && tr.valid_vi;
    let text = match word.lang {
        Lang::Vi => tr.text,
        Lang::En => word.raw.clone(),
    };
    Expected {
        text,
        ambiguous,
        lang: word.lang,
    }
}

/// Engine settings used for both training and evaluation.
pub(crate) fn pretrain_config(cfg: EngineConfig, opts: PretrainOptions) -> EngineConfig {
    EngineConfig {
        hotkeys: false,
        threading: Threading::Inline,
        snapshot_every_secs: f64::MAX,
        teach_batch: opts.batch.max(1),
        replay_every: 0,
        ..cfg
    }
}

/// Evaluates the snapshot that [`pretrain`] saved in `storage` on arbitrary
/// `scripts` (for example a focused slice such as lone-`w` typing), without
/// modifying `storage`. `cfg` and `opts` must match the `pretrain` call.
///
/// # Errors
///
/// `InvalidData` if the snapshot is missing or the evaluation log does not
/// line up with the keys; I/O errors from reading the snapshot.
pub fn evaluate_snapshot(
    scripts: &[Script],
    cfg: EngineConfig,
    opts: PretrainOptions,
    storage: &Arc<dyn Storage>,
) -> io::Result<EvalReport> {
    evaluate(scripts, &pretrain_config(cfg, opts), storage)
}

/// Fresh engine over an isolated copy of the snapshot saved in `storage`.
pub(crate) fn eval_engine(
    cfg: &EngineConfig,
    storage: &Arc<dyn Storage>,
) -> io::Result<(Engine, MemStorage)> {
    // Isolated copy: the evaluation engine neither logs into nor saves over
    // the caller's storage.
    let mem = MemStorage::new();
    let key = snapshot_key(cfg.policy);
    let bytes = storage
        .blobs()
        .get(&key)?
        .ok_or_else(|| invalid_data(format!("snapshot {key} missing after training")))?;
    mem.blobs().put(&key, &bytes)?;
    let mut engine = Engine::new(cfg.clone(), Arc::new(mem.clone()), session(EVAL_SESSION));
    check_diagnostics(&mut engine, "eval start")?;
    Ok((engine, mem))
}

/// Feeds `scripts` through a fresh engine holding the saved snapshot.
fn evaluate(
    scripts: &[Script],
    cfg: &EngineConfig,
    storage: &Arc<dyn Storage>,
) -> io::Result<EvalReport> {
    let mut report = EvalReport::default();
    if scripts.is_empty() {
        return Ok(report);
    }
    let (mut engine, mem) = eval_engine(cfg, storage)?;
    let mut targets: Vec<Action> = Vec::new();
    let mut expect_words: Vec<Expected> = Vec::new();
    for (field_id, script) in (0u64..).zip(scripts) {
        let t0 = script.keys.first().map_or(0.0, |k| k.timestamp);
        let mut committed: Vec<String> = Vec::new();
        committed.extend(
            engine
                .on_context(&synthetic_context(&script.bundle_id, field_id, t0))
                .commit,
        );
        for key in &script.keys {
            committed.extend(engine.on_key(key).commit);
        }
        let end = script.keys.last().map_or(t0, |k| k.timestamp);
        committed.extend(engine.commit_pending(end).commit);
        check_diagnostics(&mut engine, &script.session)?;

        let words: Vec<Expected> = script.words.iter().map(expected).collect();
        let total = words.len().max(committed.len());
        // A committed chunk carries its boundary ("tiếng ", ADR 0011).
        let matched = words
            .iter()
            .zip(&committed)
            .filter(|(w, c)| w.text == c.trim_end_matches(|ch: char| !ch.is_alphanumeric()))
            .count();
        report.scripts += 1;
        report.text_total += total as u64;
        report.text_matched += matched as u64;
        report.scripts_exact += u64::from(matched == total);
        targets.extend_from_slice(&script.targets);
        expect_words.extend(words);
    }
    engine.shutdown().map_err(engine_err)?;

    let (key_count, decisions) = word_decisions(&mem)?;
    if key_count != targets.len() {
        return Err(invalid_data(format!(
            "eval logged {key_count} key records for {} keys",
            targets.len()
        )));
    }
    // Keys the model never sees (ADR 0011: boundary keys joining a pending
    // word, Return, shortcuts) log no decision; align by key index.
    let mut by_key: Vec<Option<Decided>> = vec![None; targets.len()];
    for (idx, d) in decisions {
        if let Some(slot) = by_key.get_mut(idx) {
            *slot = Some(d);
        }
    }
    let mut next_word = 0usize;
    let mut shown: Option<Lang> = None;
    for (d, target) in by_key.iter().zip(&targets) {
        let mut word = (false, target.render);
        if target.edit == EditAction::Commit {
            if let Some(w) = expect_words.get(next_word) {
                word = (w.ambiguous, w.lang);
            }
            next_word += 1;
        }
        if let Some(d) = d {
            report.count(*d, *target, word);
            report.count_display(*d, *target, &mut shown);
        }
    }
    Ok(report)
}

/// Number of key records and every word-level `Decision` of the in-memory
/// log, each with the index of the key that triggered it.
pub(crate) fn word_decisions(mem: &MemStorage) -> io::Result<(usize, Vec<(usize, Decided)>)> {
    let source = mem.event_source()?;
    let mut keys = 0usize;
    let mut out = Vec::new();
    for id in source.segments()? {
        for raw in source.read_segment(&id)? {
            let record: Record = serde_json::from_slice(&raw?).map_err(io::Error::other)?;
            match record.event {
                Event::Key { .. } => keys += 1,
                Event::Decision {
                    target: Target::Word(_),
                    edit: Some(edit),
                    chosen,
                    p_vi,
                    at_key,
                    ..
                } if keys > 0 => out.push((
                    keys - 1,
                    Decided {
                        edit,
                        chosen,
                        p_vi,
                        at_key,
                    },
                )),
                _ => {}
            }
        }
    }
    Ok((keys, out))
}

#[cfg(test)]
#[path = "pretrain_tests.rs"]
mod tests;
