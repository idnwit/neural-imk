//! Single-word probe of a pre-trained snapshot: what does the model decide
//! for one raw word after a given context?

use std::io;
use std::sync::Arc;

use nimk_engine::EngineConfig;
use nimk_storage::Storage;
use nimk_types::Lang;

use crate::pretrain::{
    PretrainOptions, check_diagnostics, eval_engine, pretrain_config, word_decisions,
};
use crate::{generate::script_of, log::synthetic_context};

/// The model's decision at the commit key of the probed word.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProbeResult {
    /// Rendering chosen (after hysteresis).
    pub chosen: Lang,
    /// Model output before hysteresis.
    pub p_vi: Option<f32>,
}

/// Types `context` (`(language, text)` words; Vietnamese words are typed
/// with [`nimk_telex::encode`]) and then `raw`, with Space after each, into
/// an engine holding the snapshot saved by `pretrain`, and returns the
/// decision at the commit key of `raw`. Nothing is learned or saved.
///
/// # Errors
///
/// `InvalidData` if the snapshot is missing or the engine reports
/// diagnostics; I/O errors from reading the snapshot.
pub fn probe_word(
    cfg: EngineConfig,
    opts: PretrainOptions,
    storage: &Arc<dyn Storage>,
    context: &[(Lang, &str)],
    raw: &str,
) -> io::Result<ProbeResult> {
    let cfg = pretrain_config(cfg, opts);
    let typed: Vec<(Lang, String)> = context
        .iter()
        .map(|&(lang, text)| {
            let keys = match lang {
                Lang::Vi => nimk_telex::encode(text),
                Lang::En => text.to_owned(),
            };
            (lang, keys)
        })
        .collect();
    let mut words: Vec<(Lang, &str, &str)> = context
        .iter()
        .zip(&typed)
        .map(|(&(lang, text), (_, keys))| (lang, text, keys.as_str()))
        .collect();
    words.push((Lang::Vi, raw, raw));
    let script = script_of(&words);

    let (mut engine, mem) = eval_engine(&cfg, storage)?;
    let t0 = script.keys.first().map_or(0.0, |k| k.timestamp);
    engine.on_context(&synthetic_context(0, t0));
    for key in &script.keys {
        engine.on_key(key);
    }
    check_diagnostics(&mut engine, "probe")?;
    engine.shutdown().map_err(io::Error::other)?;
    let (_, decisions) = word_decisions(&mem)?;
    let last = script.keys.len() - 1;
    decisions
        .iter()
        .rev()
        .find(|(i, _)| *i == last)
        .map(|(_, d)| ProbeResult {
            chosen: d.chosen,
            p_vi: d.p_vi,
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no decision at the commit key"))
}

#[cfg(test)]
#[path = "probe_tests.rs"]
mod tests;
