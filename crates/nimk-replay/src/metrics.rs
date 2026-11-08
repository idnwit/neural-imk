//! Aligns the replayed engine's decisions with the logged labels.
//!
//! Ground truth is the original log's `Label` records (last one per target
//! wins). Everything else comes from the replayed engine's own log: word
//! commits, decisions and the bundle id each word was typed in. Alignment is
//! by `word_id` / focus id, which the engine assigns mode-independently.

use std::collections::{BTreeMap, HashMap};

use nimk_types::{DecisionRule, Event, Lang, Record, Target};
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
    /// Labelled words whose last replayed decision came from the model
    /// (`rule = null`), not a hard rule.
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
    let mut session: BTreeMap<String, Counts> = BTreeMap::new();
    let mut bundle = NO_BUNDLE.to_owned();
    let mut word_bundle: HashMap<u64, String> = HashMap::new();
    let mut last_rule: HashMap<u64, Option<DecisionRule>> = HashMap::new();
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
                rule,
                chosen,
                ..
            } => {
                let b = match target {
                    Target::Word(w) => bundle_of(&word_bundle, *w, &bundle),
                    Target::Focus(_) => &bundle,
                };
                let c = session.entry(b.to_owned()).or_default();
                let label = labels.get(target).copied();
                if let Target::Word(w) = target {
                    last_rule.insert(*w, *rule);
                } else if let Some(l) = label {
                    c.priors_labelled += 1;
                    c.priors_correct += u64::from(l == *chosen);
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
                word_id, mode: m, ..
            } => {
                let b = bundle_of(&word_bundle, *word_id, &bundle);
                let c = session.entry(b.to_owned()).or_default();
                c.words += 1;
                if let Some(&l) = labels.get(&Target::Word(*word_id)) {
                    let ok = u64::from(l == *m);
                    c.labelled_words += 1;
                    c.correct_words += ok;
                    if matches!(last_rule.get(word_id), Some(None)) {
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

    fn decision(target: Target, rule: Option<DecisionRule>, chosen: Lang) -> Event {
        Event::Decision {
            target,
            at_key: 1,
            policy_id: "p".into(),
            p_vi: None,
            rule,
            chosen,
        }
    }

    fn commit(word_id: u64, mode: Lang) -> Event {
        Event::WordCommit {
            word_id,
            raw: "x".into(),
            vi: "x".into(),
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
            rec(1, decision(Target::Focus(0), None, Lang::En)),
            rec(2, decision(Target::Word(0), None, Lang::Vi)),
            rec(3, commit(0, Lang::Vi)),
            rec(
                4,
                decision(Target::Word(1), Some(DecisionRule::InvalidVi), Lang::En),
            ),
            rec(5, commit(1, Lang::En)),
            rec(6, commit(2, Lang::En)),
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
        };
        assert_eq!(overall, expected);
        assert_eq!(by_bundle.get("app"), Some(&expected));
    }
}
