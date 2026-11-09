//! What the trainer knows about one word: its finalization info, its
//! keystrokes' display decisions and its score inputs.
//!
//! The scores themselves are counted by the `model_stats` hook
//! (`nimk-hook`), from the `model_score` records the engine logs.

use nimk_types::{Lang, StatCategory};

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

    /// Keystrokes with a display decision.
    pub(crate) fn keys(&self) -> u16 {
        self.vi.saturating_add(self.en)
    }

    /// Keystrokes whose display decision was `label`.
    pub(crate) fn matches(&self, label: Lang) -> u16 {
        match label {
            Lang::Vi => self.vi,
            Lang::En => self.en,
        }
    }

    /// Display changes between consecutive keystrokes.
    pub(crate) fn flips(&self) -> u16 {
        self.flips
    }
}

/// A word a core scored, kept so a late correction can re-score it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Scored {
    pub(crate) word_id: u64,
    /// Keystroke decisions of the word.
    pub(crate) acc: WordAcc,
    /// The model's rendering (active: committed; shadow: its own decision
    /// at the commit step).
    pub(crate) render: Lang,
    /// Current ground truth.
    pub(crate) label: Lang,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acc_counts_keys_matches_and_flips() {
        let mut acc = WordAcc::default();
        for l in [Lang::Vi, Lang::En, Lang::En, Lang::Vi] {
            acc.push(l);
        }
        assert_eq!((acc.keys(), acc.matches(Lang::En), acc.flips()), (4, 2, 2));
        assert_eq!(acc.last(), Some(Lang::Vi));
    }
}
