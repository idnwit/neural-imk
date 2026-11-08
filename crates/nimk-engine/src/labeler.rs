//! Labeler: turns commits, focus priors and corrections into final labels.
//!
//! Every committed word (and every focus prior) becomes a pending item that
//! keeps the [`Features`] of its deciding call. An item is finalized:
//! - by a correction ([`Labeler::finalize`]) at once, with the given label;
//! - implicitly, with the mode used and `implicit_weight`, once
//!   `correction_words` further words were committed or `label_timeout_secs`
//!   of event time passed, whichever comes first.
//!
//! Implicit labels can be disabled (`hotkeys = false`, replay and synthetic
//! pre-training): pending items then wait for [`Labeler::finalize`] so an
//! injected ground-truth label is never preceded by a guessed one. At most
//! [`MAX_PENDING`] items are kept; the oldest is dropped beyond that.

use std::collections::VecDeque;

use nimk_types::{Features, LabelSource, Lang, Sample, Target};

/// Cap on pending items (only reached when implicit labels are disabled).
pub(crate) const MAX_PENDING: usize = 256;

/// A label ready to be logged and trained on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FinalLabel {
    /// What the label is for.
    pub(crate) target: Target,
    /// How it was obtained.
    pub(crate) source: LabelSource,
    /// Training example (features captured at decision time).
    pub(crate) sample: Sample,
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
}

#[derive(Debug, Clone)]
struct Pending {
    target: Target,
    features: Features,
    mode: Lang,
    t: f64,
    words_after: usize,
}

/// Pending-label bookkeeping.
#[derive(Debug, Clone)]
pub(crate) struct Labeler {
    params: LabelerParams,
    pending: VecDeque<Pending>,
}

impl Labeler {
    /// Creates an empty labeler.
    #[must_use]
    pub(crate) fn new(params: LabelerParams) -> Self {
        Self {
            params,
            pending: VecDeque::with_capacity(8),
        }
    }

    /// Records a focus prior decided at `t` with `mode`.
    pub(crate) fn on_focus(&mut self, focus: Target, features: Features, mode: Lang, t: f64) {
        self.push(Pending {
            target: focus,
            features,
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
        features: Features,
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
            Some(label) => out.push(FinalLabel {
                target: word,
                source: LabelSource::Correction,
                sample: Sample {
                    features,
                    label,
                    weight: 1.0,
                },
            }),
            None => self.push(Pending {
                target: word,
                features,
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
        Some(FinalLabel {
            target,
            source,
            sample: Sample {
                features: p.features,
                label,
                weight,
            },
        })
    }

    /// Number of pending items.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn pending_len(&self) -> usize {
        self.pending.len()
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
                    out.push(FinalLabel {
                        target: p.target,
                        source: LabelSource::Implicit,
                        sample: Sample {
                            features: p.features,
                            label: p.mode,
                            weight,
                        },
                    });
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

    fn params(implicit: bool) -> LabelerParams {
        LabelerParams {
            implicit,
            correction_words: 2,
            timeout_secs: 10.0,
            implicit_weight: 0.3,
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
                Features::default(),
                Lang::Vi,
                f64::from(id as u32),
                None,
                &mut out,
            );
        }
        assert_eq!(targets(&out), vec![Target::Word(0)]);
        assert_eq!(out[0].source, LabelSource::Implicit);
        assert_eq!(out[0].sample.label, Lang::Vi);
        assert!((out[0].sample.weight - 0.3).abs() < 1e-6);
        assert_eq!(l.pending_len(), 2);
    }

    #[test]
    fn timeout_by_event_time() {
        let mut l = Labeler::new(params(true));
        let mut out = Vec::new();
        l.on_focus(Target::Focus(0), Features::default(), Lang::En, 0.0);
        l.on_commit(
            Target::Word(0),
            Features::default(),
            Lang::En,
            5.0,
            None,
            &mut out,
        );
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
        l.on_commit(
            Target::Word(0),
            Features::default(),
            Lang::En,
            0.0,
            None,
            &mut out,
        );
        let f = l.finalize(Target::Word(0), Lang::Vi, 1.0, LabelSource::Correction);
        assert_eq!(
            f.map(|f| (f.sample.label, f.source)),
            Some((Lang::Vi, LabelSource::Correction))
        );
        assert!(
            l.finalize(Target::Word(0), Lang::Vi, 1.0, LabelSource::Correction)
                .is_none()
        );
        l.tick(100.0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn in_word_correction_is_immediate() {
        let mut l = Labeler::new(params(true));
        let mut out = Vec::new();
        l.on_commit(
            Target::Word(0),
            Features::default(),
            Lang::Vi,
            0.0,
            Some(Lang::Vi),
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].source, LabelSource::Correction);
        assert_eq!(l.pending_len(), 0);
    }

    #[test]
    fn disabled_implicit_waits_for_injection() {
        let mut l = Labeler::new(params(false));
        let mut out = Vec::new();
        for id in 0..5 {
            l.on_commit(
                Target::Word(id),
                Features::default(),
                Lang::En,
                0.0,
                None,
                &mut out,
            );
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
                Features::default(),
                Lang::En,
                0.0,
                None,
                &mut out,
            );
        }
        assert_eq!(l.pending_len(), MAX_PENDING);
    }
}
