//! The switch hotkey (double Shift / Ctrl+Space), including the flip of a
//! pending word (ADR 0011).

use nimk_types::{Event, LabelSource, SwitchScope, Target};

use super::{Engine, EngineOutput, Overlay};

impl Engine {
    /// Hotkey, by when it is pressed:
    /// - mid-word: flips the current word's rendering and locks it;
    /// - while a word is pending: flips the pending word's rendering inside
    ///   the marked text and marks it a correction (labelled when it is
    ///   committed);
    /// - before any typing after focus: flips the sticky mode (focus prior
    ///   correction);
    /// - in raw mode (a command is being typed): flips the sticky mode only,
    ///   with no label;
    /// - otherwise (typed past the last word): flips the sticky mode only.
    ///
    /// The result becomes the sticky mode, and corrections become labels.
    pub(super) fn manual_switch(&mut self, t: f64, out: &mut EngineOutput) {
        self.last_switch_t = Some(t);
        let mut label: Option<(Target, f32)> = None;
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
        } else if self.raw {
            // Raw mode: flips the sticky mode only, no label.
            (SwitchScope::AfterCommit, None, self.mode.flip())
        } else if self.committed_since_focus {
            (
                SwitchScope::AfterCommit,
                self.composer.last_committed(),
                self.mode.flip(),
            )
        } else {
            if let Some(f) = &self.focus {
                label = Some((Target::Focus(f.id), 1.0));
            }
            (SwitchScope::BeforeTyping, None, self.mode.flip())
        };
        self.mode = to;
        if self.composer.is_empty() {
            self.shown = to;
        }
        self.hooks
            .log(t, Event::ManualSwitch { scope, word_id, to });
        let finals: Vec<_> = label
            .and_then(|(target, weight)| {
                self.labeler
                    .refinalize(target, to, weight, LabelSource::Correction)
            })
            .into_iter()
            .collect();
        self.emit_labels(t, finals);
        out.mode = Some(to);
        out.overlay = Some(Overlay::Manual);
        self.drain_trainer(t);
    }
}
