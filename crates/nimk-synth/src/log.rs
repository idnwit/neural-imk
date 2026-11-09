//! Synthetic scripts as an ordinary event log (`write_log`).

use std::io;

use nimk_engine::EngineConfig;
use nimk_storage::{EventSink, Storage};
use nimk_types::{AppContext, Event, KeyEvent, LabelSource, Record, SCHEMA_VERSION, Target};

use crate::Script;
use crate::track::{Class, Step, Tracker, classify};

/// `policy_id` written in synthetic `SessionStart` records.
const SYNTHETIC_POLICY_ID: &str = "synthetic";
/// `app_version` written in synthetic `SessionStart` records.
pub(crate) const APP_VERSION: &str = "nimk-synth";

/// One key of a script with the word bookkeeping the engine would do.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WalkedKey<'a> {
    /// The key.
    pub(crate) key: &'a KeyEvent,
    /// Word id the engine logs for this key: the current word for letters
    /// and for the boundary key that ends a word, `None` otherwise.
    pub(crate) word_id: Option<u64>,
    /// Index of the word this boundary key ends (commits), if any.
    pub(crate) ends_word: Option<usize>,
    /// The ending key left the word pending (a boundary character stays
    /// marked); the engine commits it on the next key.
    pub(crate) pending: bool,
}

/// Walks `keys`, assigning word ids the way the engine does (0, 1, 2... per
/// committed non-empty word).
pub(crate) fn walk(keys: &[KeyEvent]) -> impl Iterator<Item = WalkedKey<'_>> {
    let mut next = 0usize;
    let mut tracker = Tracker::new();
    keys.iter().map(move |key| {
        let class = classify(key);
        let step = class.map_or(Step::Pass, |c| tracker.step(c));
        match step {
            Step::Append | Step::Pop => WalkedKey {
                key,
                word_id: Some(next as u64),
                ends_word: None,
                pending: false,
            },
            Step::EndWord => {
                let ended = next;
                next += 1;
                WalkedKey {
                    key,
                    word_id: Some(ended as u64),
                    ends_word: Some(ended),
                    pending: matches!(class, Some(Class::Boundary(_))),
                }
            }
            Step::Pass => WalkedKey {
                key,
                word_id: None,
                ends_word: None,
                pending: false,
            },
        }
    })
}

/// Synthetic app context for script number `field_id`.
pub(crate) fn synthetic_context(bundle_id: &str, field_id: u64, timestamp: f64) -> AppContext {
    AppContext {
        bundle_id: bundle_id.to_owned(),
        field_id,
        caret: None,
        timestamp,
    }
}

/// Serializes records of one session with contiguous `seq`.
struct SessionWriter<'a> {
    sink: Box<dyn EventSink>,
    session: &'a str,
    seq: u64,
}

impl SessionWriter<'_> {
    fn write(&mut self, t: f64, event: Event) -> io::Result<()> {
        let record = Record {
            v: SCHEMA_VERSION,
            seq: self.seq,
            t,
            session: self.session.to_owned(),
            event,
        };
        let bytes = serde_json::to_vec(&record).map_err(io::Error::other)?;
        self.sink.append(&bytes)?;
        self.seq += 1;
        Ok(())
    }
}

impl SessionWriter<'_> {
    fn write_label(&mut self, t: f64, word: &crate::ScriptWord) -> io::Result<()> {
        self.write(
            t,
            Event::Label {
                target: Target::Word(word.word_id),
                label: word.lang,
                weight: 1.0,
                source: LabelSource::Correction,
            },
        )
    }
}

/// Writes scripts as an ordinary event log (`SessionStart`, one synthetic
/// `Context`, `Key`s, and a `Label { source: Correction, weight: 1.0 }` per
/// word right after the key that commits it) so `nimk-replay` can
/// benchmark policies on it. One event sink per script, named after
/// [`Script::session`].
///
/// # Errors
/// Returns an I/O error if storage fails, or `InvalidData` if a script's
/// keys spell more words than [`Script::words`] lists.
pub fn write_log(scripts: &[Script], storage: &dyn Storage) -> io::Result<()> {
    let cfg = EngineConfig {
        hotkeys: true,
        ..EngineConfig::default()
    };
    let config_json = serde_json::to_string(&cfg).map_err(io::Error::other)?;
    for script in scripts {
        write_script(script, storage, &config_json)?;
    }
    Ok(())
}

fn write_script(script: &Script, storage: &dyn Storage, config_json: &str) -> io::Result<()> {
    let mut w = SessionWriter {
        sink: storage.event_sink(&script.session)?,
        session: &script.session,
        seq: 0,
    };
    let t0 = script.keys.first().map_or(0.0, |k| k.timestamp);
    w.write(
        t0,
        Event::SessionStart {
            wall_unix_ms: 0,
            app_version: APP_VERSION.to_owned(),
            config_json: config_json.to_owned(),
            policy_id: SYNTHETIC_POLICY_ID.to_owned(),
            utc_offset_min: 0,
        },
    )?;
    let ctx = synthetic_context(&script.bundle_id, 0, t0);
    w.write(
        t0,
        Event::Context {
            bundle_id: ctx.bundle_id,
            field_id: ctx.field_id,
            caret: ctx.caret,
        },
    )?;
    // A label follows its word's commit. A word left pending by a boundary
    // character is committed by the next key, so its label waits for it.
    let mut held: Option<&crate::ScriptWord> = None;
    for k in walk(&script.keys) {
        let t = k.key.timestamp;
        w.write(
            t,
            Event::Key {
                key: k.key.clone(),
                word_id: k.word_id,
            },
        )?;
        if let Some(word) = held.take() {
            w.write_label(t, word)?;
        }
        if let Some(i) = k.ends_word {
            let word = script.words.get(i).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}: keys spell more than {i} words", script.session),
                )
            })?;
            if k.pending {
                held = Some(word);
            } else {
                w.write_label(t, word)?;
            }
        }
    }
    if let Some(word) = held {
        let t = script.keys.last().map_or(t0, |k| k.timestamp);
        w.write_label(t, word)?;
    }
    w.sink.flush()
}

#[cfg(test)]
mod tests {
    use nimk_storage::MemStorage;
    use nimk_types::Lang;

    use super::*;
    use crate::{Corpus, SynthConfig, generate};

    fn scripts() -> Vec<Script> {
        let cfg = SynthConfig {
            seed: 3,
            sessions: 3,
            words_per_session: 120,
            ..SynthConfig::default()
        };
        generate(&cfg, &Corpus::builtin())
    }

    fn read_all(storage: &MemStorage) -> Vec<Vec<Record>> {
        let source = storage.event_source().unwrap();
        source
            .segments()
            .unwrap()
            .iter()
            .map(|id| {
                source
                    .read_segment(id)
                    .unwrap()
                    .map(|r| serde_json::from_slice::<Record>(&r.unwrap()).unwrap())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn records_are_contiguous_with_one_label_per_word() {
        let scripts = scripts();
        let storage = MemStorage::new();
        write_log(&scripts, &storage).unwrap();
        let segments = read_all(&storage);
        assert_eq!(segments.len(), scripts.len());
        for records in &segments {
            let session = &records[0].session;
            let script = scripts.iter().find(|s| &s.session == session).unwrap();
            for (i, r) in records.iter().enumerate() {
                assert_eq!(r.seq, i as u64);
                assert_eq!(r.v, SCHEMA_VERSION);
                assert_eq!(&r.session, session);
            }
            match &records[0].event {
                Event::SessionStart { config_json, .. } => {
                    let cfg: EngineConfig = serde_json::from_str(config_json).unwrap();
                    assert!(cfg.hotkeys);
                }
                other => panic!("first record is {other:?}"),
            }
            assert!(matches!(
                &records[1].event,
                Event::Context { bundle_id, field_id: 0, caret: None } if *bundle_id == script.bundle_id
            ));
            let keys = records
                .iter()
                .filter(|r| matches!(r.event, Event::Key { .. }))
                .count();
            assert_eq!(keys, script.keys.len());
            let labels: Vec<(u64, Lang)> = records
                .iter()
                .enumerate()
                .filter_map(|(i, r)| match &r.event {
                    Event::Label {
                        target: Target::Word(id),
                        label,
                        weight,
                        source: LabelSource::Correction,
                    } if *weight == 1.0 => {
                        // The label follows a key of its own word.
                        assert!(
                            records[..i].iter().any(|p| matches!(
                                p.event,
                                Event::Key { word_id: Some(k), .. } if k == *id
                            )),
                            "label {id} before its word"
                        );
                        Some((*id, *label))
                    }
                    _ => None,
                })
                .collect();
            let expected: Vec<(u64, Lang)> =
                script.words.iter().map(|w| (w.word_id, w.lang)).collect();
            assert_eq!(labels, expected);
        }
    }

    #[test]
    fn walk_assigns_engine_word_ids() {
        let script = &scripts()[0];
        let ends: Vec<usize> = walk(&script.keys).filter_map(|k| k.ends_word).collect();
        assert_eq!(ends, (0..script.words.len()).collect::<Vec<_>>());
    }
}
