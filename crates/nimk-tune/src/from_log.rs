//! Real event logs as [`Script`]s: the inverse of [`crate::write_log`].
//!
//! A user's log holds every key the IME received, the language each word
//! really had (a hotkey correction, else the committed mode) and the app it
//! was typed in. That is exactly what pre-training teaches from, so a model
//! can be (re)trained on a user's own history: pre-train on synthetic data,
//! then teach the scripts built here.
//!
//! One log session becomes one script per focus (`Context`) change, so the
//! app (`Script::bundle_id`) stays attached to its keys. Targets are built
//! the way the generator builds them: the edit comes from the engine state
//! machine ([`crate::track`]), the render is the word's real language
//! (sticky across keys that belong to no word).

use std::collections::{BTreeMap, HashMap};
use std::io;

use nimk_storage::EventSource;
use nimk_types::{
    Action, Event, KeyEvent, KeyKind, LabelSource, Lang, Modifiers, Record, SCHEMA_VERSION, Target,
    keycode,
};

use crate::track::{Step, Tracker, classify};
use crate::{Script, ScriptWord};

/// Where a script came from in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptMeta {
    /// Log session the script was typed in.
    pub session: String,
    /// `seq` of the script's last key record.
    pub last_seq: u64,
}

/// Scripts read from a log, with what was left out.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LogScripts {
    /// Scripts in log order (segments sort chronologically).
    pub scripts: Vec<Script>,
    /// Origin of each script, same index as `scripts`.
    pub meta: Vec<ScriptMeta>,
    /// Per session, one past the highest `seq` read.
    pub next_seq: BTreeMap<String, u64>,
    /// Sessions read.
    pub sessions: usize,
    /// Pieces dropped for having fewer than two keys.
    pub skipped: usize,
    /// Words dropped because their ending key had no word id in the log.
    pub dropped_no_id: usize,
    /// Words dropped because the log gives them no language (no commit,
    /// label or switch).
    pub dropped_no_lang: usize,
    /// Lines that did not parse or have another schema version.
    pub malformed: usize,
}

/// What the log says about the words of one session.
#[derive(Default)]
struct Truth {
    /// Final language per word id.
    lang: HashMap<u64, Lang>,
}

impl Truth {
    fn of(records: &[Record]) -> Self {
        let mut t = Self::default();
        // Precedence, weakest first: committed mode, implicit label,
        // correction label, manual switch (the last one wins).
        let mut rank: HashMap<u64, u8> = HashMap::new();
        let mut set = |t: &mut Self, id: u64, lang: Lang, r: u8| {
            if rank.get(&id).is_none_or(|&old| r >= old) {
                rank.insert(id, r);
                t.lang.insert(id, lang);
            }
        };
        for rec in records {
            match &rec.event {
                Event::WordCommit { word_id, mode, .. } => set(&mut t, *word_id, *mode, 0),
                Event::Label {
                    target: Target::Word(id),
                    label,
                    source,
                    ..
                } => {
                    let r = match source {
                        LabelSource::Implicit => 1,
                        LabelSource::Correction => 2,
                    };
                    set(&mut t, *id, *label, r);
                }
                Event::ManualSwitch {
                    word_id: Some(id),
                    to,
                    ..
                } => set(&mut t, *id, *to, 3),
                _ => {}
            }
        }
        t
    }
}

/// A run of keys in one focus.
struct Run {
    bundle_id: String,
    keys: Vec<KeyEvent>,
    word_ids: Vec<Option<u64>>,
    /// `seq` of each key's record.
    seqs: Vec<u64>,
}

impl Run {
    fn new(bundle_id: String) -> Self {
        Self {
            bundle_id,
            keys: Vec::new(),
            word_ids: Vec::new(),
            seqs: Vec::new(),
        }
    }
}

/// Ctrl+Space is the optional switch hotkey in old sessions; it is not a
/// typed key.
fn is_hotkey(key: &KeyEvent) -> bool {
    key.keycode == keycode::SPACE && key.mods.has(Modifiers::CONTROL)
}

/// Builds the scripts of one run.
///
/// A word-ending key the log gives no word id or language for (about 3% in
/// real logs: the engine did not treat it as a word) is dropped with the
/// keys of its word, and the run is cut there: no consistent target exists
/// for those keys, and the two sides stay valid scripts of their own.
fn build(run: Run, truth: &Truth, session: &str, name: &str, out: &mut LogScripts) {
    let mut tracker = Tracker::new();
    let mut last = Lang::Vi;
    let mut seg_start = 0usize;
    let mut word_start = 0usize;
    let mut targets: Vec<Action> = Vec::new();
    let mut words: Vec<ScriptWord> = Vec::new();
    // Letters of the word being typed (what the engine calls `raw`).
    let mut raw = String::new();
    let mut made = 0usize;
    let mut emit =
        |lo: usize, hi: usize, targets: &mut Vec<Action>, words: &mut Vec<ScriptWord>| {
            let keep = hi.saturating_sub(lo);
            if keep < 2 {
                out.skipped += 1;
            } else {
                targets.truncate(keep);
                out.meta.push(ScriptMeta {
                    session: session.to_owned(),
                    last_seq: run.seqs[hi - 1],
                });
                out.scripts.push(Script {
                    session: format!("{name}.{made}"),
                    bundle_id: run.bundle_id.clone(),
                    keys: run.keys[lo..hi].to_vec(),
                    words: std::mem::take(words),
                    targets: std::mem::take(targets),
                });
                made += 1;
            }
            targets.clear();
            words.clear();
        };
    for (i, (key, id)) in run.keys.iter().zip(&run.word_ids).enumerate() {
        let step = classify(key).map_or(Step::Pass, |c| tracker.step(c));
        if step == Step::Append && raw.is_empty() {
            word_start = i;
        }
        if step == Step::EndWord {
            let lang = match id {
                None => Err(true),
                Some(w) => truth.lang.get(w).copied().ok_or(false),
            };
            match lang {
                Ok(lang) => {
                    let text = match lang {
                        Lang::Vi => nimk_telex::transform(&raw).text,
                        Lang::En => raw.clone(),
                    };
                    words.push(ScriptWord {
                        word_id: words.len() as u64,
                        lang,
                        text,
                        raw: std::mem::take(&mut raw),
                    });
                    last = lang;
                }
                Err(no_id) => {
                    if no_id {
                        out.dropped_no_id += 1;
                    } else {
                        out.dropped_no_lang += 1;
                    }
                    // Keep the words typed before this one.
                    emit(seg_start, word_start, &mut targets, &mut words);
                    seg_start = i + 1;
                    word_start = seg_start;
                    tracker = Tracker::new();
                    raw.clear();
                    continue;
                }
            }
        } else if let Some(lang) = id
            .filter(|_| step != Step::Pass)
            .and_then(|w| truth.lang.get(&w))
        {
            last = *lang;
        }
        match step {
            Step::Append => raw.push_str(&key.chars),
            Step::Pop => {
                raw.pop();
            }
            _ => {}
        }
        targets.push(Action {
            edit: step.edit(),
            render: last,
        });
    }
    emit(seg_start, run.keys.len(), &mut targets, &mut words);
}

/// Scripts of one session's records (sorted by `seq`).
fn session_scripts(id: &str, records: &[Record], out: &mut LogScripts) {
    let truth = Truth::of(records);
    let mut bundle = String::new();
    let mut field = None;
    let mut run = Run::new(bundle.clone());
    let mut n = 0usize;
    let flush = |run: Run, out: &mut LogScripts, n: &mut usize| {
        if !run.keys.is_empty() {
            build(run, &truth, id, &format!("log-{id}-{n}"), out);
            *n += 1;
        }
    };
    for rec in records {
        match &rec.event {
            Event::Context {
                bundle_id,
                field_id,
                ..
            } if field != Some(*field_id) || &bundle != bundle_id => {
                let done = std::mem::replace(&mut run, Run::new(bundle_id.clone()));
                flush(done, out, &mut n);
                bundle.clone_from(bundle_id);
                field = Some(*field_id);
            }
            Event::Key { key, word_id } if key.kind == KeyKind::KeyDown && !is_hotkey(key) => {
                run.keys.push(key.clone());
                run.word_ids.push(*word_id);
                run.seqs.push(rec.seq);
            }
            _ => {}
        }
    }
    flush(run, out, &mut n);
}

/// Reads every segment of `source` into scripts, one per focus run.
///
/// # Errors
/// An I/O error from listing or reading a segment; malformed lines are
/// counted in [`LogScripts::malformed`] instead.
pub fn scripts_from_log(source: &dyn EventSource) -> io::Result<LogScripts> {
    let mut out = LogScripts::default();
    let mut order: Vec<String> = Vec::new();
    let mut by_session: HashMap<String, Vec<Record>> = HashMap::new();
    for seg in source.segments()? {
        for line in source.read_segment(&seg)? {
            let line = line?;
            match serde_json::from_slice::<Record>(&line) {
                Ok(r) if r.v == SCHEMA_VERSION => {
                    if !by_session.contains_key(&r.session) {
                        order.push(r.session.clone());
                    }
                    by_session.entry(r.session.clone()).or_default().push(r);
                }
                _ => out.malformed += 1,
            }
        }
    }
    for id in order {
        let Some(mut records) = by_session.remove(&id) else {
            continue;
        };
        records.sort_by_key(|r| r.seq);
        out.sessions += 1;
        if let Some(last) = records.last() {
            out.next_seq.insert(id.clone(), last.seq + 1);
        }
        session_scripts(&id, &records, &mut out);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_correction_overrides_the_committed_mode() {
        use nimk_types::{Boundary, SwitchScope};
        let rec = |seq, event| Record {
            v: SCHEMA_VERSION,
            seq,
            t: 0.0,
            session: "s".into(),
            event,
        };
        let records = vec![
            rec(
                0,
                Event::WordCommit {
                    word_id: 0,
                    raw: "sex".into(),
                    vi: "sẽ".into(),
                    en: "sex".into(),
                    mode: Lang::En,
                    boundary: Boundary::Space,
                },
            ),
            rec(
                1,
                Event::Label {
                    target: Target::Word(0),
                    label: Lang::En,
                    weight: 0.3,
                    source: LabelSource::Implicit,
                },
            ),
            rec(
                2,
                Event::ManualSwitch {
                    scope: SwitchScope::AfterCommit,
                    word_id: Some(0),
                    to: Lang::Vi,
                },
            ),
        ];
        assert_eq!(Truth::of(&records).lang[&0], Lang::Vi);
    }

    #[test]
    fn hotkeys_and_modifier_keys_are_not_typed_keys() {
        let space = |mods| KeyEvent {
            kind: KeyKind::KeyDown,
            keycode: keycode::SPACE,
            chars: " ".into(),
            mods: Modifiers(mods),
            timestamp: 0.0,
        };
        assert!(is_hotkey(&space(Modifiers::CONTROL)));
        assert!(!is_hotkey(&space(0)));
    }
}
