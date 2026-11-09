//! Per-key flow of [`Engine`]: observation, model call, mechanical
//! application of the action, commits, labels and trainer outputs.

use nimk_hook::HookMsg;
use nimk_policy::{ModelInput, ModelKind, ModelTransform};
use nimk_types::{
    Action, ActionMask, Boundary, EditAction, Event, Example, KeyEvent, KeyKind, Lang, ModelStats,
    Observation, RECENT_WINDOW, StatCategory, Target, Transform,
};

use super::machine::{Class, Move, State, transition};
use super::{Engine, EngineOutput, META_KEPT, Overlay, PendingWord, TelexFlags, WordMeta};
use crate::composer::{Committed, boundary_of, printable_char};
use crate::labeler::{FinalLabel, Step};
use crate::trainer::{KeyObs, TrainerOut, WordInfo};

/// Name of the hook publishing per-model counters.
const MODEL_STATS: &str = "model_stats";

/// Counters per [`ModelKind::index`] from the `model_stats` summary
/// (`{"<kind>": ModelStats, ...}`); a missing or malformed entry reads as
/// zero counters.
fn parse_model_stats(json: &str) -> [ModelStats; 2] {
    let mut stats = [ModelStats::default(); 2];
    let Ok(serde_json::Value::Object(mut map)) = serde_json::from_str::<serde_json::Value>(json)
    else {
        return stats;
    };
    for kind in ModelKind::ALL {
        if let (Some(v), Some(slot)) = (map.remove(kind.name()), stats.get_mut(kind.index()))
            && let Ok(s) = serde_json::from_value(v)
        {
            *slot = s;
        }
    }
    stats
}

/// Most model calls kept per word for training (a pathological word stops
/// recording instead of growing without bound).
const MAX_STEPS: usize = 128;

/// The mask a step allows: the structural part of the edit (ADR 0012).
fn mask_of(step: Move) -> ActionMask {
    ActionMask {
        append: matches!(step, Move::Append | Move::CommitThenAppend),
        pop: step == Move::Pop,
        commit: matches!(step, Move::EndWordPending | Move::CommitWord),
    }
}

/// The edit a mask leaves no choice about (exactly one structural edit
/// legal), if any.
fn structural_edit(mask: ActionMask) -> Option<EditAction> {
    match (mask.append, mask.pop, mask.commit) {
        (true, false, false) => Some(EditAction::Append),
        (false, true, false) => Some(EditAction::Pop),
        (false, false, true) => Some(EditAction::Commit),
        _ => None,
    }
}

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
        let step = transition(self.state(), self.classify(key));
        let mask = mask_of(step);
        let word_id = (!self.composer.is_empty() || mask.append).then(|| self.composer.word_id());
        self.hooks.log(
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
        match step {
            Move::Pass => {}
            Move::EnterRaw => self.raw = true,
            Move::ExitRaw => self.raw = false,
            Move::PendingBackspace => self.pending_backspace(&mut out),
            Move::CommitThenPass => self.finalize_pending(t, &mut out),
            Move::CommitThenAppend => {
                self.finalize_pending(t, &mut out);
                self.model_step(key, mask, t, forced, &mut out);
                return out;
            }
            Move::Append | Move::Pop | Move::EndWordPending | Move::CommitWord => {
                self.model_step(key, mask, t, forced, &mut out);
                return out;
            }
        }
        // Everything else never consults the model: the key either goes
        // straight to the app or was handled mechanically above.
        self.note_key(key);
        out
    }

    /// Where typing is now (ADR 0012).
    fn state(&self) -> State {
        if self.raw {
            State::Raw
        } else if !self.composer.is_empty() {
            State::Composing
        } else if self.pending.is_some() {
            State::Pending
        } else {
            State::Idle
        }
    }

    /// Whether `key` ends the word being composed or pending: a configured
    /// commit keycode, a Cmd/Ctrl/Opt chord, or text the buffer cannot hold.
    fn commits(&self, key: &KeyEvent) -> bool {
        key.mods.is_shortcut()
            || self.live.commit_keycodes.contains(&key.keycode)
            || !key.chars.is_empty() && !key.chars.chars().all(char::is_control)
    }

    /// Classifies `key` for the transition table.
    fn classify(&self, key: &KeyEvent) -> Class {
        if key.kind != KeyKind::KeyDown {
            return Class::Ignored;
        }
        let printable = printable_char(key);
        if self.raw {
            let ends = key.mods.is_shortcut()
                || self.live.command_end_keycodes.contains(&key.keycode)
                || printable.is_some_and(|c| self.live.command_end_chars.contains(c));
            return if ends {
                Class::Terminator
            } else {
                Class::Ignored
            };
        }
        match printable {
            Some(c) if self.live.is_letter(c) => {
                if self.composer.is_full() {
                    Class::LetterFull
                } else {
                    Class::Letter
                }
            }
            Some(c) if self.live.is_trigger(c) => Class::Trigger,
            Some(_) => Class::Boundary,
            None if key.keycode == nimk_types::keycode::BACKSPACE && !key.mods.is_shortcut() => {
                Class::Backspace
            }
            None if self.commits(key) => Class::Commit,
            None => Class::Ignored,
        }
    }

    /// Backspace over the pending text: drops the boundary character and the
    /// word becomes the composing word again
    /// (so the next Backspace erases a letter).
    fn pending_backspace(&mut self, out: &mut EngineOutput) {
        out.consumed = true;
        let Some(p) = self.pending.as_mut() else {
            return;
        };
        p.boundary.pop();
        if !p.boundary.is_empty() {
            out.marked = Some(p.text());
            return;
        }
        let Some(p) = self.pending.take() else {
            return;
        };
        let word = Committed {
            id: p.id,
            raw: p.raw,
            vi: p.vi,
        };
        if !self.composer.restore(&word) {
            // Unreachable by construction (the pending word is always the
            // latest taken); keep the text rather than lose it.
            out.commit = Some(match p.lang {
                Lang::Vi => word.vi,
                Lang::En => word.raw,
            });
            out.marked = Some(String::new());
            return;
        }
        self.shown = p.lang;
        self.locked = p.locked;
        self.word_steps = p.steps;
        // The step of the undone Commit must not be learned.
        if self
            .word_steps
            .last()
            .is_some_and(|s| s.edit == EditAction::Commit)
        {
            self.word_steps.pop();
        }
        self.refresh_telex();
        out.marked = Some(self.composer.rendering(self.shown).to_owned());
    }

    /// Records a key in the recent window (every non-hotkey key-down).
    fn note_key(&mut self, key: &KeyEvent) {
        self.recent.push_back(key.clone());
        if self.recent.len() > RECENT_WINDOW {
            self.recent.pop_front();
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

    /// Recomputes the Telex rendering and flags of the buffer.
    fn refresh_telex(&mut self) {
        let st = nimk_telex::TelexTransform.apply(self.composer.raw());
        self.composer.set_vi(&st.vi);
        self.telex = TelexFlags {
            valid_vi: st.valid_vi,
            viable_vi: st.viable_vi,
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
        let structural = structural_edit(mask);
        let (action, p_vi) = match forced {
            Some(f) => (
                Action {
                    edit: match structural {
                        Some(edit) => edit,
                        None => EditAction::Pass,
                    },
                    render: f.target.render,
                },
                None,
            ),
            None => {
                let out = ModelTransform::new(&*self.model).apply(&ModelInput {
                    obs: &obs,
                    base: self.shown,
                    locked: self.locked,
                    margin: self.live.switch_margin,
                    structural,
                    prior: false,
                });
                (out.action, out.p_vi)
            }
        };

        let before = self.composer.key_count();
        let was_empty = self.composer.is_empty();
        let at_key = match action.edit {
            EditAction::Append => before.saturating_add(1),
            EditAction::Pop => before.saturating_sub(1),
            EditAction::Commit | EditAction::Pass => before,
        };
        self.hooks.log(
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
                if !was_empty || action.edit == EditAction::Append {
                    // Statistics only: shadow models score this observation
                    // on the trainer side; the active model's decision is
                    // reused, never recomputed.
                    let shadow = self.shadow_on().then(|| Box::new(obs.clone()));
                    self.trainer.submit_key(KeyObs {
                        word_id: self.composer.word_id(),
                        active_render: action.render,
                        margin: self.live.switch_margin,
                        obs: shadow,
                    });
                    if self.word_steps.len() < MAX_STEPS {
                        self.word_steps.push(Step {
                            obs,
                            edit: action.edit,
                        });
                    }
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
                // A non-letter character keeps the word pending as marked
                // text with that one character; any other ender commits it
                // now and the key passes on.
                self.end_word(boundary_of(key), out);
                match printable_char(key).filter(|c| !self.live.is_letter(*c)) {
                    Some(c) => {
                        if let Some(p) = self.pending.as_mut() {
                            p.boundary.push(c);
                            out.consumed = true;
                            out.marked = Some(p.text());
                        }
                    }
                    None => self.finalize_pending(t, out),
                }
            }
            EditAction::Pass => {
                if !was_empty && prev_shown != self.shown {
                    out.marked = Some(self.composer.rendering(self.shown).to_owned());
                }
            }
        }
        if self.composer.is_empty() {
            self.shown = self.mode;
        }
        self.note_key(key);
    }

    /// Whether a shadow model is measured: another loaded model with online
    /// training on.
    fn shadow_on(&self) -> bool {
        ModelKind::ALL.iter().any(|k| {
            *k != self.cfg.policy && self.loaded[k.index()] && self.cfg.models.online_training(*k)
        })
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
        let scored = ModelTransform::new(&*self.model).apply(&ModelInput {
            obs: &obs,
            base: self.mode,
            locked: false,
            margin: self.live.switch_margin,
            structural: None,
            prior: true,
        });
        let (p_vi, render) = (scored.p_vi, scored.action.render);
        let target = Target::Focus(focus_id);
        self.hooks.log(
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
        // The focus prior only moves the mode and the status item: no overlay
        // (every click would pop one up).
        self.set_sticky(render, out);
        out.overlay = None;
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

    /// Ends the word being composed: it becomes the pending word in its
    /// shown rendering (for a model `Commit`, `shown` was just set to the
    /// model's fresh decision) and the sticky mode follows it. Nothing is
    /// logged or labelled until [`Engine::finalize_pending`]. Returns
    /// whether there was a word.
    pub(super) fn end_word(&mut self, ended_by: Boundary, out: &mut EngineOutput) -> bool {
        let lang = self.shown;
        let Some(w) = self.composer.take() else {
            return false;
        };
        self.pending = Some(PendingWord {
            id: w.id,
            raw: w.raw,
            vi: w.vi,
            lang,
            locked: self.locked,
            ended_by,
            boundary: String::new(),
            steps: std::mem::take(&mut self.word_steps),
        });
        self.set_sticky(lang, out);
        self.reset_word();
        true
    }

    /// Commits the pending word for real: `insertText` of word plus
    /// boundary, `WordCommit` record and label bookkeeping.
    pub(super) fn finalize_pending(&mut self, t: f64, out: &mut EngineOutput) {
        let Some(p) = self.pending.take() else {
            return;
        };
        let text = p.text();
        self.hooks.log(
            t,
            Event::WordCommit {
                word_id: p.id,
                raw: p.raw.clone(),
                vi: p.vi.clone(),
                en: p.raw.clone(),
                mode: p.lang,
                boundary: p.ended_by,
            },
        );
        out.commit = Some(text);
        // Registered before the labels so an immediate correction finds it.
        let category = match &self.focus {
            Some(f) => self
                .cfg
                .category_name(&f.ctx.bundle_id)
                .map_or(StatCategory::Other, StatCategory::from_name),
            None => StatCategory::Other,
        };
        if self.word_meta.len() >= META_KEPT {
            self.word_meta.pop_front();
        }
        self.word_meta.push_back(WordMeta { id: p.id, category });
        let correction = p.locked.then_some(p.lang);
        let mut finals = Vec::new();
        self.labeler.on_commit(
            Target::Word(p.id),
            p.steps,
            p.lang,
            t,
            correction,
            &mut finals,
        );
        self.emit_labels(t, finals);
        self.prev_langs.insert(0, p.lang);
        self.prev_langs.truncate(3);
        self.committed_since_focus = true;
        self.drain_trainer(t);
    }

    /// Ends the word being composed (if any) and commits it, and any
    /// pending word, for real.
    pub(super) fn commit_word(&mut self, boundary: Boundary, t: f64, out: &mut EngineOutput) {
        self.end_word(boundary, out);
        self.finalize_pending(t, out);
    }

    /// Hands the queued teacher-forced examples to the trainer as one batch.
    pub(super) fn flush_taught(&mut self) {
        if !self.taught.is_empty() {
            let batch = std::mem::take(&mut self.taught);
            self.trainer.submit(batch, false, None);
        }
    }

    /// Logs final labels and hands their examples to the trainer.
    pub(super) fn emit_labels(&mut self, t: f64, finals: Vec<FinalLabel>) {
        for f in finals {
            self.hooks.log(
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
            let info = match f.target {
                Target::Word(id) => {
                    self.word_meta
                        .iter()
                        .rev()
                        .find(|m| m.id == id)
                        .map(|m| WordInfo {
                            word_id: id,
                            label: f.label,
                            committed: f.committed,
                            amends: f.amends,
                            category: m.category,
                        })
                }
                Target::Focus(_) => None,
            };
            self.trainer.submit(f.examples, f.corrected, info);
        }
    }

    /// Picks up trainer outputs (word boundaries only, never mid-word).
    /// Scores are logged before the hooks are drained, so in `Inline` mode
    /// the `model_stats` summary is current when this returns.
    pub(super) fn drain_trainer(&mut self, t: f64) {
        let mut outs = Vec::new();
        self.trainer.drain(&mut outs);
        for o in outs {
            match self.log_trainer_out(t, o) {
                Some(TrainerOut::Snapshot(model)) => {
                    self.model_id = std::sync::Arc::from(model.id());
                    self.model = model;
                }
                Some(TrainerOut::Diagnostic(d)) => self.diagnostics.push(d),
                Some(TrainerOut::Update { .. } | TrainerOut::Score { .. }) | None => {}
            }
        }
        self.drain_hooks();
    }

    /// Logs a trainer output that is a record (`Update` as `model_update`,
    /// `Score` as `model_score`) at event time `t`; hands back the others.
    pub(super) fn log_trainer_out(&mut self, t: f64, o: TrainerOut) -> Option<TrainerOut> {
        let event = match o {
            TrainerOut::Update {
                policy_id,
                n_samples,
                loss,
            } => Event::ModelUpdate {
                policy_id,
                n_samples,
                loss,
            },
            TrainerOut::Score {
                kind,
                word_id,
                category,
                correct,
                display_hits,
                display_total,
                flips,
                amend,
                served,
            } => Event::ModelScore {
                kind: kind.name().into(),
                word_id,
                category,
                correct,
                display_hits,
                display_total,
                flips,
                amend,
                served,
            },
            other @ (TrainerOut::Snapshot(_) | TrainerOut::Diagnostic(_)) => return Some(other),
        };
        self.hooks.log(t, event);
        None
    }

    /// Picks up hook messages: caches the latest summary per hook (the
    /// `model_stats` one also parsed into the counters) and queues
    /// diagnostics.
    pub(super) fn drain_hooks(&mut self) {
        for m in self.hooks.drain() {
            match m {
                HookMsg::Summary { hook, json } => {
                    if hook == MODEL_STATS {
                        self.stats = parse_model_stats(&json);
                    }
                    self.summaries.insert(hook, json);
                }
                HookMsg::Diagnostic(d) => self.diagnostics.push(d),
            }
        }
    }
}
