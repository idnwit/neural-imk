//! A run of keys with the target action of every key.

use nimk_types::{Action, KeyEvent, Lang};

/// One word of a script with its label.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptWord {
    /// Word id the engine will assign (0, 1, 2... per session, one per
    /// committed non-empty word).
    pub word_id: u64,
    /// Intended language.
    pub lang: Lang,
    /// Text the user intends to see (`tiếng` or `has`).
    pub text: String,
    /// Raw keys typed for it (`tieengs` or `has`).
    pub raw: String,
}

/// One typing session (synthetic) or one focus run of a real log.
#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    /// Session id (`synth-<seed>-<n>` or `log-<session>-<n>.<k>`).
    pub session: String,
    /// Bundle id of the app the keys were typed in.
    pub bundle_id: String,
    /// Every key in order, including word boundaries (space, punctuation).
    /// Contains no hotkeys.
    pub keys: Vec<KeyEvent>,
    /// Words in order with their labels.
    pub words: Vec<ScriptWord>,
    /// Target [`Action`] for every key, same index as `keys`.
    pub targets: Vec<Action>,
}
