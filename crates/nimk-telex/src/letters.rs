//! Vietnamese letter tables: tones, vowel marks, composing and decomposing
//! precomposed characters. All functions take and return lowercase letters.

/// Tone of a syllable (thanh điệu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Tone {
    /// Thanh ngang (no mark).
    #[default]
    Level,
    /// Sắc, Telex `s`.
    Acute,
    /// Huyền, Telex `f`.
    Grave,
    /// Hỏi, Telex `r`.
    Hook,
    /// Ngã, Telex `x`.
    Tilde,
    /// Nặng, Telex `j`.
    Dot,
}

impl Tone {
    const ALL: [Self; 6] = [
        Self::Level,
        Self::Acute,
        Self::Grave,
        Self::Hook,
        Self::Tilde,
        Self::Dot,
    ];

    /// Column of this tone in [`VOWELS`].
    const fn index(self) -> usize {
        match self {
            Self::Level => 0,
            Self::Acute => 1,
            Self::Grave => 2,
            Self::Hook => 3,
            Self::Tilde => 4,
            Self::Dot => 5,
        }
    }

    /// Tone set by a Telex key (case-insensitive), if `key` is a tone key.
    pub(crate) fn from_key(key: char) -> Option<Self> {
        match key.to_ascii_lowercase() {
            's' => Some(Self::Acute),
            'f' => Some(Self::Grave),
            'r' => Some(Self::Hook),
            'x' => Some(Self::Tilde),
            'j' => Some(Self::Dot),
            _ => None,
        }
    }

    /// Telex key that sets this tone; `None` for the level tone.
    pub(crate) const fn key(self) -> Option<char> {
        match self {
            Self::Level => None,
            Self::Acute => Some('s'),
            Self::Grave => Some('f'),
            Self::Hook => Some('r'),
            Self::Tilde => Some('x'),
            Self::Dot => Some('j'),
        }
    }

    /// Whether the tone may combine with a stop final (`c ch p t`).
    pub(crate) const fn fits_stop_final(self) -> bool {
        matches!(self, Self::Acute | Self::Dot)
    }
}

/// Diacritic that changes a letter's base (not a tone).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mark {
    /// Circumflex: â ê ô (Telex `aa ee oo`).
    Hat,
    /// Breve: ă (Telex `aw`).
    Breve,
    /// Horn: ơ ư (Telex `ow uw`).
    Horn,
    /// Stroke: đ (Telex `dd`).
    Stroke,
}

/// Every vowel with its six tone forms, in [`Tone::index`] order.
const VOWELS: [[char; 6]; 12] = [
    ['a', 'á', 'à', 'ả', 'ã', 'ạ'],
    ['ă', 'ắ', 'ằ', 'ẳ', 'ẵ', 'ặ'],
    ['â', 'ấ', 'ầ', 'ẩ', 'ẫ', 'ậ'],
    ['e', 'é', 'è', 'ẻ', 'ẽ', 'ẹ'],
    ['ê', 'ế', 'ề', 'ể', 'ễ', 'ệ'],
    ['i', 'í', 'ì', 'ỉ', 'ĩ', 'ị'],
    ['o', 'ó', 'ò', 'ỏ', 'õ', 'ọ'],
    ['ô', 'ố', 'ồ', 'ổ', 'ỗ', 'ộ'],
    ['ơ', 'ớ', 'ờ', 'ở', 'ỡ', 'ợ'],
    ['u', 'ú', 'ù', 'ủ', 'ũ', 'ụ'],
    ['ư', 'ứ', 'ừ', 'ử', 'ữ', 'ự'],
    ['y', 'ý', 'ỳ', 'ỷ', 'ỹ', 'ỵ'],
];

/// Consonant letters used in Vietnamese spelling (lowercase).
const CONSONANTS: &str = "bcdđghklmnpqrstvx";

/// Returns true if `base` (lowercase, no tone) is a vowel.
pub(crate) fn is_vowel(base: char) -> bool {
    VOWELS.iter().any(|row| row[0] == base)
}

/// Returns true if `base` (lowercase, no tone) is a letter of the Vietnamese alphabet.
pub(crate) fn is_vi_letter(base: char) -> bool {
    is_vowel(base) || CONSONANTS.contains(base)
}

/// Puts `tone` on vowel `base`; non-vowels are returned unchanged.
pub(crate) fn with_tone(base: char, tone: Tone) -> char {
    VOWELS
        .iter()
        .find(|row| row[0] == base)
        .map_or(base, |row| row[tone.index()])
}

/// Splits a lowercase character into its toneless base and tone.
pub(crate) fn split_tone(c: char) -> (char, Tone) {
    for row in &VOWELS {
        for tone in Tone::ALL {
            if row[tone.index()] == c {
                return (row[0], tone);
            }
        }
    }
    (c, Tone::Level)
}

/// The letter without its mark (`â` -> `a`, `đ` -> `d`).
pub(crate) fn plain(base: char) -> char {
    match base {
        'ă' | 'â' => 'a',
        'ê' => 'e',
        'ô' | 'ơ' => 'o',
        'ư' => 'u',
        'đ' => 'd',
        other => other,
    }
}

/// The mark carried by `base`, if any.
pub(crate) fn mark_of(base: char) -> Option<Mark> {
    match base {
        'â' | 'ê' | 'ô' => Some(Mark::Hat),
        'ă' => Some(Mark::Breve),
        'ơ' | 'ư' => Some(Mark::Horn),
        'đ' => Some(Mark::Stroke),
        _ => None,
    }
}

/// Lowercases one character (Vietnamese letters map to a single char).
pub(crate) fn to_lower(c: char) -> char {
    match c.to_lowercase().next() {
        Some(l) => l,
        None => c,
    }
}

/// Uppercases one character (Vietnamese letters map to a single char).
pub(crate) fn to_upper(c: char) -> char {
    match c.to_uppercase().next() {
        Some(u) => u,
        None => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_round_trip() {
        for row in &VOWELS {
            for tone in Tone::ALL {
                assert_eq!(split_tone(with_tone(row[0], tone)), (row[0], tone));
            }
        }
    }

    #[test]
    fn marks() {
        assert_eq!(plain('ư'), 'u');
        assert_eq!(mark_of('ă'), Some(Mark::Breve));
        assert_eq!(mark_of('a'), None);
        assert!(is_vi_letter('đ') && !is_vi_letter('f'));
    }
}
