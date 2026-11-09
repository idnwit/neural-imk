//! Per-app statistics category and per-model counters.

use serde::{Deserialize, Serialize};

/// App category of a counted word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// Every category, in the order of `index`.
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

    /// Position in [`StatCategory::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Terminal => 0,
            Self::Ide => 1,
            Self::Browser => 2,
            Self::Editor => 3,
            Self::Other => 4,
        }
    }
}

/// Quality counters of one model, counted per committed word (a snapshot;
/// the meaning of each measure is in `docs/12-metrics.md`). Published by
/// the `model_stats` hook as its summary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
