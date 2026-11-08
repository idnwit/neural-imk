//! Per-key flow of [`Engine`]: observation, model call, mechanical
//! application of the action, commits, labels and trainer outputs.

use nimk_policy::decide;
use nimk_types::{
    Action, ActionMask, Boundary, EditAction, Event, Example, KeyEvent, KeyKind, Lang, Observation,
    RECENT_WINDOW, Target,
};

use super::{Engine, EngineOutput, LastWord, Overlay, TelexFlags};
use crate::composer::{boundary_of, printable_char};
use crate::labeler::{FinalLabel, Step};
use crate::trainer::TrainerOut;

/// Most model calls kept per word for training (a pathological word stops
/// recording instead of growing without bound).
const MAX_STEPS: usize = 128;

/// Most boundary characters remembered after a committed word.
const MAX_BOUNDARY: usize = 16;

/// Teacher-forced action for [`Engine::teach`].
#[derive(Debug, Clone, Copy)]
pub(super) struct Forced {
    pub(super) target: Action,
    pub(super) edit_weight: f32,
    pub(super) render_weight: f32,
}

impl Engine {
    /// `on_key` / `teach` body; may only panic through a bug in a dependency.
    pub(super) fn handle_key(&mut self, key: &KeyEvent, forced: Option<Forced>) -> EngineOutput {
        let t = key.timestamp;
        let mut out = EngineOutput::default();
        self.tick(t);
        let mask = self.mask_for(key);
        let word_id = (!self.composer.is_empty() || mask.append).then(|| self.composer.word_id());
        self.logger.log(
            t,
            Event::Key {
                key: key.clone(),
                word_id,
            },
        );

        // Hotkeys are detected even with `hotkeys = false` so they are
        // swallowed the same way and word ids stay aligned with the log;
        // they only switch modes when hotkeys are enabled. They never reach
        // the model or the recent window.
        if self.hotkey.feed(key).is_some() {
            if self.cfg.hotkeys {
                self.manual_switch(t, &mut out);
            }
            out.consumed = true;
            return out;
        }
        // Bare modifier changes (including the taps of an unfinished double
        // Shift) are not typing: no model call, no effect on the buffer.
        if key.kind == KeyKind::FlagsChanged {
            return out;
        }
        self.model_step(key, mask, t, forced, &mut out);
        out
    }

    /// Structural legality of each edit for `key` (not a language rule).
    fn mask_for(&self, key: &KeyEvent) -> ActionMask {
        let filled = !self.composer.is_empty();
        ActionMask {
            append: printable_char(key).is_some() && !self.composer.is_full(),
            pop: filled,
            commit: filled,
        }
    }

    fn secs_since_switch(&self, t: f64) -> f32 {
        self.last_switch_t
            .map_or(f32::INFINITY, |s| (t - s).max(0.0) as f32)
    }

    fn context(&self) -> (String, Option<nimk_types::CaretRel>) {
        match &self.focus {
            Some(f) => (f.ctx.bundle_id.clone(), f.ctx.caret),
            None => (String::new(), None),
        }
    }

    /// Builds the model input for `key` (`None` = focus prior).
    fn observation(&self, key: Option<&KeyEvent>, mask: ActionMask, t: f64) -> Observation {
        let (bundle_id, caret) = self.context();
        Observation {
            key: key.cloned(),
            recent: self.recent.iter().cloned().collect(),
            raw: self.composer.raw().to_owned(),
            vi: self.composer.rendering(Lang::Vi).to_owned(),
            valid_vi: self.telex.valid_vi,
            viable_vi: self.telex.viable_vi,
            shown: self.shown,
            mode: self.mode,
            prev_langs: self.prev_langs.clone(),
            bundle_id,
            caret,
            secs_since_switch: self.secs_since_switch(t),
            mask,
        }
    }

    /// Hysteresis: leave `base` only if `|p - 0.5| > margin`. Returns the
    /// sanitized probability for the log (`None` if not finite) and the
    /// chosen rendering.
    fn hysteresis(&self, base: Lang, p: f32) -> (Option<f32>, Lang) {
        if !p.is_finite() {
            return (None, base);
        }
        let p = p.clamp(0.0, 1.0);
        let chosen = if (p - 0.5).abs() > self.cfg.switch_margin {
            Lang::from_p_vi(p)
        } else {
            base
        };
        (Some(p), chosen)
    }

    /// Recomputes the Telex rendering and flags of the buffer.
    fn refresh_telex(&mut self) {
        let tr = nimk_telex::transform(self.composer.raw());
        self.composer.set_vi(&tr.text);
        self.telex = TelexFlags {
            valid_vi: tr.valid_vi,
            viable_vi: tr.viable_vi,
        };
    }

    /// Back to "no word": the next one starts in the sticky mode.
    fn reset_word(&mut self) {
        self.locked = false;
        self.shown = self.mode;
        self.telex = TelexFlags::default();
        self.word_steps.clear();
    }

    /// One model call for one non-hotkey key-down, and its application.
    fn model_step(
        &mut self,
        key: &KeyEvent,
        mask: ActionMask,
        t: f64,
        forced: Option<Forced>,
        out: &mut EngineOutput,
    ) {
        let obs = self.observation(Some(key), mask, t);
        let (action, p_vi) = match forced {
            Some(f) => {
                let edit = if mask.allows(f.target.edit) {
                    f.target.edit
                } else {
                    EditAction::Pass
                };
                (
                    Action {
                        edit,
                        render: f.target.render,
                    },
                    None,
                )
            }
            None => {
                let scores = self.model.score(&obs);
                let edit = decide(&scores, mask).edit;
                let base = self.shown;
                let (p, held) = self.hysteresis(base, scores.p_vi);
                let render = if self.locked {
                    // A manual switch locks the rendering for the rest of the word.
                    base
                } else if edit == EditAction::Commit {
                    // Hysteresis only steadies the display mid-word; the
                    // committed text is the model's decision at the commit
                    // step itself.
                    p.map_or(base, Lang::from_p_vi)
                } else {
                    held
                };
                (Action { edit, render }, p)
            }
        };

        let before = self.composer.key_count();
        let was_empty = self.composer.is_empty();
        let at_key = match action.edit {
            EditAction::Append => before.saturating_add(1),
            EditAction::Pop => before.saturating_sub(1),
            EditAction::Commit | EditAction::Pass => before,
        };
        self.logger.log(
            t,
            Event::Decision {
                target: Target::Word(self.composer.word_id()),
                at_key,
                policy_id: self.model_id.to_string(),
                p_vi,
                rule: None,
                chosen: action.render,
                edit: Some(action.edit),
            },
        );
        match forced {
            Some(f) => {
                self.taught.push(Example {
                    obs,
                    target: action,
                    edit_weight: f.edit_weight,
                    render_weight: f.render_weight,
                });
                if self.taught.len() >= self.cfg.teach_batch.max(1) {
                    self.flush_taught();
                }
            }
            None => {
                if (!was_empty || action.edit == EditAction::Append)
                    && self.word_steps.len() < MAX_STEPS
                {
                    self.word_steps.push(Step {
                        obs,
                        edit: action.edit,
                    });
                }
            }
        }

        let prev_shown = self.shown;
        self.shown = action.render;
        match action.edit {
            EditAction::Append => {
                if let Some(c) = printable_char(key) {
                    self.composer.push(c);
                    self.refresh_telex();
                }
                self.last_word = None;
                out.consumed = true;
                out.marked = Some(self.composer.rendering(self.shown).to_owned());
            }
            EditAction::Pop => {
                self.composer.pop();
                out.consumed = true;
                if self.composer.is_empty() {
                    self.reset_word();
                    out.marked = Some(String::new());
                } else {
                    self.refresh_telex();
                    out.marked = Some(self.composer.rendering(self.shown).to_owned());
                }
            }
            EditAction::Commit => {
                let committed = self.commit_word(boundary_of(key), t, out);
                self.last_word = match (committed, printable_char(key)) {
                    (Some(mut w), Some(c)) => {
                        w.boundary.push(c);
                        Some(w)
                    }
                    _ => None,
                };
            }
            EditAction::Pass => {
                if was_empty {
                    self.note_passed_char(key);
                } else if prev_shown != self.shown {
                    out.marked = Some(self.composer.rendering(self.shown).to_owned());
                }
            }
        }
        if self.composer.is_empty() {
            self.shown = self.mode;
        }
        self.recent.push_back(key.clone());
        if self.recent.len() > RECENT_WINDOW {
            self.recent.pop_front();
        }
    }

    /// A key reached the app with no word in progress: a printable one
    /// extends the boundary after the last committed word, anything else
    /// (letters of a new word, Backspace, arrows, shortcuts) forgets it.
    fn note_passed_char(&mut self, key: &KeyEvent) {
        match (self.last_word.as_mut(), printable_char(key)) {
            (Some(w), Some(c)) if w.boundary.chars().count() < MAX_BOUNDARY => w.boundary.push(c),
            _ => self.last_word = None,
        }
    }

    /// Housekeeping on every input event: event clock, implicit label
    /// timeouts and periodic snapshots.
    pub(super) fn tick(&mut self, t: f64) {
        self.last_t = t;
        let mut finals = Vec::new();
        self.labeler.tick(t, &mut finals);
        self.emit_labels(t, finals);
        match self.last_snapshot_t {
            Some(last) if t < last => self.last_snapshot_t = Some(t),
            Some(last) if t - last >= self.cfg.snapshot_every_secs => {
                self.trainer.request_save();
                self.last_snapshot_t = Some(t);
            }
            Some(_) => {}
            None => self.last_snapshot_t = Some(t),
        }
    }

    /// Prior decision on a focus change: the model sees an observation
    /// without a key and a mask that only allows `Pass`; its render choice
    /// sets the sticky mode.
    pub(super) fn decide_prior(&mut self, focus_id: u64, t: f64, out: &mut EngineOutput) {
        self.shown = self.mode;
        let obs = self.observation(None, ActionMask::default(), t);
        let scores = self.model.score(&obs);
        let (p_vi, render) = self.hysteresis(self.mode, scores.p_vi);
        let target = Target::Focus(focus_id);
        self.logger.log(
            t,
            Event::Decision {
                target,
                at_key: 0,
                policy_id: self.model_id.to_string(),
                p_vi,
                rule: None,
                chosen: render,
                edit: None,
            },
        );
        self.set_sticky(render, out);
        self.shown = self.mode;
        let step = Step {
            obs,
            edit: EditAction::Pass,
        };
        self.labeler.on_focus(target, vec![step], render, t);
    }

    /// Moves the sticky mode on a model decision (overlay `Auto`).
    fn set_sticky(&mut self, lang: Lang, out: &mut EngineOutput) {
        if lang != self.mode {
            self.mode = lang;
            out.mode = Some(lang);
            out.overlay = Some(Overlay::Auto);
        }
    }

    /// Commits the current word (if any) in its shown rendering (for a model
    /// `Commit`, `shown` was just set to the model's fresh decision) and returns
    /// what the retroactive correction needs to remember about it.
    pub(super) fn commit_word(
        &mut self,
        boundary: Boundary,
        t: f64,
        out: &mut EngineOutput,
    ) -> Option<LastWord> {
        let lang = self.shown;
        let text = self.composer.rendering(lang).to_owned();
        let w = self.composer.take()?;
        self.logger.log(
            t,
            Event::WordCommit {
                word_id: w.id,
                raw: w.raw.clone(),
                vi: w.vi.clone(),
                en: w.raw.clone(),
                mode: lang,
                boundary,
            },
        );
        out.commit = Some(text);
        let correction = self.locked.then_some(lang);
        let steps = std::mem::take(&mut self.word_steps);
        let mut finals = Vec::new();
        self.labeler
            .on_commit(Target::Word(w.id), steps, lang, t, correction, &mut finals);
        self.emit_labels(t, finals);
        self.prev_langs.insert(0, lang);
        self.prev_langs.truncate(3);
        self.committed_since_focus = true;
        self.set_sticky(lang, out);
        self.reset_word();
        self.last_word = None;
        self.drain_trainer(t);
        Some(LastWord {
            id: w.id,
            raw: w.raw,
            vi: w.vi,
            lang,
            boundary: String::new(),
        })
    }

    /// Hands the queued teacher-forced examples to the trainer as one batch.
    pub(super) fn flush_taught(&mut self) {
        if !self.taught.is_empty() {
            let batch = std::mem::take(&mut self.taught);
            self.trainer.submit(batch, false);
        }
    }

    /// Logs final labels and hands their examples to the trainer.
    pub(super) fn emit_labels(&mut self, t: f64, finals: Vec<FinalLabel>) {
        for f in finals {
            self.logger.log(
                t,
                Event::Label {
                    target: f.target,
                    label: f.label,
                    weight: f.weight,
                    source: f.source,
                },
            );
            // Re-train around a correction only when it contradicts what
            // was rendered; a confirming label is an ordinary update.
            self.trainer.submit(f.examples, f.corrected);
        }
    }

    /// Picks up trainer outputs (word boundaries only, never mid-word).
    pub(super) fn drain_trainer(&mut self, t: f64) {
        let mut outs = Vec::new();
        self.trainer.drain(&mut outs);
        for o in outs {
            match o {
                TrainerOut::Update {
                    policy_id,
                    n_samples,
                    loss,
                } => {
                    self.logger.log(
                        t,
                        Event::ModelUpdate {
                            policy_id,
                            n_samples,
                            loss,
                        },
                    );
                }
                TrainerOut::Snapshot(model) => {
                    self.model_id = std::sync::Arc::from(model.id());
                    self.model = model;
                }
                TrainerOut::Diagnostic(d) => self.diagnostics.push(d),
            }
        }
    }
}
