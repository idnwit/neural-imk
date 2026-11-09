//! Aligns the replayed engine's decisions with the logged labels.
//!
//! Ground truth is the original log's `Label` records (last one per target
//! wins). Everything else comes from the replayed engine's own log: word
//! commits, decisions and the bundle id each word was typed in. Alignment is
//! by `word_id` / focus id, which the engine assigns mode-independently.
//!
//! Edit accuracy compares each replayed decision's edit with the edit the
//! original log recorded for the same `(target, at_key)`; a pair that occurs
//! several times (Append then Commit at the same key count) is matched in
//! order. Decisions without an original edit (old logs, focus priors) are
//! skipped.

use std::collections::{BTreeMap, HashMap};

use nimk_types::{EditAction, Event, Lang, Record, Target};
use serde::{Deserialize, Serialize};

/// Raw counters behind the derived metrics in [`crate::Metrics`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    /// Words committed by the replayed engine.
    pub words: u64,
    /// Committed words with a logged label.
    pub labelled_words: u64,
    /// Labelled words whose mode at commit equals the label.
    pub correct_words: u64,
    /// Labelled words whose Telex and pass-through renderings differ (the
    /// choice mattered).
    pub ambiguous_words: u64,
    /// Ambiguous labelled words that were correct.
    pub ambiguous_correct: u64,
    /// Decisions that changed the mode (policy or hard rule).
    pub auto_switches: u64,
    /// Auto switches whose target has a logged label.
    pub auto_switches_labelled: u64,
    /// Labelled auto switches whose chosen mode equals the label.
    pub auto_switches_agreed: u64,
    /// Focus priors with a logged label.
    pub priors_labelled: u64,
    /// Labelled focus priors that were correct.
    pub priors_correct: u64,
    /// Replayed decisions with a logged original edit to compare against.
    pub edits_compared: u64,
    /// Compared decisions whose edit equals the original's.
    pub edits_agreed: u64,
}

impl Counts {
    /// Adds `other` into `self`.
    pub fn add(&mut self, other: &Self) {
        self.words += other.words;
        self.labelled_words += other.labelled_words;
        self.correct_words += other.correct_words;
        self.ambiguous_words += other.ambiguous_words;
        self.ambiguous_correct += other.ambiguous_correct;
        self.auto_switches += other.auto_switches;
        self.auto_switches_labelled += other.auto_switches_labelled;
        self.auto_switches_agreed += other.auto_switches_agreed;
        self.priors_labelled += other.priors_labelled;
        self.priors_correct += other.priors_correct;
        self.edits_compared += other.edits_compared;
        self.edits_agreed += other.edits_agreed;
    }
}

/// Bundle id used for records seen before any `Context`.
const NO_BUNDLE: &str = "(none)";

/// Bundle a word was typed in, or `current` if none of its keys was seen.
fn bundle_of<'a>(word_bundle: &'a HashMap<u64, String>, word: u64, current: &'a str) -> &'a str {
    match word_bundle.get(&word) {
        Some(b) => b,
        None => current,
    }
}

/// Adds one session's counts to `overall` and to `by_bundle`.
pub(crate) fn accumulate(
    original: &[Record],
    replayed: &[Record],
    overall: &mut Counts,
    by_bundle: &mut BTreeMap<String, Counts>,
) {
    let labels: HashMap<Target, Lang> = original
        .iter()
        .filter_map(|r| match r.event {
            Event::Label { target, label, .. } => Some((target, label)),
            _ => None,
        })
        .collect();
    let mut orig_edits: HashMap<(Target, u16), Vec<EditAction>> = HashMap::new();
    for r in original {
        if let Event::Decision {
            target,
            at_key,
            edit: Some(edit),
            ..
        } = &r.event
        {
            orig_edits
                .entry((*target, *at_key))
                .or_default()
                .push(*edit);
        }
    }
    let mut seen_edits: HashMap<(Target, u16), usize> = HashMap::new();
    let mut session: BTreeMap<String, Counts> = BTreeMap::new();
    let mut bundle = NO_BUNDLE.to_owned();
    let mut word_bundle: HashMap<u64, String> = HashMap::new();
    // Hotkeys are off in replay, so only decisions change the mode.
    let mut mode = Lang::En;
    for r in replayed {
        match &r.event {
            Event::Context { bundle_id, .. } => bundle.clone_from(bundle_id),
            Event::Key {
                word_id: Some(w), ..
            } => {
                word_bundle.entry(*w).or_insert_with(|| bundle.clone());
            }
            Event::Decision {
                target,
                at_key,
                edit,
                chosen,
                ..
            } => {
                let b = match target {
                    Target::Word(w) => bundle_of(&word_bundle, *w, &bundle),
                    Target::Focus(_) => &bundle,
                };
                let c = session.entry(b.to_owned()).or_default();
                let label = labels.get(target).copied();
                if let (Target::Focus(_), Some(l)) = (target, label) {
                    c.priors_labelled += 1;
                    c.priors_correct += u64::from(l == *chosen);
                }
                if let Some(edit) = edit {
                    let key = (*target, *at_key);
                    let n = seen_edits.entry(key).or_insert(0);
                    if let Some(orig) = orig_edits.get(&key).and_then(|v| v.get(*n)) {
                        c.edits_compared += 1;
                        c.edits_agreed += u64::from(orig == edit);
                    }
                    *n += 1;
                }
                if *chosen != mode {
                    mode = *chosen;
                    c.auto_switches += 1;
                    if let Some(l) = label {
                        c.auto_switches_labelled += 1;
                        c.auto_switches_agreed += u64::from(l == *chosen);
                    }
                }
            }
            Event::WordCommit {
                word_id,
                raw,
                vi,
                mode: m,
                ..
            } => {
                let b = bundle_of(&word_bundle, *word_id, &bundle);
                let c = session.entry(b.to_owned()).or_default();
                c.words += 1;
                if let Some(&l) = labels.get(&Target::Word(*word_id)) {
                    let ok = u64::from(nimk_types::word_correct(
                        *m,
                        nimk_types::ground_truth(*m, Some(l)),
                    ));
                    c.labelled_words += 1;
                    c.correct_words += ok;
                    if raw != vi {
                        c.ambiguous_words += 1;
                        c.ambiguous_correct += ok;
                    }
                }
            }
            _ => {}
        }
    }
    for (b, c) in session {
        overall.add(&c);
        by_bundle.entry(b).or_default().add(&c);
    }
}

#[cfg(test)]
mod tests {
    use nimk_types::{Boundary, LabelSource};

    use super::*;

    fn rec(seq: u64, event: Event) -> Record {
        Record {
            v: 1,
            seq,
            t: 0.0,
            session: "s".into(),
            event,
        }
    }

    fn decision_at(target: Target, at_key: u16, edit: Option<EditAction>, chosen: Lang) -> Event {
        Event::Decision {
            target,
            at_key,
            policy_id: "p".into(),
            p_vi: None,
            rule: None,
            chosen,
            edit,
        }
    }

    fn decision(target: Target, chosen: Lang) -> Event {
        decision_at(target, 1, None, chosen)
    }

    /// Commit of a word whose renderings differ iff `ambiguous`.
    fn commit(word_id: u64, mode: Lang, ambiguous: bool) -> Event {
        Event::WordCommit {
            word_id,
            raw: "x".into(),
            vi: if ambiguous { "y" } else { "x" }.into(),
            en: "x".into(),
            mode,
            boundary: Boundary::Space,
        }
    }

    fn label(target: Target, label: Lang) -> Event {
        Event::Label {
            target,
            label,
            weight: 1.0,
            source: LabelSource::Correction,
        }
    }

    #[test]
    fn counts_accuracy_switches_and_priors() {
        let ctx = Event::Context {
            bundle_id: "app".into(),
            field_id: 1,
            caret: None,
        };
        let replayed = vec![
            rec(0, ctx),
            rec(1, decision(Target::Focus(0), Lang::En)),
            rec(2, decision(Target::Word(0), Lang::Vi)),
            rec(3, commit(0, Lang::Vi, true)),
            rec(4, decision(Target::Word(1), Lang::En)),
            rec(5, commit(1, Lang::En, false)),
            rec(6, commit(2, Lang::En, false)),
        ];
        let original = vec![
            rec(0, label(Target::Focus(0), Lang::Vi)),
            rec(1, label(Target::Word(0), Lang::Vi)),
            rec(2, label(Target::Word(1), Lang::Vi)),
        ];
        let mut overall = Counts::default();
        let mut by_bundle = BTreeMap::new();
        accumulate(&original, &replayed, &mut overall, &mut by_bundle);
        let expected = Counts {
            words: 3,
            labelled_words: 2,
            correct_words: 1,
            ambiguous_words: 1,
            ambiguous_correct: 1,
            auto_switches: 2,
            auto_switches_labelled: 2,
            auto_switches_agreed: 1,
            priors_labelled: 1,
            priors_correct: 0,
            edits_compared: 0,
            edits_agreed: 0,
        };
        assert_eq!(overall, expected);
        assert_eq!(by_bundle.get("app"), Some(&expected));
    }

    #[test]
    fn edit_accuracy_matches_by_target_and_key_in_order() {
        use EditAction::{Append, Commit, Pass};
        let w = Target::Word(0);
        let original = vec![
            rec(0, decision_at(w, 1, Some(Append), Lang::Vi)),
            rec(1, decision_at(w, 2, Some(Append), Lang::Vi)),
            rec(2, decision_at(w, 2, Some(Commit), Lang::Vi)),
            rec(3, decision_at(Target::Word(1), 0, None, Lang::Vi)),
        ];
        let replayed = vec![
            rec(0, decision_at(w, 1, Some(Append), Lang::Vi)),
            rec(1, decision_at(w, 2, Some(Append), Lang::Vi)),
            // Same key as the original Commit, but the model passed.
            rec(2, decision_at(w, 2, Some(Pass), Lang::Vi)),
            // No original counterpart: skipped.
            rec(3, decision_at(w, 2, Some(Commit), Lang::Vi)),
            rec(4, decision_at(Target::Word(1), 0, Some(Pass), Lang::Vi)),
            rec(5, decision(Target::Focus(0), Lang::Vi)),
        ];
        let mut overall = Counts::default();
        accumulate(&original, &replayed, &mut overall, &mut BTreeMap::new());
        assert_eq!((overall.edits_compared, overall.edits_agreed), (3, 2));
    }
}
