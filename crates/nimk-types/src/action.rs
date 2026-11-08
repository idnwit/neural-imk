//! Neural action contract (ADR 0007): event + buffer in, action out.

use serde::{Deserialize, Serialize};

use crate::{CaretRel, KeyEvent, Lang};

/// Maximum number of past events in [`Observation::recent`].
pub const RECENT_WINDOW: usize = 16;

/// What happens to the buffer and the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditAction {
    /// Append the key's character to the raw buffer; key consumed.
    Append,
    /// Remove the last raw key; key consumed.
    Pop,
    /// Commit the shown rendering, then pass the key to the app.
    Commit,
    /// Pass the key to the app; buffer unchanged.
    Pass,
}

impl EditAction {
    /// All actions in head order (index = position in model output).
    pub const ALL: [Self; 4] = [Self::Append, Self::Pop, Self::Commit, Self::Pass];

    /// Index in [`EditAction::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Append => 0,
            Self::Pop => 1,
            Self::Commit => 2,
            Self::Pass => 3,
        }
    }
}

/// One decision of the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    /// Buffer edit.
    pub edit: EditAction,
    /// Rendering to show (and to commit on `Commit`).
    pub render: Lang,
}

/// Structurally legal edits for the current event. Not a language rule: it
/// only removes physically meaningless actions. `Pass` is always legal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ActionMask {
    /// The key produces a printable character that can join a word.
    pub append: bool,
    /// The buffer is non-empty.
    pub pop: bool,
    /// The buffer is non-empty.
    pub commit: bool,
}

impl ActionMask {
    /// Whether `edit` is legal.
    #[must_use]
    pub const fn allows(self, edit: EditAction) -> bool {
        match edit {
            EditAction::Append => self.append,
            EditAction::Pop => self.pop,
            EditAction::Commit => self.commit,
            EditAction::Pass => true,
        }
    }
}

/// Everything the model sees for one event. Owned so it can be stored as a
/// training example and logged for replay.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Observation {
    /// The incoming event; `None` for the mode prior on a focus change.
    /// Never a hotkey (hotkeys belong to the I/O layer).
    pub key: Option<KeyEvent>,
    /// Up to [`RECENT_WINDOW`] previous non-hotkey events, oldest first.
    pub recent: Vec<KeyEvent>,
    /// Raw keys of the current word before this event.
    pub raw: String,
    /// Telex rendering of `raw` (tool output, a feature).
    pub vi: String,
    /// `vi` is a complete valid Vietnamese syllable (feature).
    pub valid_vi: bool,
    /// `vi` is a valid syllable or a prefix of one (feature).
    pub viable_vi: bool,
    /// Rendering currently shown for the buffer.
    pub shown: Lang,
    /// Sticky mode the next word starts in.
    pub mode: Lang,
    /// Languages of up to 3 previous words in this field, most recent first.
    pub prev_langs: Vec<Lang>,
    /// Bundle id of the focused app.
    pub bundle_id: String,
    /// Caret position, if known.
    pub caret: Option<CaretRel>,
    /// Seconds since the last manual switch (`f32::INFINITY` if none).
    pub secs_since_switch: f32,
    /// Legal edits for this event.
    pub mask: ActionMask,
}

/// Model output for one observation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ActionScores {
    /// Probabilities over [`EditAction::ALL`] (before masking; sum to 1).
    pub edit: [f32; 4],
    /// Probability the user wants the Vietnamese rendering.
    pub p_vi: f32,
}

/// One supervised example for both heads.
#[derive(Debug, Clone, PartialEq)]
pub struct Example {
    /// Model input.
    pub obs: Observation,
    /// Target action.
    pub target: Action,
    /// Weight of the edit-head loss (0 disables it for this example).
    pub edit_weight: f32,
    /// Weight of the render-head loss (0 disables it for this example).
    pub render_weight: f32,
}
