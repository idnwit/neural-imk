use serde::{Deserialize, Serialize};

use crate::{CaretRel, EditAction, KeyEvent, Lang, StatCategory};

/// Version of the event log schema. Bump on breaking changes.
pub const SCHEMA_VERSION: u32 = 1;

/// One line of the event log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Schema version, [`SCHEMA_VERSION`] when written.
    pub v: u32,
    /// Monotonic sequence number within the session, starting at 0.
    pub seq: u64,
    /// Seconds since boot, taken from the triggering input event. Not unique
    /// and resets on reboot; order by `(session, seq)`.
    pub t: f64,
    /// Random id of the IME process run that wrote the record.
    pub session: String,
    /// Payload.
    #[serde(flatten)]
    pub event: Event,
}

/// Event log payload, serialized with a `type` tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// First record of every session; everything replay needs to rebuild
    /// the engine as it was.
    SessionStart {
        /// Wall clock at start, Unix milliseconds (supplied by the host).
        wall_unix_ms: u64,
        /// IME build version.
        app_version: String,
        /// Serialized `EngineConfig` (JSON), opaque to this crate.
        config_json: String,
        /// Policy serving predictions at start.
        policy_id: String,
        /// Local time offset from UTC at start, minutes (host-supplied).
        #[serde(default)]
        utc_offset_min: i32,
    },
    /// App focus or caret context changed.
    Context {
        /// Bundle id of the focused app.
        bundle_id: String,
        /// Focused text input client id.
        field_id: u64,
        /// Caret position, if known.
        caret: Option<CaretRel>,
    },
    /// A key reached the IME.
    Key {
        /// The key.
        key: KeyEvent,
        /// Word the key belongs to, if any.
        word_id: Option<u64>,
    },
    /// The policy (or a hard rule) was consulted.
    Decision {
        /// What is being decided.
        target: Target,
        /// Number of keys in the word when decided (0 for a focus prior).
        at_key: u16,
        /// Policy id, e.g. `tiny_mlp@v1`.
        policy_id: String,
        /// Model output; `None` when a hard rule decided.
        p_vi: Option<f32>,
        /// Hard rule that decided, if any.
        rule: Option<DecisionRule>,
        /// Mode chosen.
        chosen: Lang,
        /// Buffer edit applied (ADR 0007); `None` in old logs and for focus
        /// priors.
        #[serde(default)]
        edit: Option<EditAction>,
    },
    /// The user pressed the switch hotkey.
    ManualSwitch {
        /// Where in the word cycle the switch happened.
        scope: SwitchScope,
        /// Current or most recent word.
        word_id: Option<u64>,
        /// Mode after the switch.
        to: Lang,
    },
    /// A word was committed to the app.
    WordCommit {
        /// Word id.
        word_id: u64,
        /// Raw keys.
        raw: String,
        /// Telex rendering.
        vi: String,
        /// Pass-through rendering.
        en: String,
        /// Mode used for the committed text.
        mode: Lang,
        /// What ended the word.
        boundary: Boundary,
    },
    /// A label was finalized for a word (or for the focus prior).
    Label {
        /// What the label is for.
        target: Target,
        /// Final label.
        label: Lang,
        /// Training weight.
        weight: f32,
        /// How the label was obtained.
        source: LabelSource,
    },
    /// The trainer updated the model.
    ModelUpdate {
        /// Policy id after the update.
        policy_id: String,
        /// Samples in the update.
        n_samples: u32,
        /// Mean weighted log-loss of the batch before the update.
        loss: f32,
    },
    /// The user switched the action model (additive; replay ignores it).
    ModelSwitch {
        /// Policy id before the switch (e.g. `linear_action@v3`).
        from: String,
        /// Policy id after the switch.
        to: String,
    },
    /// A model's quality counters for one counted word (additive; replay
    /// ignores it). One record per model per word.
    ModelScore {
        /// Model kind name, e.g. `gru_action`.
        kind: String,
        /// Word the score belongs to.
        word_id: u64,
        /// App category of the word.
        category: StatCategory,
        /// The model's rendering equalled the ground truth.
        correct: bool,
        /// Keystrokes of the word whose display decision equalled the truth.
        display_hits: u16,
        /// Keystrokes of the word with a display decision.
        display_total: u16,
        /// Display changes between consecutive keystrokes of the word.
        flips: u16,
        /// The record amends an earlier score of the same word.
        amend: bool,
        /// The model was the active one (its output reached the user).
        #[serde(default)]
        served: bool,
    },
    /// The user reset a model to its bundled default; its quality counters
    /// restart from zero (additive; replay ignores it).
    ModelReset {
        /// Model kind name, e.g. `linear_action`.
        kind: String,
    },
}

/// Subject of a decision or label. Ids are per session, assigned by the
/// engine from mode-independent rules so replay ids line up with the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Target {
    /// A word, by word id.
    Word(u64),
    /// The mode prior chosen on a focus change, by focus id (one per
    /// `Context` event that changed app or field).
    Focus(u64),
}

/// Hard rule that decided without consulting the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionRule {
    /// Telex output is not a valid Vietnamese syllable, so English.
    InvalidVi,
    /// Raw keys contain no Telex transform, so both renderings are equal.
    NoTelex,
}

/// When a manual switch happened relative to typing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchScope {
    /// While a word was being composed.
    InWord,
    /// Shortly after a word was committed.
    AfterCommit,
    /// After focus, before any word was typed.
    BeforeTyping,
}

/// What ended a word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Boundary {
    /// Space.
    Space,
    /// Return.
    Return,
    /// Tab.
    Tab,
    /// Punctuation or other non-letter character.
    Punct,
    /// Arrow key or other navigation.
    Navigation,
    /// App shortcut (Cmd/Ctrl/Option).
    Shortcut,
    /// IMK asked to commit (focus loss, mouse click).
    External,
}

/// Origin of a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelSource {
    /// No correction within the window, so the chosen mode is assumed right.
    Implicit,
    /// The user corrected with the hotkey.
    Correction,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn old_session_start_parses_with_zero_offset_and_model_score_round_trips() {
        let old = r#"{"v":1,"seq":0,"t":1.0,"session":"s","type":"session_start","wall_unix_ms":5,"app_version":"a","config_json":"{}","policy_id":"p"}"#;
        let r: Record = serde_json::from_str(old).unwrap();
        assert!(matches!(
            r.event,
            Event::SessionStart {
                utc_offset_min: 0,
                ..
            }
        ));
        let ms = Event::ModelScore {
            kind: "gru_action".into(),
            word_id: 7,
            category: StatCategory::Terminal,
            correct: true,
            display_hits: 3,
            display_total: 4,
            flips: 1,
            amend: false,
            served: true,
        };
        let json = serde_json::to_string(&ms).unwrap();
        assert!(
            json.contains(r#""type":"model_score""#) && json.contains(r#""category":"terminal""#)
        );
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), ms);
        let reset = Event::ModelReset {
            kind: "linear_action".into(),
        };
        let json = serde_json::to_string(&reset).unwrap();
        assert_eq!(json, r#"{"type":"model_reset","kind":"linear_action"}"#);
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), reset);
    }
}
