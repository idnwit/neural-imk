//! Fixed-size encoding of an [`Observation`] shared by all action models.
//!
//! An observation becomes three parts, all stored inline (no heap):
//!
//! - **tokens**: the last [`SEQ_LEN`] tokens of
//!   `recent events ++ [SEP] ++ raw chars ++ [incoming key]`, oldest first,
//!   left-padded with [`tok::PAD`]. Recent events and `raw` overlap on
//!   purpose: `recent` carries history across word boundaries, `SEP ++ raw`
//!   states the buffer explicitly. When the sequence is longer than the
//!   window, the oldest tokens are dropped (a very long `raw` may push out
//!   `SEP` itself).
//! - **scalars**: [`SCALARS`] dense features in `[-1, 1]` (see [`scalar`]).
//! - **context ids**: hashed `bundle_id` bucket and caret cell.
//!
//! Token ids `0..=127` are ASCII bytes (only graphic `0x21..=0x7E` occur;
//! space is [`tok::SPACE`]); ids `128..` are the specials in [`tok`].

use nimk_types::{KeyEvent, KeyKind, Lang, Modifiers, Observation, keycode};

/// Number of tokens fed to the models.
pub const SEQ_LEN: usize = 24;
/// Number of dense scalar features.
pub const SCALARS: usize = 18;
/// Hash buckets for `bundle_id`.
pub const BUNDLE_BUCKETS: usize = 256;
/// Caret cells: an 8x8 grid plus "unknown".
pub const CARET_CELLS: usize = 65;
/// Caret grid side.
const CARET_GRID: u32 = 8;

/// Special token ids (after the 128 byte ids).
pub mod tok {
    /// Left padding.
    pub const PAD: u8 = 128;
    /// Marks the start of the current buffer (`raw`).
    pub const SEP: u8 = 129;
    /// Space bar.
    pub const SPACE: u8 = 130;
    /// Return / Enter.
    pub const RETURN: u8 = 131;
    /// Tab.
    pub const TAB: u8 = 132;
    /// Backspace.
    pub const BACKSPACE: u8 = 133;
    /// Arrow keys.
    pub const NAV: u8 = 134;
    /// Any key with Command, Control or Option held.
    pub const SHORTCUT: u8 = 135;
    /// Escape, non-ASCII or empty characters.
    pub const OTHER: u8 = 136;
    /// A modifier-only (`FlagsChanged`) event.
    pub const FLAGS: u8 = 137;
    /// No incoming key (mode prior on focus change).
    pub const NONE: u8 = 138;
    /// Vocabulary size (embedding rows).
    pub const VOCAB: usize = 139;
}

/// Indices into [`Encoded::scalars`].
pub mod scalar {
    /// `valid_vi` (0/1).
    pub const VALID_VI: usize = 0;
    /// `viable_vi` (0/1).
    pub const VIABLE_VI: usize = 1;
    /// `vi != raw`: the Telex tool changed the buffer (0/1).
    pub const TELEX_CHANGED: usize = 2;
    /// Reserved, always 0. Formerly `shown == Vi`: under teacher forcing
    /// `shown` is the previous key's render *target*, so the feature leaked
    /// the label and the model learned to copy it (held-out render accuracy
    /// 0.65). Kept as a slot so the snapshot dimensions stay unchanged.
    pub const SHOWN_VI: usize = 3;
    /// `mode == Vi` (0/1).
    pub const MODE_VI: usize = 4;
    /// `prev_langs[0..3]`: +1 VI, -1 EN, 0 absent (3 slots).
    pub const PREV_LANGS: usize = 5;
    /// `1 / (1 + secs_since_switch / 10)`: 1 just switched, 0 never.
    pub const SWITCH_RECENCY: usize = 8;
    /// `raw` length / 10, capped at 1.
    pub const RAW_LEN: usize = 9;
    /// `mask.append` (0/1).
    pub const MASK_APPEND: usize = 10;
    /// `mask.pop` (0/1).
    pub const MASK_POP: usize = 11;
    /// `mask.commit` (0/1).
    pub const MASK_COMMIT: usize = 12;
    /// Shift held on the incoming key (0/1).
    pub const SHIFT: usize = 13;
    /// Command held (0/1).
    pub const CMD: usize = 14;
    /// Control held (0/1).
    pub const CTRL: usize = 15;
    /// Option held (0/1).
    pub const OPT: usize = 16;
    /// An incoming key is present (0/1).
    pub const HAS_KEY: usize = 17;
}

/// Model input for one observation; `Copy` and heap-free.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Encoded {
    /// Token ids, oldest first, incoming key last, left-padded.
    pub tokens: [u8; SEQ_LEN],
    /// Number of non-padding tokens (suffix of `tokens`).
    pub len: usize,
    /// Dense features, see [`scalar`].
    pub scalars: [f32; SCALARS],
    /// `bundle_id` bucket in `0..BUNDLE_BUCKETS`.
    pub bundle: u16,
    /// Caret cell in `0..CARET_CELLS` (`64` = unknown).
    pub caret: u8,
}

impl Default for Encoded {
    fn default() -> Self {
        Self {
            tokens: [tok::PAD; SEQ_LEN],
            len: 0,
            scalars: [0.0; SCALARS],
            bundle: 0,
            caret: 0,
        }
    }
}

impl Encoded {
    /// Encodes `obs`. Does not allocate.
    #[must_use]
    pub fn new(obs: &Observation) -> Self {
        let mut e = Self::default();
        e.fill_tokens(obs);
        e.fill_scalars(obs);
        e.bundle = bundle_bucket(&obs.bundle_id);
        e.caret = obs.caret.map_or(64, |c| c.cell(CARET_GRID).min(63) as u8);
        e
    }

    /// Index where the current buffer starts (the position after the last
    /// `SEP`), or the first real token if `SEP` fell out of the window.
    #[must_use]
    pub fn buffer_start(&self) -> usize {
        let first = SEQ_LEN - self.len;
        self.tokens[first..]
            .iter()
            .rposition(|&t| t == tok::SEP)
            .map_or(first, |i| first + i + 1)
    }

    fn fill_tokens(&mut self, obs: &Observation) {
        // Fill right-to-left so the newest tokens survive truncation.
        let mut w = TokenWriter {
            tokens: &mut self.tokens,
            next: SEQ_LEN,
        };
        w.push(obs.key.as_ref().map_or(tok::NONE, key_token));
        for c in obs.raw.chars().rev() {
            w.push(char_token(c));
        }
        w.push(tok::SEP);
        for ev in obs.recent.iter().rev() {
            w.push(key_token(ev));
        }
        self.len = SEQ_LEN - w.next;
    }

    fn fill_scalars(&mut self, obs: &Observation) {
        let s = &mut self.scalars;
        let b = |v: bool| if v { 1.0 } else { 0.0 };
        s[scalar::VALID_VI] = b(obs.valid_vi);
        s[scalar::VIABLE_VI] = b(obs.viable_vi);
        s[scalar::TELEX_CHANGED] = b(obs.vi != obs.raw);
        s[scalar::MODE_VI] = b(obs.mode == Lang::Vi);
        for (slot, lang) in s[scalar::PREV_LANGS..scalar::PREV_LANGS + 3]
            .iter_mut()
            .zip(obs.prev_langs.iter())
        {
            *slot = match lang {
                Lang::Vi => 1.0,
                Lang::En => -1.0,
            };
        }
        let secs = obs.secs_since_switch;
        s[scalar::SWITCH_RECENCY] = if secs.is_finite() && secs >= 0.0 {
            1.0 / (1.0 + secs / 10.0)
        } else {
            0.0
        };
        s[scalar::RAW_LEN] = (obs.raw.chars().count() as f32 / 10.0).min(1.0);
        s[scalar::MASK_APPEND] = b(obs.mask.append);
        s[scalar::MASK_POP] = b(obs.mask.pop);
        s[scalar::MASK_COMMIT] = b(obs.mask.commit);
        let mods = obs.key.as_ref().map_or(Modifiers(0), |k| k.mods);
        s[scalar::SHIFT] = b(mods.has(Modifiers::SHIFT));
        s[scalar::CMD] = b(mods.has(Modifiers::COMMAND));
        s[scalar::CTRL] = b(mods.has(Modifiers::CONTROL));
        s[scalar::OPT] = b(mods.has(Modifiers::OPTION));
        s[scalar::HAS_KEY] = b(obs.key.is_some());
    }
}

/// Writes tokens from the end of the window towards the start.
struct TokenWriter<'a> {
    tokens: &'a mut [u8; SEQ_LEN],
    next: usize,
}

impl TokenWriter<'_> {
    fn push(&mut self, t: u8) {
        if self.next > 0 {
            self.next -= 1;
            self.tokens[self.next] = t;
        }
    }
}

/// Token id of one buffer character.
#[must_use]
pub fn char_token(c: char) -> u8 {
    match c {
        ' ' => tok::SPACE,
        '\x21'..='\x7e' => c as u8,
        _ => tok::OTHER,
    }
}

/// Token id of one key event.
#[must_use]
pub fn key_token(ev: &KeyEvent) -> u8 {
    if ev.kind == KeyKind::FlagsChanged {
        return tok::FLAGS;
    }
    if ev.mods.is_shortcut() {
        return tok::SHORTCUT;
    }
    match ev.keycode {
        keycode::SPACE => tok::SPACE,
        keycode::RETURN => tok::RETURN,
        keycode::TAB => tok::TAB,
        keycode::BACKSPACE => tok::BACKSPACE,
        keycode::LEFT | keycode::RIGHT | keycode::UP | keycode::DOWN => tok::NAV,
        keycode::ESCAPE => tok::OTHER,
        _ => ev.chars.chars().next().map_or(tok::OTHER, char_token),
    }
}

/// FNV-1a over `bytes`, seeded so different feature namespaces differ.
#[must_use]
pub(crate) fn fnv1a(seed: u32, bytes: &[u8]) -> u32 {
    let mut h = 0x811c_9dc5_u32 ^ seed.wrapping_mul(0x0100_0193);
    for &b in bytes {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Bucket of `bundle_id` in `0..BUNDLE_BUCKETS`.
#[must_use]
pub fn bundle_bucket(bundle_id: &str) -> u16 {
    (fnv1a(0xb0b0, bundle_id.as_bytes()) % BUNDLE_BUCKETS as u32) as u16
}
