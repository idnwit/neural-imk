//! The switch hotkey (double Shift / Ctrl+Space), including the flip of a
//! pending word (ADR 0011).

use nimk_types::{Event, SwitchScope, Target};

use super::retro::UndoSwitch;
use super::{Engine, EngineOutput, Overlay};

impl Engine {
    /// Hotkey, by when it is pressed:
    /// - mid-word: flips the current word's rendering and locks it;
    /// - while a word is pending: flips the pending word's rendering inside
    ///   the marked text and marks it a correction (labelled when it is
    ///   committed);
    /// - before any typing after focus: flips the sticky mode (focus prior
    ///   correction, written at the next event unless a caret flip undoes
    ///   the press, ADR 0015);
    /// - in raw mode (a command is being typed): flips the sticky mode only,
    ///   with no label;
    /// - otherwise (typed past the last word): flips the sticky mode only.
    ///
    /// The result becomes the sticky mode, and corrections become labels.
    pub(super) fn manual_switch(&mut self, t: f64, out: &mut EngineOutput) {
        self.last_switch_t = Some(t);
        let (scope, word_id, to) = if !self.composer.is_empty() {
            let to = self.shown.flip();
            self.shown = to;
            self.locked = true;
            out.marked = Some(self.composer.rendering(to).to_owned());
            (SwitchScope::InWord, Some(self.composer.word_id()), to)
        } else if let Some(p) = self.pending.as_mut() {
            let to = p.lang.flip();
            p.lang = to;
            p.locked = true;
            out.marked = Some(p.text());
            let id = p.id;
            (SwitchScope::AfterCommit, Some(id), to)
        } else {
            // No word: only the sticky mode flips. A caret flip (ADR 0015)
            // may undo that until the next event, so a focus-prior
            // correction is held back until then (`settle_switch`).
            let to = self.mode.flip();
            let mut focus_label = None;
            let scoped = if self.raw {
                // Raw mode: no label.
                (SwitchScope::AfterCommit, None, to)
            } else if self.committed_since_focus {
                (SwitchScope::AfterCommit, self.composer.last_committed(), to)
            } else {
                focus_label = self.focus.as_ref().map(|f| (Target::Focus(f.id), to));
                (SwitchScope::BeforeTyping, None, to)
            };
            self.undo_switch = Some(UndoSwitch {
                prev: self.mode,
                focus_label,
            });
            scoped
        };
        self.mode = to;
        if self.composer.is_empty() {
            self.shown = to;
        }
        self.hooks
            .log(t, Event::ManualSwitch { scope, word_id, to });
        out.mode = Some(to);
        out.overlay = Some(Overlay::Manual);
        self.drain_trainer(t);
    }
}
