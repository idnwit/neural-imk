//! Per-app statistics category.

use serde::{Deserialize, Serialize};

/// App category of a counted word.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatCategory {
    /// Terminal emulators.
    Terminal,
    /// Code editors / IDEs.
    Ide,
    /// Web browsers.
    Browser,
    /// Text editors and note apps.
    Editor,
    /// Any other app.
    Other,
}

impl StatCategory {
    /// Every category, in the order of `index`.
    pub const ALL: [Self; 5] = [
        Self::Terminal,
        Self::Ide,
        Self::Browser,
        Self::Editor,
        Self::Other,
    ];

    /// Maps an `[[app_category]]` name; unknown names are `Other`.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "terminal" => Self::Terminal,
            "ide" => Self::Ide,
            "browser" => Self::Browser,
            "editor" => Self::Editor,
            _ => Self::Other,
        }
    }

    /// Position in [`StatCategory::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Terminal => 0,
            Self::Ide => 1,
            Self::Browser => 2,
            Self::Editor => 3,
            Self::Other => 4,
        }
    }
}
