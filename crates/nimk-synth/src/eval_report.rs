//! Held-out evaluation counters and diagnostics of [`crate::pretrain`].

use nimk_types::{EditAction, Lang};

/// Key positions tracked by [`EvalReport::append_by_pos`] (the last bucket
/// holds every later position).
pub const POS_BUCKETS: usize = 8;

/// Held-out evaluation counters. All rates are 0 when their denominator is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EvalReport {
    /// Held-out scripts.
    pub scripts: u64,
    /// Keys fed (one model decision each).
    pub keys: u64,
    /// Keys where the engine's edit equals the target edit.
    pub edit_correct: u64,
    /// Words (keys whose target is `Commit`).
    pub words: u64,
    /// Words whose chosen rendering at the commit key equals the target.
    pub render_correct: u64,
    /// Words whose model decision at the commit key (`p_vi >= 0.5`, before
    /// hysteresis) equals the target.
    pub render_fresh_correct: u64,
    /// Ambiguous words: Telex text differs from raw and is a valid syllable.
    pub ambiguous: u64,
    /// Ambiguous words with the right rendering.
    pub ambiguous_correct: u64,
    /// Ambiguous words whose intended language is English (the hard case:
    /// `has`, `for`, `user`).
    pub ambiguous_en: u64,
    /// Ambiguous English words rendered English.
    pub ambiguous_en_correct: u64,
    /// Expected committed words (`max` of expected and actual count).
    pub text_total: u64,
    /// Committed words equal to the expected text at the same position.
    pub text_matched: u64,
    /// Scripts whose committed text sequence equals the expected one.
    pub scripts_exact: u64,
    /// `p_vi` at the commit key in 10 bins of 0.1, by target language
    /// (`[0]` = Vietnamese, `[1]` = English).
    pub commit_p_hist: [[u64; 10]; 2],
    /// `(keys, correct)` of the model's render decision on `Append` keys by
    /// word length after the key (`[0]` = first letter, last bucket = 8+).
    pub append_by_pos: [(u64, u64); POS_BUCKETS],
}

fn ratio(n: u64, d: u64) -> f64 {
    if d == 0 { 0.0 } else { n as f64 / d as f64 }
}

/// One logged word-level decision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Decided {
    pub(crate) edit: EditAction,
    pub(crate) chosen: Lang,
    pub(crate) p_vi: Option<f32>,
    pub(crate) at_key: u16,
}

impl EvalReport {
    /// Edit accuracy over all keys.
    #[must_use]
    pub fn edit_accuracy(&self) -> f64 {
        ratio(self.edit_correct, self.keys)
    }

    /// Render accuracy at commit, per word.
    #[must_use]
    pub fn render_accuracy(&self) -> f64 {
        ratio(self.render_correct, self.words)
    }

    /// Accuracy of the model's own decision at commit (no hysteresis).
    #[must_use]
    pub fn render_fresh_accuracy(&self) -> f64 {
        ratio(self.render_fresh_correct, self.words)
    }

    /// Render accuracy on ambiguous words.
    #[must_use]
    pub fn ambiguous_accuracy(&self) -> f64 {
        ratio(self.ambiguous_correct, self.ambiguous)
    }

    /// Render accuracy on ambiguous English words.
    #[must_use]
    pub fn ambiguous_en_accuracy(&self) -> f64 {
        ratio(self.ambiguous_en_correct, self.ambiguous_en)
    }

    /// Fraction of committed words equal to the expected text.
    #[must_use]
    pub fn text_match_rate(&self) -> f64 {
        ratio(self.text_matched, self.text_total)
    }

    /// Fraction of scripts committed exactly as expected.
    #[must_use]
    pub fn script_exact_rate(&self) -> f64 {
        ratio(self.scripts_exact, self.scripts)
    }

    /// Render accuracy on `Append` keys per position bucket.
    #[must_use]
    pub fn append_accuracy_by_pos(&self) -> [f64; POS_BUCKETS] {
        self.append_by_pos.map(|(n, ok)| ratio(ok, n))
    }

    /// Counts one key: `d` is what the engine did, `target` the script's
    /// action, `word` the `(ambiguous, intended)` of the word the key
    /// belongs to (for `Commit` keys).
    pub(crate) fn count(&mut self, d: Decided, target: nimk_types::Action, word: (bool, Lang)) {
        self.keys += 1;
        self.edit_correct += u64::from(d.edit == target.edit);
        let fresh = d.p_vi.map(Lang::from_p_vi);
        match target.edit {
            EditAction::Commit => {
                let ok = u64::from(d.chosen == target.render);
                self.words += 1;
                self.render_correct += ok;
                self.render_fresh_correct += u64::from(fresh == Some(target.render));
                let (ambiguous, lang) = word;
                if ambiguous {
                    self.ambiguous += 1;
                    self.ambiguous_correct += ok;
                    if lang == Lang::En {
                        self.ambiguous_en += 1;
                        self.ambiguous_en_correct += ok;
                    }
                }
                if let Some(p) = d.p_vi {
                    let bin = ((p.clamp(0.0, 1.0) * 10.0) as usize).min(9);
                    let row = usize::from(target.render == Lang::En);
                    self.commit_p_hist[row][bin] += 1;
                }
            }
            EditAction::Append if d.edit == EditAction::Append => {
                let pos = usize::from(d.at_key.max(1) - 1).min(POS_BUCKETS - 1);
                let slot = &mut self.append_by_pos[pos];
                slot.0 += 1;
                slot.1 += u64::from(fresh == Some(target.render));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use nimk_types::Action;

    use super::*;

    fn d(edit: EditAction, chosen: Lang, p: f32, at_key: u16) -> Decided {
        Decided {
            edit,
            chosen,
            p_vi: Some(p),
            at_key,
        }
    }

    fn t(edit: EditAction, render: Lang) -> Action {
        Action { edit, render }
    }

    #[test]
    fn counts_commits_positions_and_histogram() {
        let mut r = EvalReport::default();
        let (vi, en) = (Lang::Vi, Lang::En);
        r.count(
            d(EditAction::Append, vi, 0.9, 1),
            t(EditAction::Append, vi),
            (false, vi),
        );
        r.count(
            d(EditAction::Append, en, 0.2, 9),
            t(EditAction::Append, vi),
            (false, vi),
        );
        r.count(
            d(EditAction::Commit, en, 0.45, 2),
            t(EditAction::Commit, en),
            (true, en),
        );
        r.count(
            d(EditAction::Commit, en, 0.97, 2),
            t(EditAction::Commit, vi),
            (true, vi),
        );
        assert_eq!((r.keys, r.edit_correct, r.words), (4, 4, 2));
        assert_eq!((r.render_correct, r.render_fresh_correct), (1, 2));
        assert_eq!((r.ambiguous, r.ambiguous_correct), (2, 1));
        assert_eq!((r.ambiguous_en, r.ambiguous_en_correct), (1, 1));
        assert_eq!(r.commit_p_hist[1][4], 1);
        assert_eq!(r.commit_p_hist[0][9], 1);
        assert_eq!(r.append_by_pos[0], (1, 1));
        assert_eq!(r.append_by_pos[POS_BUCKETS - 1], (1, 0));
        assert!((r.render_fresh_accuracy() - 1.0).abs() < 1e-9);
    }
}
