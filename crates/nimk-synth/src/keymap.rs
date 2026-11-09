//! Deterministic RNG and the US-ANSI character -> macOS keycode map.

use nimk_types::keycode;

/// SplitMix64: tiny, fast, good enough for synthetic data, and stable
/// across platforms and releases (unlike external RNG crates' streams).
#[derive(Debug, Clone)]
pub(crate) struct Rng(u64);

impl Rng {
    /// Seeds the generator.
    pub(crate) const fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Next 64 random bits.
    pub(crate) fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub(crate) fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// True with probability `p`.
    pub(crate) fn chance(&mut self, p: f32) -> bool {
        self.unit() < f64::from(p)
    }

    /// Uniform index in `0..n`; `n` must be > 0.
    pub(crate) fn below(&mut self, n: usize) -> usize {
        // Modulo bias is negligible for word-list sizes.
        (self.next_u64() % n as u64) as usize
    }
}

/// macOS virtual keycode (`kVK_ANSI_*`) and whether Shift is needed for `c`
/// on a US keyboard. Unknown characters map to `kVK_ANSI_Grave` (50).
pub(crate) fn key_for(c: char) -> (u16, bool) {
    let shift = c.is_ascii_uppercase() || "!?\"()_".contains(c);
    let code = match c.to_ascii_lowercase() {
        'a' => 0x00,
        's' => 0x01,
        'd' => 0x02,
        'f' => 0x03,
        'h' => 0x04,
        'g' => 0x05,
        'z' => 0x06,
        'x' => 0x07,
        'c' => 0x08,
        'v' => 0x09,
        'b' => 0x0B,
        'q' => 0x0C,
        'w' => 0x0D,
        'e' => 0x0E,
        'r' => 0x0F,
        'y' => 0x10,
        't' => 0x11,
        '1' | '!' => 0x12,
        '2' => 0x13,
        '3' => 0x14,
        '4' => 0x15,
        '6' => 0x16,
        '5' => 0x17,
        '9' | '(' => 0x19,
        '7' => 0x1A,
        '-' | '_' => 0x1B,
        '8' => 0x1C,
        '0' | ')' => 0x1D,
        'o' => 0x1F,
        'u' => 0x20,
        'i' => 0x22,
        'p' => 0x23,
        'l' => 0x25,
        'j' => 0x26,
        '\'' | '"' => 0x27,
        'k' => 0x28,
        ';' => 0x29,
        ',' => 0x2B,
        '/' | '?' => 0x2C,
        'n' => 0x2D,
        'm' => 0x2E,
        '.' => 0x2F,
        ' ' => keycode::SPACE,
        '\r' | '\n' => keycode::RETURN,
        '\t' => keycode::TAB,
        _ => 0x32,
    };
    (code, shift)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
            let u = a.unit();
            assert_eq!(u, b.unit());
            assert!((0.0..1.0).contains(&u));
            assert!(a.below(10) < 10);
            assert!(b.below(10) < 10);
        }
    }

    #[test]
    fn keycodes() {
        assert_eq!(key_for('a'), (0, false));
        assert_eq!(key_for('A'), (0, true));
        assert_eq!(key_for('?'), (0x2C, true));
        assert_eq!(key_for(' '), (keycode::SPACE, false));
        let letters: std::collections::HashSet<u16> = ('a'..='z').map(|c| key_for(c).0).collect();
        assert_eq!(letters.len(), 26);
    }
}
