//! Per-model quality statistics, counted per committed word.
//!
//! The trainer scores every word with a language decision for the active
//! model and for each shadow model whose online training is on, and the
//! engine logs each score as a `model_score` record. The ground truth is
//! defined once ([`nimk_types::ground_truth`], `docs/12-metrics.md`): the
//! committed rendering when the user did not correct it, the language the
//! user switched to otherwise. Measures:
//! - word accuracy: the rendering equals the truth (active: the rendering
//!   really committed, so accuracy is `1 - corrected / words`; shadow: the
//!   rendering its own fresh decision at the commit step would have
//!   committed). `corrected` counts the words that are not accurate;
//! - display accuracy: the fraction of the word's keystrokes whose display
//!   decision equals the truth (each model keeps its own hysteresis
//!   state; the active model's is the real one);
//! - flips: display changes between consecutive keystrokes of the word.
//!
//! Counted over the lifetime and over the last [`WINDOW`] words. A late
//! correction of a word already counted amends its record
//! ([`StatsTracker::amend`]); the word count never grows twice.
//!
//! Persisted as JSON in the blob `hooks/model_stats/<kind>` (versioned;
//! formerly `stats/<kind>`, same format).

use std::collections::VecDeque;

use nimk_types::{ModelStats, StatCategory};

/// Words in the rolling window.
pub(crate) const WINDOW: usize = 500;

/// Blob format version.
/// Version 2: accuracy is measured against the shared ground truth and late
/// corrections amend instead of double counting; older blobs start empty.
const VERSION: u32 = 2;

/// One scored word as counted by [`StatsTracker::record`] (the counts the
/// trainer derived from the word's keystrokes and its ground truth).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scored {
    /// Word id.
    pub(crate) word_id: u64,
    /// Category of the app the word was typed in.
    pub(crate) category: StatCategory,
    /// The model was the active one for the word.
    pub(crate) served: bool,
    /// The model's rendering equalled the ground truth.
    pub(crate) correct: bool,
    /// Keystrokes of the word with a display decision.
    pub(crate) keys: u16,
    /// Of those, keystrokes whose display decision equalled the truth.
    pub(crate) matched: u16,
    /// Display changes between consecutive keystrokes of the word.
    pub(crate) flips: u16,
}

/// A recently counted word, kept in memory so a late correction can amend it.
#[derive(Debug, Clone, Copy)]
struct Recorded {
    /// Sequence number of the record (position in the rolling window).
    seq: u64,
    word_id: u64,
    category: StatCategory,
    correct: bool,
    matched: u16,
}

/// Recorded words kept for amendments (the labeler keeps as many).
const RECORDED_KEPT: usize = 8;

/// One word of the rolling window: corrected, keystrokes, keystrokes whose
/// display matched the label, flips.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Entry(bool, u16, u16, u16);

#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    version: u32,
    words_total: u64,
    corrected_total: u64,
    served_words_total: u64,
    corrected_by_category: [u64; 5],
    keys_total: u64,
    keys_match_total: u64,
    flips_total: u64,
    /// Rolling window, oldest first.
    recent: Vec<Entry>,
}

/// Counters plus the rolling window.
#[derive(Debug, Clone, Default)]
pub(crate) struct StatsTracker {
    totals: ModelStats,
    recent: VecDeque<Entry>,
    recorded: VecDeque<Recorded>,
    next_seq: u64,
    dirty: bool,
}

impl StatsTracker {
    /// Restores a tracker from the blob; `Err` carries the reason when the
    /// bytes are not a usable version-1 file (the caller starts empty).
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let s: Saved = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if s.version != VERSION {
            return Err(format!("unsupported stats version {}", s.version));
        }
        let skip = s.recent.len().saturating_sub(WINDOW);
        let recent: VecDeque<Entry> = s.recent.iter().skip(skip).copied().collect();
        let mut tracker = Self {
            totals: ModelStats {
                words_total: s.words_total,
                corrected_total: s.corrected_total,
                served_words_total: s.served_words_total,
                corrected_by_category: s.corrected_by_category,
                keys_total: s.keys_total,
                keys_match_total: s.keys_match_total,
                flips_total: s.flips_total,
                ..ModelStats::default()
            },
            recent,
            recorded: VecDeque::new(),
            next_seq: 0,
            dirty: false,
        };
        tracker.refresh_recent();
        Ok(tracker)
    }

    /// Serialized form for the blob.
    ///
    /// # Errors
    /// The serializer's error (not expected for plain counters).
    pub(crate) fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let saved = Saved {
            version: VERSION,
            words_total: self.totals.words_total,
            corrected_total: self.totals.corrected_total,
            served_words_total: self.totals.served_words_total,
            corrected_by_category: self.totals.corrected_by_category,
            keys_total: self.totals.keys_total,
            keys_match_total: self.totals.keys_match_total,
            flips_total: self.totals.flips_total,
            recent: self.recent.iter().copied().collect(),
        };
        serde_json::to_vec(&saved)
    }

    fn refresh_recent(&mut self) {
        let (mut corrected, mut keys, mut matched, mut flips) = (0u32, 0u32, 0u32, 0u32);
        for Entry(c, k, m, f) in &self.recent {
            corrected += u32::from(*c);
            keys += u32::from(*k);
            matched += u32::from(*m);
            flips += u32::from(*f);
        }
        // At most WINDOW entries.
        self.totals.words_recent = self.recent.len().min(WINDOW) as u32;
        self.totals.corrected_recent = corrected;
        self.totals.keys_recent = keys;
        self.totals.keys_match_recent = matched;
        self.totals.flips_recent = flips;
    }

    /// Counts one word.
    pub(crate) fn record(&mut self, w: Scored) {
        let corrected = !w.correct;
        let t = &mut self.totals;
        t.words_total = t.words_total.saturating_add(1);
        if w.served {
            t.served_words_total = t.served_words_total.saturating_add(1);
        }
        if corrected {
            t.corrected_total = t.corrected_total.saturating_add(1);
            if let Some(n) = t.corrected_by_category.get_mut(w.category.index()) {
                *n = n.saturating_add(1);
            }
        }
        t.keys_total = t.keys_total.saturating_add(u64::from(w.keys));
        t.keys_match_total = t.keys_match_total.saturating_add(u64::from(w.matched));
        t.flips_total = t.flips_total.saturating_add(u64::from(w.flips));
        if self.recent.len() >= WINDOW {
            self.recent.pop_front();
        }
        self.recent
            .push_back(Entry(corrected, w.keys, w.matched, w.flips));
        if self.recorded.len() >= RECORDED_KEPT {
            self.recorded.pop_front();
        }
        self.recorded.push_back(Recorded {
            seq: self.next_seq,
            word_id: w.word_id,
            category: w.category,
            correct: w.correct,
            matched: w.matched,
        });
        self.next_seq += 1;
        self.refresh_recent();
        self.dirty = true;
    }

    /// A late correction: the already counted word `word_id` is re-scored
    /// against its new truth as `correct` with `matched` matching
    /// keystrokes. Swaps the old values for the new ones in the lifetime
    /// counters and in its rolling-window entry if it is still there. The
    /// word count does not change; an unknown word (never counted by this
    /// model, or no longer remembered) is ignored.
    pub(crate) fn amend(&mut self, word_id: u64, correct: bool, matched: u16) {
        let Some(rec) = self
            .recorded
            .iter_mut()
            .rev()
            .find(|r| r.word_id == word_id)
        else {
            return;
        };
        let old = *rec;
        rec.correct = correct;
        rec.matched = matched;
        let (old_matched, new_matched) = (old.matched, matched);
        let (old_c, new_c) = (!old.correct, !correct);
        let t = &mut self.totals;
        t.keys_match_total = t
            .keys_match_total
            .saturating_sub(u64::from(old_matched))
            .saturating_add(u64::from(new_matched));
        if old_c != new_c {
            let idx = old.category.index();
            if new_c {
                t.corrected_total = t.corrected_total.saturating_add(1);
                if let Some(n) = t.corrected_by_category.get_mut(idx) {
                    *n = n.saturating_add(1);
                }
            } else {
                t.corrected_total = t.corrected_total.saturating_sub(1);
                if let Some(n) = t.corrected_by_category.get_mut(idx) {
                    *n = n.saturating_sub(1);
                }
            }
        }
        // Window position: this session's records are its tail.
        let pos = i128::from(old.seq) + self.recent.len() as i128 - i128::from(self.next_seq);
        if let Ok(pos) = usize::try_from(pos)
            && let Some(e) = self.recent.get_mut(pos)
        {
            e.0 = new_c;
            e.2 = new_matched;
        }
        self.refresh_recent();
        self.dirty = true;
    }

    /// Current counters.
    pub(crate) fn summary(&self) -> ModelStats {
        self.totals
    }

    /// Whether counters changed since the last [`StatsTracker::mark_saved`].
    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn mark_saved(&mut self) {
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn word(
        word_id: u64,
        corrected: bool,
        category: StatCategory,
        served: bool,
        keys: u16,
        matched: u16,
        flips: u16,
    ) -> Scored {
        Scored {
            word_id,
            category,
            served,
            correct: !corrected,
            keys,
            matched,
            flips,
        }
    }

    #[test]
    fn window_rolls_and_totals_keep_counting() {
        let mut t = StatsTracker::default();
        for i in 0..(WINDOW + 10) {
            // One Vi keystroke, label Vi.
            t.record(word(0, i < 10, StatCategory::Ide, i % 2 == 0, 1, 1, 0));
        }
        let s = t.summary();
        assert_eq!(s.words_total, (WINDOW + 10) as u64);
        assert_eq!(s.corrected_total, 10);
        assert_eq!(s.words_recent as usize, WINDOW);
        assert_eq!(s.corrected_recent, 0, "the corrected words rolled out");
        assert_eq!(s.corrected_by_category, [0, 10, 0, 0, 0]);
        assert_eq!(s.served_words_total, (WINDOW as u64 + 10) / 2);
        assert_eq!(s.keys_total, (WINDOW + 10) as u64);
        assert_eq!(s.keys_recent as usize, WINDOW);
    }

    #[test]
    fn word_accuracy_is_one_minus_corrected_share() {
        let mut t = StatsTracker::default();
        assert!((t.summary().word_accuracy_total() - 1.0).abs() < 1e-9);
        // [Vi] labelled En, then [Vi] labelled Vi.
        t.record(word(0, true, StatCategory::Other, true, 1, 0, 0));
        t.record(word(0, false, StatCategory::Other, true, 1, 1, 0));
        let s = t.summary();
        assert!((s.word_accuracy_total() - 0.5).abs() < 1e-9);
        assert!((s.word_accuracy_recent() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn display_accuracy_and_flips() {
        let mut t = StatsTracker::default();
        // [Vi, En, En, Vi] labelled En.
        t.record(word(0, false, StatCategory::Other, true, 4, 2, 2));
        let s = t.summary();
        assert_eq!((s.keys_total, s.keys_match_total, s.flips_total), (4, 2, 2));
        assert_eq!(
            (s.keys_recent, s.keys_match_recent, s.flips_recent),
            (4, 2, 2)
        );
    }

    #[test]
    fn bytes_round_trip() {
        let mut t = StatsTracker::default();
        // [Vi, En] labelled En, then no keystroke.
        t.record(word(0, true, StatCategory::Terminal, true, 2, 1, 1));
        t.record(word(0, false, StatCategory::Other, false, 0, 0, 0));
        let back = StatsTracker::from_bytes(&t.to_bytes().unwrap()).unwrap();
        assert_eq!(back.summary(), t.summary());
        assert!(!back.is_dirty());
        assert!(StatsTracker::from_bytes(b"{}").is_err());
        let bad = br#"{"version":9,"words_total":0,"corrected_total":0,"served_words_total":0,"corrected_by_category":[0,0,0,0,0],"keys_total":0,"keys_match_total":0,"flips_total":0,"recent":[]}"#;
        assert!(StatsTracker::from_bytes(bad).is_err());
    }
}

#[cfg(test)]
mod amend_tests {
    use super::*;

    /// A word of category Ide, served, with `keys` keystrokes.
    fn rec(id: u64, correct: bool, keys: u16, matched: u16) -> Scored {
        Scored {
            word_id: id,
            category: StatCategory::Ide,
            served: true,
            correct,
            keys,
            matched,
            flips: 0,
        }
    }

    #[test]
    fn amend_flips_correct_to_corrected_without_growing_the_count() {
        let mut t = StatsTracker::default();
        // Rendered Vi, labelled Vi, keystrokes [Vi, Vi]; then [Vi].
        t.record(rec(1, true, 2, 2));
        t.record(rec(2, true, 1, 1));
        // The truth becomes En: wrong, no keystroke matches.
        t.amend(1, false, 0);
        let s = t.summary();
        assert_eq!((s.words_total, s.corrected_total), (2, 1));
        assert_eq!((s.words_recent, s.corrected_recent), (2, 1));
        assert_eq!(s.corrected_by_category, [0, 1, 0, 0, 0]);
        assert_eq!((s.keys_total, s.keys_match_total), (3, 1));
        assert_eq!((s.keys_recent, s.keys_match_recent), (3, 1));
        // Back again, and an unknown word is ignored.
        t.amend(1, true, 2);
        t.amend(99, false, 0);
        let s = t.summary();
        assert_eq!(
            (s.words_total, s.corrected_total, s.corrected_recent),
            (2, 0, 0)
        );
        assert_eq!(s.keys_match_total, 3);
    }

    #[test]
    fn amend_of_a_word_rolled_out_of_the_window_only_touches_totals() {
        let mut t = StatsTracker::default();
        for id in 0..(WINDOW as u64 + 3) {
            t.record(rec(id, true, 1, 1));
            // Only the last few records are amendable; amend the newest.
        }
        let last = WINDOW as u64 + 2;
        t.amend(last, false, 0);
        let s = t.summary();
        assert_eq!((s.corrected_total, s.corrected_recent), (1, 1));
        assert_eq!(s.words_recent as usize, WINDOW);
        // A word no longer remembered is ignored.
        t.amend(0, false, 0);
        assert_eq!(t.summary().corrected_total, 1);
    }
}
