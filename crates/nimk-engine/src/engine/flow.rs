//! Per-key flow of [`Engine`]: hotkeys, composition, decisions, commits,
//! manual switches, labels and trainer outputs.

use nimk_types::{
    Boundary, DecisionCtx, DecisionRule, Event, KeyEvent, KeyKind, LabelSource, Lang, SwitchScope,
    Target,
};

use super::{Engine, EngineOutput, Overlay, WordTelex};
use crate::composer::{KeyClass, classify};
use crate::labeler::FinalLabel;
use crate::trainer::TrainerOut;

/// Outcome of one decision.
struct Decided {
    p_vi: Option<f32>,
    rule: Option<DecisionRule>,
    chosen: Lang,
}

impl Engine {
    /// `on_key` body; may only panic through a bug in a dependency.
    pub(super) fn handle_key(&mut self, key: &KeyEvent) -> EngineOutput {
        let t = key.timestamp;
        let mut out = EngineOutput::default();
        self.tick(t);
        let class = classify(key);
        let word_id = (!self.composer.is_empty() || matches!(class, KeyClass::Letter(_)))
            .then(|| self.composer.word_id());
        self.logger.log(
            t,
            Event::Key {
                key: key.clone(),
                word_id,
            },
        );

        // Hotkeys are detected even with `hotkeys = false` so they are
        // swallowed the same way and word ids stay aligned with the log;
        // they only switch modes when hotkeys are enabled.
        // A completed hotkey (Shift-tap release or Ctrl+Space) is consumed;
        // other modifier changes return `consumed = false`.
        if self.hotkey.feed(key).is_some() {
            if self.cfg.hotkeys {
                self.manual_switch(t, &mut out);
            }
            out.consumed = true;
            return out;
        }
        if key.kind == KeyKind::FlagsChanged {
            return out;
        }
        match class {
            KeyClass::Ignore => {}
            KeyClass::Letter(_) if self.composer.is_full() => {
                // Overlong word: split it; the letter goes to the app.
                self.commit_word(Boundary::Punct, t, &mut out);
            }
            KeyClass::Letter(c) => {
                self.composer.push(c);
                self.decide_word(t, &mut out);
                out.marked = Some(self.composer.rendering(self.word_lang()).to_owned());
                out.consumed = true;
            }
            KeyClass::Backspace => {
                if self.composer.pop() {
                    if self.composer.is_empty() {
                        self.in_word_correction = false;
                        self.word_mode = None;
                        out.marked = Some(String::new());
                    } else {
                        self.decide_word(t, &mut out);
                        out.marked = Some(self.composer.rendering(self.word_lang()).to_owned());
                    }
                    out.consumed = true;
                }
            }
            KeyClass::Boundary(b) => self.commit_word(b, t, &mut out),
        }
        out
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

    /// Rendering language of the current word: the word-scoped hard-rule
    /// override if any, else the sticky mode.
    fn word_lang(&self) -> Lang {
        match self.word_mode {
            Some(lang) => lang,
            None => self.mode,
        }
    }

    fn secs_since_switch(&self, t: f64) -> f32 {
        self.last_switch_t
            .map_or(f32::INFINITY, |s| (t - s).max(0.0) as f32)
    }

    /// Hysteresis: leave the current mode only if `|p - 0.5| > margin`.
    fn hysteresis(&self, p: f32) -> (Option<f32>, Lang) {
        if !p.is_finite() {
            return (None, self.mode);
        }
        let p = p.clamp(0.0, 1.0);
        let chosen = if (p - 0.5).abs() > self.cfg.switch_margin {
            Lang::from_p_vi(p)
        } else {
            self.mode
        };
        (Some(p), chosen)
    }

    /// Re-renders the current word and decides its mode.
    fn decide_word(&mut self, t: f64, out: &mut EngineOutput) {
        let tr = nimk_telex::transform(self.composer.raw());
        self.composer.set_vi(&tr.text);
        self.word_telex = WordTelex {
            has_telex: tr.has_telex,
            valid_vi: tr.valid_vi,
        };
        let (bundle_id, caret) = match &self.focus {
            Some(f) => (f.ctx.bundle_id.as_str(), f.ctx.caret),
            None => ("", None),
        };
        let ctx = DecisionCtx {
            bundle_id,
            caret,
            raw: self.composer.raw(),
            valid_vi: tr.valid_vi,
            has_telex: tr.has_telex,
            prev_langs: &self.prev_langs,
            current: self.mode,
            secs_since_switch: self.secs_since_switch(t),
        };
        self.extractor.extract(&ctx, &mut self.word_features);
        if self.in_word_correction {
            // The user chose this word's mode; no automatic decision.
            return;
        }
        let d = if !tr.has_telex {
            Decided {
                p_vi: None,
                rule: Some(DecisionRule::NoTelex),
                chosen: self.mode,
            }
        } else if !tr.viable_vi {
            // Mid-word: EN only once no completion can be valid Vietnamese.
            Decided {
                p_vi: None,
                rule: Some(DecisionRule::InvalidVi),
                chosen: Lang::En,
            }
        } else {
            let (p_vi, mut chosen) = self.hysteresis(self.policy.predict(&self.word_features));
            if self.manual_hold {
                chosen = self.mode;
            }
            Decided {
                p_vi,
                rule: None,
                chosen,
            }
        };
        let target = Target::Word(self.composer.word_id());
        self.apply_decision(target, self.composer.key_count(), d, t, out);
    }

    /// Prior decision on a focus change (`raw = ""`, no hard rules).
    pub(super) fn decide_prior(&mut self, focus_id: u64, t: f64, out: &mut EngineOutput) {
        let (bundle_id, caret) = match &self.focus {
            Some(f) => (f.ctx.bundle_id.as_str(), f.ctx.caret),
            None => ("", None),
        };
        let ctx = DecisionCtx {
            bundle_id,
            caret,
            raw: "",
            valid_vi: false,
            has_telex: false,
            prev_langs: &[],
            current: self.mode,
            secs_since_switch: self.secs_since_switch(t),
        };
        let mut features = nimk_types::Features::default();
        self.extractor.extract(&ctx, &mut features);
        let (p_vi, chosen) = self.hysteresis(self.policy.predict(&features));
        let target = Target::Focus(focus_id);
        self.apply_decision(
            target,
            0,
            Decided {
                p_vi,
                rule: None,
                chosen,
            },
            t,
            out,
        );
        self.labeler.on_focus(target, features, chosen, t);
    }

    fn apply_decision(
        &mut self,
        target: Target,
        at_key: u16,
        d: Decided,
        t: f64,
        out: &mut EngineOutput,
    ) {
        self.logger.log(
            t,
            Event::Decision {
                target,
                at_key,
                policy_id: self.policy_id.to_string(),
                p_vi: d.p_vi,
                rule: d.rule,
                chosen: d.chosen,
            },
        );
        if d.rule.is_some() {
            // Hard rules are word-scoped: they pick this word's rendering
            // but never move the sticky mode, and show no overlay.
            self.word_mode = (d.chosen != self.mode).then_some(d.chosen);
            return;
        }
        self.word_mode = None;
        if d.chosen != self.mode {
            self.mode = d.chosen;
            out.mode = Some(d.chosen);
            out.overlay = Some(Overlay::Auto);
        }
    }

    /// Commits the current word (if any) in its rendering language.
    ///
    /// Runs the final hard rule first: a word whose Telex rendering is not
    /// a complete valid syllable commits in EN (logged as an `InvalidVi`
    /// decision); otherwise the rendering is kept and nothing is logged.
    /// Either way the sticky mode is unchanged and the next word starts in it.
    pub(super) fn commit_word(&mut self, boundary: Boundary, t: f64, out: &mut EngineOutput) {
        let wt = self.word_telex;
        if !self.composer.is_empty() && !self.in_word_correction && wt.has_telex && !wt.valid_vi {
            let d = Decided {
                p_vi: None,
                rule: Some(DecisionRule::InvalidVi),
                chosen: Lang::En,
            };
            let target = Target::Word(self.composer.word_id());
            self.apply_decision(target, self.composer.key_count(), d, t, out);
        }
        self.word_telex = WordTelex::default();
        let mode = self.word_lang();
        self.word_mode = None;
        let text = self.composer.rendering(mode).to_owned();
        let Some(w) = self.composer.take() else {
            return;
        };
        self.logger.log(
            t,
            Event::WordCommit {
                word_id: w.id,
                raw: w.raw.clone(),
                vi: w.vi,
                en: w.raw,
                mode,
                boundary,
            },
        );
        out.commit = Some(text);
        let correction = self.in_word_correction.then_some(mode);
        let features = std::mem::take(&mut self.word_features);
        let mut finals = Vec::new();
        self.labeler.on_commit(
            Target::Word(w.id),
            features,
            mode,
            t,
            correction,
            &mut finals,
        );
        self.emit_labels(t, finals);
        self.prev_langs.insert(0, mode);
        self.prev_langs.truncate(3);
        self.committed_since_focus = true;
        self.in_word_correction = false;
        self.manual_hold = false;
        self.drain_trainer(t);
    }

    /// Hotkey: flips the rendering of the current word (or the sticky mode
    /// between words), makes the result the sticky mode, and turns the
    /// context into labels.
    fn manual_switch(&mut self, t: f64, out: &mut EngineOutput) {
        let to = self.word_lang().flip();
        self.mode = to;
        self.word_mode = None;
        self.last_switch_t = Some(t);
        let (scope, word_id) = if !self.composer.is_empty() {
            (SwitchScope::InWord, Some(self.composer.word_id()))
        } else if self.committed_since_focus {
            (SwitchScope::AfterCommit, self.composer.last_committed())
        } else {
            (SwitchScope::BeforeTyping, None)
        };
        self.logger
            .log(t, Event::ManualSwitch { scope, word_id, to });
        let mut finals = Vec::new();
        let mut correct = |target: Target, weight: f32| {
            if let Some(f) = self
                .labeler
                .finalize(target, to, weight, LabelSource::Correction)
            {
                finals.push(f);
            }
        };
        match scope {
            SwitchScope::InWord => {
                self.in_word_correction = true;
                out.marked = Some(self.composer.rendering(to).to_owned());
            }
            SwitchScope::AfterCommit => {
                self.manual_hold = true;
                if let Some(last) = word_id {
                    correct(Target::Word(last), 1.0);
                    if self.cfg.correction_words >= 2
                        && let Some(prev) = last.checked_sub(1)
                    {
                        correct(Target::Word(prev), self.cfg.correction_prev_weight);
                    }
                }
            }
            SwitchScope::BeforeTyping => {
                self.manual_hold = true;
                if let Some(f) = &self.focus {
                    correct(Target::Focus(f.id), 1.0);
                }
            }
        }
        self.emit_labels(t, finals);
        out.mode = Some(to);
        out.overlay = Some(Overlay::Manual);
        self.drain_trainer(t);
    }

    /// Logs final labels and hands their samples to the trainer.
    pub(super) fn emit_labels(&mut self, t: f64, finals: Vec<FinalLabel>) {
        for f in finals {
            self.logger.log(
                t,
                Event::Label {
                    target: f.target,
                    label: f.sample.label,
                    weight: f.sample.weight,
                    source: f.source,
                },
            );
            self.trainer
                .submit(f.sample, f.source == LabelSource::Correction);
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
                TrainerOut::Snapshot(policy) => {
                    self.policy_id = std::sync::Arc::from(policy.id());
                    self.policy = policy;
                }
                TrainerOut::Diagnostic(d) => self.diagnostics.push(d),
            }
        }
    }
}
