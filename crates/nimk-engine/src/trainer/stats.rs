//! Per-model quality statistics, counted per committed word.
//!
//! Every word that gets a final label is recorded for the active model and
//! for each shadow model whose online training is on. Three measures:
//! - corrected: the committed rendering differs from the final label
//!   (active: the rendering really committed, i.e. the user corrected the
//!   word; shadow: the rendering its own fresh decision at the commit step
//!   would have committed, i.e. it would have been corrected). The
//!   percentage a UI shows is `corrected / words`;
//! - display accuracy: the fraction of the word's keystrokes whose display
//!   decision equals the final label (each model keeps its own hysteresis
//!   state; the active model's is the real one);
//! - flips: display changes between consecutive keystrokes of the word.
//!
//! Counted over the lifetime and over the last [`WINDOW`] words.
//!
//! Persisted as JSON in the blob `stats/<kind>` (versioned).

use std::collections::VecDeque;

use nimk_types::Lang;

/// Words in the rolling window.
pub(crate) const WINDOW: usize = 500;

/// Blob format version.
const VERSION: u32 = 1;

/// App category of a counted word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatCategory {
    /// Terminal emulators.
    Terminal,
    /// Code editors / IDEs.
    Ide,
    /// Web browsers.
    Browser,
    /// Text editors and note apps.
    Editor,
    /// Any other app.
    Other,
}

impl StatCategory {
    /// Every category, in the order of [`ModelStats::corrected_by_category`].
    pub const ALL: [Self; 5] = [
        Self::Terminal,
        Self::Ide,
        Self::Browser,
        Self::Editor,
        Self::Other,
    ];

    /// Maps an `[[app_category]]` name; unknown names are `Other`.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "terminal" => Self::Terminal,
            "ide" => Self::Ide,
            "browser" => Self::Browser,
            "editor" => Self::Editor,
            _ => Self::Other,
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Terminal => 0,
            Self::Ide => 1,
            Self::Browser => 2,
            Self::Editor => 3,
            Self::Other => 4,
        }
    }
}

/// What the engine knows about one finalized word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WordInfo {
    /// Word id (matches the keystrokes' [`super::KeyObs::word_id`]).
    pub(crate) word_id: u64,
    /// Final label.
    pub(crate) label: Lang,
    /// The label contradicts what was committed (the user corrected it).
    pub(crate) corrected: bool,
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
    pub(crate) corrected: bool,
    pub(crate) category: StatCategory,
    pub(crate) served: bool,
    pub(crate) acc: WordAcc,
    pub(crate) label: Lang,
}

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
        let t = &mut self.totals;
        t.words_total = t.words_total.saturating_add(1);
        if w.served {
            t.served_words_total = t.served_words_total.saturating_add(1);
        }
        if w.corrected {
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
            .push_back(Entry(w.corrected, keys, matched, w.acc.flips));
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
            corrected,
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
