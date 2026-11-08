use serde::{Deserialize, Serialize};

/// Input language mode of the IME.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    /// Keys pass through unchanged.
    #[default]
    En,
    /// Keys are interpreted as Telex.
    Vi,
}

impl Lang {
    /// Returns the other language.
    #[must_use]
    pub const fn flip(self) -> Self {
        match self {
            Self::En => Self::Vi,
            Self::Vi => Self::En,
        }
    }

    /// Maps a probability of Vietnamese to the more likely language.
    #[must_use]
    pub fn from_p_vi(p_vi: f32) -> Self {
        if p_vi >= 0.5 { Self::Vi } else { Self::En }
    }

    /// Target value for binary training: `1.0` for Vietnamese, `0.0` for English.
    #[must_use]
    pub const fn as_target(self) -> f32 {
        match self {
            Self::En => 0.0,
            Self::Vi => 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flip_is_involution() {
        assert_eq!(Lang::Vi.flip(), Lang::En);
        assert_eq!(Lang::En.flip().flip(), Lang::En);
    }

    #[test]
    fn from_p_vi_threshold() {
        assert_eq!(Lang::from_p_vi(0.5), Lang::Vi);
        assert_eq!(Lang::from_p_vi(0.49), Lang::En);
    }
}
