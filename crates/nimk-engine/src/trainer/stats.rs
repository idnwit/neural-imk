//! Per-model quality statistics, counted per committed word.
//!
//! Every word with a language decision is recorded for the active model and
//! for each shadow model whose online training is on. The ground truth is
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
//! Persisted as JSON in the blob `stats/<kind>` (versioned).

use std::collections::VecDeque;

use nimk_types::Lang;

/// Words in the rolling window.
pub(crate) const WINDOW: usize = 500;

/// Blob format version.
/// Version 2: accuracy is measured against the shared ground truth and late
/// corrections amend instead of double counting; older blobs start empty.
const VERSION: u32 = 2;

pub use nimk_types::StatCategory;

/// What the engine knows about one finalized word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WordInfo {
    /// Word id (matches the keystrokes' [`super::KeyObs::word_id`]).
    pub(crate) word_id: u64,
    /// Ground truth of the word.
    pub(crate) label: Lang,
    /// The rendering the model chose (committed) for the word.
    pub(crate) committed: Lang,
    /// `Some(previous truth)`: a late correction of a word already counted.
    pub(crate) amends: Option<Lang>,
    /// Category of the app the word was typed in.
    pub(crate) category: StatCategory,
}

/// Display decisions of one word's keystrokes so far.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct WordAcc {
    vi: u16,
    en: u16,
    flips: u16,
    last: Option<Lang>,
}

impl WordAcc {
    /// Adds one keystroke whose display decision was `lang`.
    pub(crate) fn push(&mut self, lang: Lang) {
        match lang {
            Lang::Vi => self.vi = self.vi.saturating_add(1),
            Lang::En => self.en = self.en.saturating_add(1),
        }
        if self.last.is_some_and(|prev| prev != lang) {
            self.flips = self.flips.saturating_add(1);
        }
        self.last = Some(lang);
    }

    /// The latest display decision (at the commit step: the commit
    /// rendering).
    pub(crate) fn last(&self) -> Option<Lang> {
        self.last
    }

    fn keys(&self) -> u16 {
        self.vi.saturating_add(self.en)
    }

    fn matches(&self, label: Lang) -> u16 {
        match label {
            Lang::Vi => self.vi,
            Lang::En => self.en,
        }
    }
}

/// One word as counted by [`StatsTracker::record`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct WordResult {
    pub(crate) word_id: u64,
    /// The model's rendering of the word (active: committed; shadow: its own
    /// decision at the commit step).
    pub(crate) render: Lang,
    pub(crate) category: StatCategory,
    pub(crate) served: bool,
    pub(crate) acc: WordAcc,
    pub(crate) label: Lang,
}

impl WordResult {
    fn corrected(&self) -> bool {
        !nimk_types::word_correct(self.render, self.label)
    }
}

/// A recently counted word, kept in memory so a late correction can amend it.
#[derive(Debug, Clone, Copy)]
struct Recorded {
    /// Sequence number of the record (position in the rolling window).
    seq: u64,
    word: WordResult,
}

/// Recorded words kept for amendments (the labeler keeps as many).
const RECORDED_KEPT: usize = 8;

/// One word of the rolling window: corrected, keystrokes, keystrokes whose
/// display matched the label, flips.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Entry(bool, u16, u16, u16);

/// Counters of one model (a snapshot; see the module docs for the meaning).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelStats {
    /// Words counted over the lifetime of the model's stats.
    pub words_total: u64,
    /// Of those, corrected (or, for a shadow, would-have-been corrected).
    pub corrected_total: u64,
    /// Words in the rolling window (at most 500).
    pub words_recent: u32,
    /// Corrected words in the rolling window.
    pub corrected_recent: u32,
    /// Words (of `words_total`) typed while this model was the active one.
    pub served_words_total: u64,
    /// Lifetime corrected words per [`StatCategory::ALL`].
    pub corrected_by_category: [u64; 5],
    /// Keystrokes counted for display accuracy, lifetime.
    pub keys_total: u64,
    /// Of those, keystrokes whose display decision equalled the final label.
    pub keys_match_total: u64,
    /// Display changes between consecutive keystrokes of a word, lifetime.
    pub flips_total: u64,
    /// `keys_total` over the rolling window.
    pub keys_recent: u32,
    /// `keys_match_total` over the rolling window.
    pub keys_match_recent: u32,
    /// `flips_total` over the rolling window.
    pub flips_recent: u32,
}

impl ModelStats {
    /// Word accuracy over the lifetime: `1 - corrected / words` (1 when
    /// nothing is counted). Same definition as the offline reports.
    #[must_use]
    pub fn word_accuracy_total(&self) -> f64 {
        accuracy(self.words_total, self.corrected_total)
    }

    /// Word accuracy over the rolling window.
    #[must_use]
    pub fn word_accuracy_recent(&self) -> f64 {
        accuracy(
            u64::from(self.words_recent),
            u64::from(self.corrected_recent),
        )
    }
}

fn accuracy(words: u64, corrected: u64) -> f64 {
    if words == 0 {
        1.0
    } else {
        1.0 - corrected as f64 / words as f64
    }
}

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
    pub(crate) fn record(&mut self, w: WordResult) {
        let keys = w.acc.keys();
        let matched = w.acc.matches(w.label);
        let corrected = w.corrected();
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
        t.keys_total = t.keys_total.saturating_add(u64::from(keys));
        t.keys_match_total = t.keys_match_total.saturating_add(u64::from(matched));
        t.flips_total = t.flips_total.saturating_add(u64::from(w.acc.flips));
        if self.recent.len() >= WINDOW {
            self.recent.pop_front();
        }
        self.recent
            .push_back(Entry(corrected, keys, matched, w.acc.flips));
        if self.recorded.len() >= RECORDED_KEPT {
            self.recorded.pop_front();
        }
        self.recorded.push_back(Recorded {
            seq: self.next_seq,
            word: w,
        });
        self.next_seq += 1;
        self.refresh_recent();
        self.dirty = true;
    }

    /// A late correction: the truth of the already counted word `word_id`
    /// is now `label`. Re-evaluates the word against it, adjusting the
    /// lifetime counters and its rolling-window entry if it is still there.
    /// The word count does not change; an unknown word (never counted by
    /// this model, or no longer remembered) is ignored.
    pub(crate) fn amend(&mut self, word_id: u64, label: Lang) {
        let Some(rec) = self
            .recorded
            .iter_mut()
            .rev()
            .find(|r| r.word.word_id == word_id)
        else {
            return;
        };
        let old = rec.word;
        if old.label == label {
            return;
        }
        let mut new = old;
        new.label = label;
        rec.word = new;
        let seq = rec.seq;
        let (old_matched, new_matched) = (old.acc.matches(old.label), new.acc.matches(label));
        let (old_c, new_c) = (old.corrected(), new.corrected());
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
        let pos = i128::from(seq) + self.recent.len() as i128 - i128::from(self.next_seq);
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
    use super::*;

    fn word(
        corrected: bool,
        cat: StatCategory,
        served: bool,
        langs: &[Lang],
        label: Lang,
    ) -> WordResult {
        let mut acc = WordAcc::default();
        for l in langs {
            acc.push(*l);
        }
        WordResult {
            word_id: 0,
            render: if corrected { label.flip() } else { label },
            category: cat,
            served,
            acc,
            label,
        }
    }

    #[test]
    fn window_rolls_and_totals_keep_counting() {
        let mut t = StatsTracker::default();
        for i in 0..(WINDOW + 10) {
            t.record(word(
                i < 10,
                StatCategory::Ide,
                i % 2 == 0,
                &[Lang::Vi],
                Lang::Vi,
            ));
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
        t.record(word(true, StatCategory::Other, true, &[Lang::Vi], Lang::En));
        t.record(word(
            false,
            StatCategory::Other,
            true,
            &[Lang::Vi],
            Lang::Vi,
        ));
        let s = t.summary();
        assert!((s.word_accuracy_total() - 0.5).abs() < 1e-9);
        assert!((s.word_accuracy_recent() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn display_accuracy_and_flips() {
        let mut t = StatsTracker::default();
        t.record(word(
            false,
            StatCategory::Other,
            true,
            &[Lang::Vi, Lang::En, Lang::En, Lang::Vi],
            Lang::En,
        ));
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
        t.record(word(
            true,
            StatCategory::Terminal,
            true,
            &[Lang::Vi, Lang::En],
            Lang::En,
        ));
        t.record(word(false, StatCategory::Other, false, &[], Lang::Vi));
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

    fn rec(id: u64, render: Lang, label: Lang, langs: &[Lang]) -> WordResult {
        let mut acc = WordAcc::default();
        for l in langs {
            acc.push(*l);
        }
        WordResult {
            word_id: id,
            render,
            category: StatCategory::Ide,
            served: true,
            acc,
            label,
        }
    }

    #[test]
    fn amend_flips_correct_to_corrected_without_growing_the_count() {
        let mut t = StatsTracker::default();
        t.record(rec(1, Lang::Vi, Lang::Vi, &[Lang::Vi, Lang::Vi]));
        t.record(rec(2, Lang::Vi, Lang::Vi, &[Lang::Vi]));
        t.amend(1, Lang::En);
        let s = t.summary();
        assert_eq!((s.words_total, s.corrected_total), (2, 1));
        assert_eq!((s.words_recent, s.corrected_recent), (2, 1));
        assert_eq!(s.corrected_by_category, [0, 1, 0, 0, 0]);
        assert_eq!((s.keys_total, s.keys_match_total), (3, 1));
        assert_eq!((s.keys_recent, s.keys_match_recent), (3, 1));
        // Back again, and an unknown word is ignored.
        t.amend(1, Lang::Vi);
        t.amend(99, Lang::En);
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
            t.record(rec(id, Lang::Vi, Lang::Vi, &[Lang::Vi]));
            // Only the last few records are amendable; amend the newest.
        }
        let last = WINDOW as u64 + 2;
        t.amend(last, Lang::En);
        let s = t.summary();
        assert_eq!((s.corrected_total, s.corrected_recent), (1, 1));
        assert_eq!(s.words_recent as usize, WINDOW);
        // A word no longer remembered is ignored.
        t.amend(0, Lang::En);
        assert_eq!(t.summary().corrected_total, 1);
    }
}
