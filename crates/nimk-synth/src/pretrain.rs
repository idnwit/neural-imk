//! Synthetic pre-training through an inline engine (`pretrain`).

use std::io;
use std::sync::Arc;

use nimk_engine::{Engine, EngineConfig, EngineError, SessionInfo, Threading};
use nimk_policy::{Model, PolicyKind};
use nimk_storage::{BlobStore, EventSink, EventSource, Storage};
use nimk_types::{LabelSource, Lang, Target};

use crate::log::{APP_VERSION, synthetic_context};
use crate::{Script, ScriptWord};

/// Engine session id used while pre-training (its events are discarded).
const PRETRAIN_SESSION: &str = "nimk-synth-pretrain";

/// Outcome of a pre-training run, measured prequentially: the mode the
/// engine committed for a word is compared with its label before the label
/// is learned.
///
/// Accuracy only counts ambiguous words: Telex rendering differs from the
/// raw keys and is a valid syllable, so the model (not a hard rule) decided.
#[derive(Debug, Clone, PartialEq)]
pub struct PretrainReport {
    /// Words seen (committed and labelled), ambiguous or not.
    pub words: u64,
    /// Ambiguous words, the denominator of the accuracies.
    pub ambiguous: u64,
    /// Ambiguous words where the committed mode matched the label.
    pub correct: u64,
    /// Accuracy over the first 10% of ambiguous words (cold start).
    pub head_accuracy: f64,
    /// Accuracy over the last 10% of ambiguous words (after warm-up).
    pub tail_accuracy: f64,
}

impl PretrainReport {
    /// Overall accuracy on ambiguous words (0 if there were none).
    #[must_use]
    pub fn accuracy(&self) -> f64 {
        ratio(self.correct, self.ambiguous)
    }
}

fn ratio(n: u64, d: u64) -> f64 {
    if d == 0 { 0.0 } else { n as f64 / d as f64 }
}

/// Accuracy over a slice of outcomes (0 if empty).
fn slice_accuracy(outcomes: &[bool]) -> f64 {
    let correct = outcomes.iter().filter(|&&ok| ok).count();
    ratio(correct as u64, outcomes.len() as u64)
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

/// Name prefix of the learner ids of `kind` (`tiny_mlp@v1`).
fn kind_name(kind: PolicyKind) -> &'static str {
    match kind {
        PolicyKind::Baseline => "baseline",
        PolicyKind::Linear => "linear",
        PolicyKind::TinyMlp => "tiny_mlp",
    }
}

fn invalid_data(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

/// Prequential outcome of one committed word.
struct Outcome {
    /// Counts towards accuracy.
    ambiguous: bool,
    /// Committed mode matched the label (always true if both renderings
    /// are equal).
    right: bool,
}

/// Compares the committed text of `word` with its label.
fn judge(word: &ScriptWord, committed: &str, session: &str) -> io::Result<Outcome> {
    let tr = nimk_telex::transform(&word.raw);
    let equal = tr.text == word.raw;
    let mode = if committed == word.raw {
        Lang::En
    } else if committed == tr.text {
        Lang::Vi
    } else {
        return Err(invalid_data(format!(
            "{session}: word {} committed {committed:?}, expected {:?} or {:?}",
            word.word_id, word.raw, tr.text
        )));
    };
    Ok(Outcome {
        ambiguous: !equal && tr.valid_vi,
        right: equal || mode == word.lang,
    })
}

/// Feeds scripts through one inline engine and saves the trained model as
/// the engine's snapshot (`model/<kind>/latest`) in `storage`, the key the
/// IME loads at start.
///
/// All scripts run through a single engine (a model cannot be moved between
/// engines); each script starts with a focus change to a new synthetic
/// field, so word ids continue across scripts. `cfg` is used with
/// `hotkeys = false`, `threading = Inline` and periodic snapshots disabled
/// (the snapshot is saved once, at the end). Right after the boundary key
/// that commits a word, its label is injected with weight 1.0: as a
/// `Correction` if the committed mode was wrong, else as `Implicit` - what a
/// user who fixes every wrong word would produce. Events are not logged
/// (use [`crate::write_log`] for a replayable log).
///
/// # Errors
/// `InvalidInput` if `model` is not of kind `cfg.policy`; `InvalidData` if
/// the engine's commits do not line up with the scripts' words or it
/// reports diagnostics (dropped labels); any I/O error from saving the
/// snapshot.
pub fn pretrain(
    scripts: &[Script],
    cfg: EngineConfig,
    model: Model,
    storage: Arc<dyn Storage>,
) -> io::Result<PretrainReport> {
    let name = kind_name(cfg.policy);
    let learner_id = model.learner.id();
    if !learner_id.starts_with(name) || learner_id.as_bytes().get(name.len()) != Some(&b'@') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("model {learner_id} is not of policy kind {name}"),
        ));
    }
    let cfg = EngineConfig {
        hotkeys: false,
        threading: Threading::Inline,
        snapshot_every_secs: f64::MAX,
        ..cfg
    };
    let session = SessionInfo {
        id: PRETRAIN_SESSION.to_owned(),
        wall_unix_ms: 0,
        app_version: APP_VERSION.to_owned(),
    };
    let storage: Arc<dyn Storage> = Arc::new(ModelOnly(storage));
    let mut engine = Engine::with_model(cfg, storage, session, model);
    let mut run = Run::default();
    for (field_id, script) in (0u64..).zip(scripts) {
        run.script(&mut engine, field_id, script)?;
    }
    engine.shutdown().map_err(|e| match e {
        EngineError::Storage(io) => io,
    })?;
    Ok(run.report())
}

/// Running totals of a pre-training run.
#[derive(Default)]
struct Run {
    /// Engine word id of the first word of the next script.
    base: u64,
    words: u64,
    /// Prequential outcomes of ambiguous words, in order.
    outcomes: Vec<bool>,
}

impl Run {
    fn script(&mut self, engine: &mut Engine, field_id: u64, script: &Script) -> io::Result<()> {
        let t0 = script.keys.first().map_or(0.0, |k| k.timestamp);
        engine.on_context(&synthetic_context(field_id, t0));
        let mut next = 0usize;
        for key in &script.keys {
            let Some(committed) = engine.on_key(key).commit else {
                continue;
            };
            let word = script.words.get(next).ok_or_else(|| {
                invalid_data(format!("{}: more commits than words", script.session))
            })?;
            let outcome = judge(word, &committed, &script.session)?;
            if outcome.ambiguous {
                self.outcomes.push(outcome.right);
            }
            let source = if outcome.right {
                LabelSource::Implicit
            } else {
                LabelSource::Correction
            };
            engine.inject_label(
                Target::Word(self.base + next as u64),
                word.lang,
                1.0,
                source,
            );
            next += 1;
        }
        if next != script.words.len() {
            return Err(invalid_data(format!(
                "{}: {next} commits for {} words",
                script.session,
                script.words.len()
            )));
        }
        let diagnostics = engine.take_diagnostics();
        if !diagnostics.is_empty() {
            return Err(invalid_data(format!(
                "{}: engine diagnostics: {}",
                script.session,
                diagnostics.join("; ")
            )));
        }
        self.base += next as u64;
        self.words += next as u64;
        Ok(())
    }

    fn report(&self) -> PretrainReport {
        let n = self.outcomes.len();
        let tenth = n.div_ceil(10);
        // `tenth <= n`, so both splits are in bounds.
        let (head, _) = self.outcomes.split_at(tenth);
        let (_, tail) = self.outcomes.split_at(n - tenth);
        let correct = self.outcomes.iter().filter(|&&ok| ok).count();
        PretrainReport {
            words: self.words,
            ambiguous: n as u64,
            correct: correct as u64,
            head_accuracy: slice_accuracy(head),
            tail_accuracy: slice_accuracy(tail),
        }
    }
}

#[cfg(test)]
#[path = "pretrain_tests.rs"]
mod tests;
