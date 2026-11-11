//! Caret flip (ADR 0015): the hotkey pressed with no pending word switches
//! an earlier word, already in the app, between its renderings. The host
//! finds the word at the caret and performs the edit; this module decides
//! the replacement from the words this engine committed, and labels it.

use nimk_types::{Event, LabelSource, Lang, SwitchScope, Target, Transform};

use super::{Engine, PendingWord};

/// Committed words remembered for caret flips.
pub(super) const HISTORY_KEPT: usize = 200;

/// A committed word, as remembered for caret flips.
#[derive(Debug, Clone)]
pub(super) struct HistWord {
    id: u64,
    /// App the word was typed in (field ids are not stable across clicks in
    /// some apps, so only the app is matched).
    bundle_id: String,
    raw: String,
    vi: String,
    /// Rendering in the app now.
    lang: Lang,
}

impl HistWord {
    fn rendering(&self, lang: Lang) -> &str {
        match lang {
            Lang::Vi => &self.vi,
            Lang::En => &self.raw,
        }
    }
}

/// What the hotkey press just before a caret flip did, so the flip can
/// undo it: with no pending word the press only switched the sticky mode.
#[derive(Debug, Clone, Copy)]
pub(super) struct UndoSwitch {
    /// Sticky mode before the press.
    pub(super) prev: Lang,
    /// Focus-prior correction the press would write (held back until the
    /// next event, so a caret flip can drop it).
    pub(super) focus_label: Option<(Target, Lang)>,
}

/// Which caret-flip strategies the focused app allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RetroPolicy {
    /// Clients that expose their text.
    pub text: bool,
    /// Clients that expose no text (terminal line tracker).
    pub terminal: bool,
}

/// A caret flip the host should apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetroFlip {
    /// Text that replaces the observed word.
    pub replacement: String,
    /// Rendering of the replacement.
    pub to: Lang,
    /// The engine's word, when it was found in its history (labelled).
    pub word_id: Option<u64>,
}

/// The other rendering of a word this engine did not type: a word with
/// Vietnamese letters becomes its Telex keys, a plain ASCII word its Telex
/// rendering when that is a valid syllable.
fn stateless_flip(word: &str) -> Option<(String, Lang)> {
    if word.is_ascii() {
        let st = nimk_telex::TelexTransform.apply(word);
        (st.valid_vi && st.vi != word).then_some((st.vi, Lang::Vi))
    } else {
        let raw = nimk_telex::encode(word);
        (raw != word && raw.is_ascii()).then_some((raw, Lang::En))
    }
}

impl Engine {
    /// Remembers a word committed for real.
    pub(super) fn remember_word(&mut self, p: &PendingWord) {
        if self.history.len() >= HISTORY_KEPT {
            self.history.pop_front();
        }
        // No focus yet: an empty app id, matched only by another such word.
        let bundle_id = match &self.focus {
            Some(f) => f.ctx.bundle_id.clone(),
            None => String::new(),
        };
        self.history.push_back(HistWord {
            id: p.id,
            bundle_id,
            raw: p.raw.clone(),
            vi: p.vi.clone(),
            lang: p.lang,
        });
    }

    /// Writes the held-back focus-prior label of the last hotkey press;
    /// after this a caret flip can no longer undo that press.
    pub(super) fn settle_switch(&mut self, t: f64) {
        let Some(undo) = self.undo_switch.take() else {
            return;
        };
        if let Some((target, to)) = undo.focus_label {
            let finals: Vec<_> = self
                .labeler
                .refinalize(target, to, 1.0, LabelSource::Correction)
                .into_iter()
                .collect();
            self.emit_labels(t, finals);
        }
    }

    /// Caret-flip strategies allowed in the focused app.
    #[must_use]
    pub fn retro_policy(&self) -> RetroPolicy {
        RetroPolicy {
            text: self.live.retro_flip,
            terminal: self.live.retro_flip_terminal,
        }
    }

    /// Caret flip of `observed`, the word at the caret (ADR 0015). Call it
    /// right after a hotkey press that found no word being composed or
    /// pending, before any other event; otherwise it returns `None`.
    ///
    /// The most recent word committed in the focused app whose rendering in
    /// the app equals `observed` is switched to its other rendering: the
    /// press's sticky-mode switch is undone, a `manual_switch` with scope
    /// `retro` is logged and the word is labelled a correction. A word the
    /// engine did not type gets [`stateless_flip`] and no label. `None`
    /// (nothing to flip) leaves the press's mode switch in force.
    pub fn retro_flip(&mut self, observed: &str, timestamp: f64) -> Option<RetroFlip> {
        let undo = self.undo_switch?;
        let bundle = self.focus.as_ref().map(|f| f.ctx.bundle_id.as_str());
        let found = self
            .history
            .iter()
            .rposition(|h| Some(h.bundle_id.as_str()) == bundle && h.rendering(h.lang) == observed);
        let flip = match found.and_then(|i| self.history.get(i)) {
            Some(h) => {
                let to = h.lang.flip();
                let replacement = h.rendering(to).to_owned();
                (replacement != observed).then_some(RetroFlip {
                    replacement,
                    to,
                    word_id: Some(h.id),
                })
            }
            None => stateless_flip(observed).map(|(replacement, to)| RetroFlip {
                replacement,
                to,
                word_id: None,
            }),
        }?;
        self.undo_switch = None;
        self.undo_sticky(undo);
        if let Some(h) = found.and_then(|i| self.history.get_mut(i)) {
            h.lang = flip.to;
        }
        self.hooks.log(
            timestamp,
            Event::ManualSwitch {
                scope: SwitchScope::Retro,
                word_id: flip.word_id,
                to: flip.to,
            },
        );
        if let Some(id) = flip.word_id {
            let target = Target::Word(id);
            match self
                .labeler
                .refinalize(target, flip.to, 1.0, LabelSource::Correction)
            {
                Some(f) => self.emit_labels(timestamp, vec![f]),
                // Out of the labeler's cache: the label is still recorded,
                // with nothing left to train on.
                None => self.hooks.log(
                    timestamp,
                    Event::Label {
                        target,
                        label: flip.to,
                        weight: 1.0,
                        source: LabelSource::Correction,
                    },
                ),
            }
        }
        self.drain_trainer(timestamp);
        Some(flip)
    }

    /// Replay of a logged caret flip: undoes the sticky-mode switch of the
    /// hotkey press before it (the flip's label is replayed separately).
    pub fn replay_retro(&mut self) {
        if let Some(undo) = self.undo_switch.take() {
            self.undo_sticky(undo);
        }
    }

    fn undo_sticky(&mut self, undo: UndoSwitch) {
        self.mode = undo.prev;
        if self.composer.is_empty() {
            self.shown = undo.prev;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stateless_flip_both_ways() {
        assert_eq!(
            stateless_flip("vieejt"),
            Some(("vi\u{1ec7}t".to_owned(), Lang::Vi))
        );
        assert_eq!(
            stateless_flip("ti\u{1ebf}ng"),
            Some(("tieengs".to_owned(), Lang::En))
        );
        assert_eq!(stateless_flip("hello"), None);
        assert_eq!(stateless_flip("abc"), None);
    }
}
