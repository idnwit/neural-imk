//! Labeler: turns commits, focus priors and corrections into final labels.
//!
//! Every committed word (and every focus prior) becomes a pending item that
//! keeps its [`Step`]s: the observation and the edit taken at each model
//! call. An item is finalized:
//! - by a correction ([`Labeler::finalize`]) at once, with the given label;
//! - implicitly, with the rendering used and `implicit_weight`, once
//!   `correction_words` further words were committed or `label_timeout_secs`
//!   of event time passed, whichever comes first.
//!
//! A word whose implicit label was already finalized keeps its steps in a
//! small cache ([`DONE_KEPT`] items) so a late correction ([`Labeler::refinalize`],
//! the retroactive double Shift of ADR 0008) can still emit its own labels.
//!
//! A final label becomes one [`Example`] per step: the render target is the
//! label (weighted by the label weight), the edit target is the edit that
//! was taken (weighted by `edit_weight`, 0 by default, ADR 0007).
//!
//! Implicit labels can be disabled (`hotkeys = false`, replay and synthetic
//! pre-training): pending items then wait for [`Labeler::finalize`] so an
//! injected ground-truth label is never preceded by a guessed one. At most
//! [`MAX_PENDING`] items are kept; the oldest is dropped beyond that.

use std::collections::VecDeque;

use nimk_types::{Action, EditAction, Example, LabelSource, Lang, Observation, Target};

/// Cap on pending items (only reached when implicit labels are disabled).
pub(crate) const MAX_PENDING: usize = 256;

/// Implicitly finalized items whose steps are kept for late corrections.
const DONE_KEPT: usize = 4;

/// One model call of a word: what it saw and the edit that was applied.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Step {
    /// Model input.
    pub(crate) obs: Observation,
    /// Edit applied.
    pub(crate) edit: EditAction,
}

/// A label ready to be logged and trained on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FinalLabel {
    /// What the label is for.
    pub(crate) target: Target,
    /// How it was obtained.
    pub(crate) source: LabelSource,
    /// Language the user wanted.
    pub(crate) label: Lang,
    /// Label weight.
    pub(crate) weight: f32,
    /// Training examples, one per step of the word.
    pub(crate) examples: Vec<Example>,
    /// The label contradicts the rendering that was used (a mid-word
    /// manual switch, a retroactive switch, or an injected correction that
    /// disagrees with the commit). Only these trigger correction
    /// re-training; a `Correction` that confirms the commit does not.
    pub(crate) corrected: bool,
}

/// Labeler tuning (from `EngineConfig`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LabelerParams {
    /// Generate implicit labels at all.
    pub(crate) implicit: bool,
    /// Further committed words that close an item's window.
    pub(crate) correction_words: usize,
    /// Event-time seconds that close an item's window.
    pub(crate) timeout_secs: f64,
    /// Weight of implicit labels.
    pub(crate) implicit_weight: f32,
    /// Edit-head weight of the examples (0 = edit head not trained online).
    pub(crate) edit_weight: f32,
}

#[derive(Debug, Clone)]
struct Pending {
    target: Target,
    steps: Vec<Step>,
    mode: Lang,
    t: f64,
    words_after: usize,
}

/// Pending-label bookkeeping.
#[derive(Debug, Clone)]
pub(crate) struct Labeler {
    params: LabelerParams,
    pending: VecDeque<Pending>,
    /// Steps of recently finalized implicit items, oldest first.
    done: VecDeque<(Target, Lang, Vec<Step>)>,
}

impl Labeler {
    /// Creates an empty labeler.
    #[must_use]
    pub(crate) fn new(params: LabelerParams) -> Self {
        Self {
            params,
            pending: VecDeque::with_capacity(8),
            done: VecDeque::with_capacity(DONE_KEPT),
        }
    }

    fn make(
        &self,
        target: Target,
        source: LabelSource,
        label: Lang,
        weight: f32,
        steps: Vec<Step>,
        corrected: bool,
    ) -> FinalLabel {
        let edit_weight = self.params.edit_weight;
        let examples = steps
            .into_iter()
            .map(|s| Example {
                obs: s.obs,
                target: Action {
                    edit: s.edit,
                    render: label,
                },
                edit_weight,
                render_weight: weight,
            })
            .collect();
        FinalLabel {
            target,
            source,
            label,
            weight,
            examples,
            corrected,
        }
    }

    /// Records a focus prior decided at `t` with `mode`.
    pub(crate) fn on_focus(&mut self, focus: Target, steps: Vec<Step>, mode: Lang, t: f64) {
        self.push(Pending {
            target: focus,
            steps,
            mode,
            t,
            words_after: 0,
        });
    }

    /// Records a committed word. Older items count it toward their window
    /// (finalized ones go to `out`). With `correction = Some(label)` the
    /// word itself is finalized at once as a weight-1.0 correction.
    pub(crate) fn on_commit(
        &mut self,
        word: Target,
        steps: Vec<Step>,
        mode: Lang,
        t: f64,
        correction: Option<Lang>,
        out: &mut Vec<FinalLabel>,
    ) {
        for p in &mut self.pending {
            p.words_after += 1;
        }
        if self.params.implicit {
            let limit = self.params.correction_words;
            self.finalize_where(|p| p.words_after >= limit, out);
        }
        match correction {
            Some(label) => {
                let f = self.make(word, LabelSource::Correction, label, 1.0, steps, true);
                out.push(f);
            }
            None => self.push(Pending {
                target: word,
                steps,
                mode,
                t,
                words_after: 0,
            }),
        }
    }

    /// Finalizes items whose timeout elapsed at event time `now`.
    pub(crate) fn tick(&mut self, now: f64, out: &mut Vec<FinalLabel>) {
        if self.params.implicit {
            let timeout = self.params.timeout_secs;
            self.finalize_where(|p| now - p.t >= timeout, out);
        }
    }

    /// Finalizes every pending item implicitly (shutdown). No-op when
    /// implicit labels are disabled.
    pub(crate) fn flush(&mut self, out: &mut Vec<FinalLabel>) {
        if self.params.implicit {
            self.finalize_where(|_| true, out);
        }
    }

    /// Finalizes `target` with an explicit label. Returns `None` if it is
    /// not pending (unknown, or already finalized).
    pub(crate) fn finalize(
        &mut self,
        target: Target,
        label: Lang,
        weight: f32,
        source: LabelSource,
    ) -> Option<FinalLabel> {
        let idx = self.pending.iter().position(|p| p.target == target)?;
        let p = self.pending.remove(idx)?;
        self.remember(target, p.mode, &p.steps);
        Some(self.make(target, source, label, weight, p.steps, label != p.mode))
    }

    /// Like [`Labeler::finalize`], but when `target` was already finalized
    /// implicitly (and is still cached) the correction overrides it: a new
    /// label is built from the cached steps. `None` if unknown.
    pub(crate) fn refinalize(
        &mut self,
        target: Target,
        label: Lang,
        weight: f32,
        source: LabelSource,
    ) -> Option<FinalLabel> {
        if let Some(f) = self.finalize(target, label, weight, source) {
            return Some(f);
        }
        let idx = self.done.iter().position(|(t, _, _)| *t == target)?;
        let (_, mode, steps) = self.done.get(idx)?.clone();
        Some(self.make(target, source, label, weight, steps, label != mode))
    }

    /// Number of pending items.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn pending_len(&self) -> usize {
        self.pending.len()
    }

    fn remember(&mut self, target: Target, mode: Lang, steps: &[Step]) {
        self.done.retain(|(t, _, _)| *t != target);
        if self.done.len() >= DONE_KEPT {
            self.done.pop_front();
        }
        self.done.push_back((target, mode, steps.to_vec()));
    }

    fn push(&mut self, p: Pending) {
        if self.pending.len() >= MAX_PENDING {
            self.pending.pop_front();
        }
        self.pending.push_back(p);
    }

    fn finalize_where(&mut self, due: impl Fn(&Pending) -> bool, out: &mut Vec<FinalLabel>) {
        let weight = self.params.implicit_weight;
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending.get(i).is_some_and(&due) {
                if let Some(p) = self.pending.remove(i) {
                    self.remember(p.target, p.mode, &p.steps);
                    let f = self.make(
                        p.target,
                        LabelSource::Implicit,
                        p.mode,
                        weight,
                        p.steps,
                        false,
                    );
                    out.push(f);
                }
            } else {
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps() -> Vec<Step> {
        vec![
            Step {
                obs: Observation::default(),
                edit: EditAction::Append,
            },
            Step {
                obs: Observation::default(),
                edit: EditAction::Commit,
            },
        ]
    }

    fn params(implicit: bool) -> LabelerParams {
        LabelerParams {
            implicit,
            correction_words: 2,
            timeout_secs: 10.0,
            implicit_weight: 0.3,
            edit_weight: 0.0,
        }
    }

    fn targets(out: &[FinalLabel]) -> Vec<Target> {
        out.iter().map(|f| f.target).collect()
    }

    #[test]
    fn implicit_after_two_words() {
        let mut l = Labeler::new(params(true));
        let mut out = Vec::new();
        for id in 0..3 {
            l.on_commit(
                Target::Word(id),
                steps(),
                Lang::Vi,
                f64::from(id as u32),
                None,
                &mut out,
            );
        }
        assert_eq!(targets(&out), vec![Target::Word(0)]);
        assert_eq!(out[0].source, LabelSource::Implicit);
        assert_eq!(out[0].label, Lang::Vi);
        assert!((out[0].weight - 0.3).abs() < 1e-6);
        assert_eq!(l.pending_len(), 2);
        // One example per step: render = label, edit = edit taken.
        let ex = &out[0].examples;
        assert_eq!(ex.len(), 2);
        assert_eq!(ex[1].target.edit, EditAction::Commit);
        assert_eq!(ex[1].target.render, Lang::Vi);
        assert!((ex[1].render_weight - 0.3).abs() < 1e-6);
        assert!(ex[1].edit_weight.abs() < 1e-6);
    }

    #[test]
    fn timeout_by_event_time() {
        let mut l = Labeler::new(params(true));
        let mut out = Vec::new();
        l.on_focus(Target::Focus(0), steps(), Lang::En, 0.0);
        l.on_commit(Target::Word(0), steps(), Lang::En, 5.0, None, &mut out);
        l.tick(9.9, &mut out);
        assert!(out.is_empty());
        l.tick(10.0, &mut out);
        assert_eq!(targets(&out), vec![Target::Focus(0)]);
        l.tick(15.0, &mut out);
        assert_eq!(targets(&out), vec![Target::Focus(0), Target::Word(0)]);
    }

    #[test]
    fn correction_replaces_implicit() {
        let mut l = Labeler::new(params(true));
        let mut out = Vec::new();
        l.on_commit(Target::Word(0), steps(), Lang::En, 0.0, None, &mut out);
        let f = l.finalize(Target::Word(0), Lang::Vi, 1.0, LabelSource::Correction);
        assert_eq!(
            f.map(|f| (f.label, f.source, f.corrected)),
            Some((Lang::Vi, LabelSource::Correction, true))
        );
        // A correction equal to the committed rendering only confirms it.
        l.on_commit(Target::Word(1), steps(), Lang::En, 0.0, None, &mut out);
        let f = l.finalize(Target::Word(1), Lang::En, 1.0, LabelSource::Correction);
        assert_eq!(f.map(|f| f.corrected), Some(false));
        assert!(
            l.finalize(Target::Word(0), Lang::Vi, 1.0, LabelSource::Correction)
                .is_none()
        );
        l.tick(100.0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn refinalize_overrides_implicit() {
        let mut l = Labeler::new(params(true));
        let mut out = Vec::new();
        l.on_commit(Target::Word(0), steps(), Lang::En, 0.0, None, &mut out);
        l.tick(100.0, &mut out);
        assert_eq!(out.len(), 1);
        assert!(
            l.finalize(Target::Word(0), Lang::Vi, 1.0, LabelSource::Correction)
                .is_none()
        );
        let f = l
            .refinalize(Target::Word(0), Lang::Vi, 1.0, LabelSource::Correction)
            .unwrap();
        assert_eq!(
            (f.label, f.source, f.examples.len(), f.corrected),
            (Lang::Vi, LabelSource::Correction, 2, true)
        );
        assert!(!out[0].corrected, "implicit labels confirm");
        // A later correction overrides again (back to the committed En).
        let f = l.refinalize(Target::Word(0), Lang::En, 1.0, LabelSource::Correction);
        assert_eq!(f.map(|f| f.corrected), Some(false));
        assert!(
            l.refinalize(Target::Word(9), Lang::En, 1.0, LabelSource::Correction)
                .is_none()
        );
    }

    #[test]
    fn in_word_correction_is_immediate() {
        let mut l = Labeler::new(params(true));
        let mut out = Vec::new();
        l.on_commit(
            Target::Word(0),
            steps(),
            Lang::Vi,
            0.0,
            Some(Lang::Vi),
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].source, LabelSource::Correction);
        assert!(out[0].corrected, "a mid-word switch is a correction");
        assert_eq!(l.pending_len(), 0);
    }

    #[test]
    fn disabled_implicit_waits_for_injection() {
        let mut l = Labeler::new(params(false));
        let mut out = Vec::new();
        for id in 0..5 {
            l.on_commit(Target::Word(id), steps(), Lang::En, 0.0, None, &mut out);
        }
        l.tick(1e6, &mut out);
        l.flush(&mut out);
        assert!(out.is_empty());
        assert!(
            l.finalize(Target::Word(0), Lang::Vi, 1.0, LabelSource::Correction)
                .is_some()
        );
        for id in 0..(MAX_PENDING as u64 + 10) {
            l.on_commit(
                Target::Word(100 + id),
                steps(),
                Lang::En,
                0.0,
                None,
                &mut out,
            );
        }
        assert_eq!(l.pending_len(), MAX_PENDING);
    }
}
