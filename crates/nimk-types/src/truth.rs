//! The single definition of ground truth for word-level accuracy, shared by
//! the online statistics, the replay report and the synthetic evaluation.
//! See `docs/12-metrics.md`.

use crate::Lang;

/// Ground truth of a committed word: the language the user wanted.
///
/// - not corrected: the committed rendering is accepted as is;
/// - corrected (mid-word switch, retroactive double-Shift, or a late
///   correction): the language the user switched to.
///
/// Offline, the labelled language of the data plays the correction.
#[must_use]
pub fn ground_truth(committed: Lang, correction: Option<Lang>) -> Lang {
    match correction {
        Some(lang) => lang,
        None => committed,
    }
}

/// A word is correct when the rendering under test equals its ground truth.
/// For the active model the rendering is the committed one, so word accuracy
/// is `1 - corrected share`; for a shadow it is its own decision at the
/// commit step.
#[must_use]
pub fn word_correct(rendering: Lang, truth: Lang) -> bool {
    rendering == truth
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncorrected_is_accepted_corrected_is_the_switch() {
        assert_eq!(ground_truth(Lang::Vi, None), Lang::Vi);
        assert_eq!(ground_truth(Lang::Vi, Some(Lang::En)), Lang::En);
        assert_eq!(ground_truth(Lang::En, Some(Lang::En)), Lang::En);
        assert!(word_correct(Lang::Vi, ground_truth(Lang::Vi, None)));
        assert!(!word_correct(
            Lang::Vi,
            ground_truth(Lang::Vi, Some(Lang::En))
        ));
    }
}
