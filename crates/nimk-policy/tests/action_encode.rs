//! Observation encoding: determinism, padding, special tokens, truncation.

mod common;

use common::{backspace, cmd_c, key, letter, ret, space};
use nimk_policy::encode::{Encoded, SEQ_LEN, bundle_bucket, key_token, scalar, tok};
use nimk_types::{
    ActionMask, CaretRel, KeyEvent, KeyKind, Lang, Modifiers, Observation, RECENT_WINDOW, keycode,
};

fn obs(key: Option<KeyEvent>, recent: Vec<KeyEvent>, raw: &str) -> Observation {
    Observation {
        key,
        recent,
        raw: raw.to_owned(),
        vi: raw.to_owned(),
        bundle_id: "com.apple.TextEdit".to_owned(),
        secs_since_switch: f32::INFINITY,
        ..Observation::default()
    }
}

#[test]
fn encoding_is_deterministic() {
    let o = obs(Some(letter('a')), vec![letter('x'), space()], "ti");
    assert_eq!(Encoded::new(&o), Encoded::new(&o.clone()));
}

#[test]
fn short_sequence_is_left_padded() {
    let e = Encoded::new(&obs(Some(letter('a')), vec![], ""));
    assert_eq!(e.len, 2);
    assert!(e.tokens[..SEQ_LEN - 2].iter().all(|&t| t == tok::PAD));
    assert_eq!(e.tokens[SEQ_LEN - 2], tok::SEP);
    assert_eq!(e.tokens[SEQ_LEN - 1], b'a');
    assert_eq!(e.buffer_start(), SEQ_LEN - 1);
}

#[test]
fn order_is_recent_sep_raw_key() {
    let e = Encoded::new(&obs(Some(letter('c')), vec![letter('x'), space()], "ab"));
    let tail = &e.tokens[SEQ_LEN - 6..];
    assert_eq!(tail, &[b'x', tok::SPACE, tok::SEP, b'a', b'b', b'c']);
    assert_eq!(e.len, 6);
    assert_eq!(e.buffer_start(), SEQ_LEN - 3);
}

#[test]
fn special_tokens() {
    let flags = KeyEvent {
        kind: KeyKind::FlagsChanged,
        keycode: keycode::LEFT_SHIFT,
        chars: String::new(),
        mods: Modifiers(Modifiers::SHIFT),
        timestamp: 0.0,
    };
    let cases = [
        (space(), tok::SPACE),
        (ret(), tok::RETURN),
        (key(keycode::TAB, "\t", 0), tok::TAB),
        (backspace(), tok::BACKSPACE),
        (key(keycode::LEFT, "", 0), tok::NAV),
        (key(keycode::UP, "", 0), tok::NAV),
        (cmd_c(), tok::SHORTCUT),
        (key(0, "a", Modifiers::CONTROL), tok::SHORTCUT),
        (key(keycode::ESCAPE, "\u{1b}", 0), tok::OTHER),
        (key(0, "é", 0), tok::OTHER),
        (key(0, "", 0), tok::OTHER),
        (flags, tok::FLAGS),
        (key(0, "A", Modifiers::SHIFT), b'A'),
        (key(43, ",", 0), b','),
    ];
    for (ev, want) in cases {
        assert_eq!(key_token(&ev), want, "{ev:?}");
    }
    let none = Encoded::new(&obs(None, vec![], ""));
    assert_eq!(none.tokens[SEQ_LEN - 1], tok::NONE);
    assert_eq!(none.scalars[scalar::HAS_KEY], 0.0);
}

#[test]
fn long_window_keeps_newest_tokens() {
    let recent: Vec<KeyEvent> = (0..RECENT_WINDOW)
        .map(|i| letter((b'a' + i as u8) as char))
        .collect();
    let raw = "klmnopqrst";
    // 16 recent + SEP + 10 raw + key = 28 > 24: the 4 oldest recent drop.
    let e = Encoded::new(&obs(Some(letter('z')), recent, raw));
    assert_eq!(e.len, SEQ_LEN);
    assert_eq!(e.tokens[0], b'e');
    assert_eq!(e.tokens[SEQ_LEN - 1], b'z');
    assert_eq!(e.tokens[SEQ_LEN - 12], tok::SEP);
    assert_eq!(e.buffer_start(), SEQ_LEN - 11);

    // A raw longer than the window pushes out SEP itself.
    let long = "abcdefghijklmnopqrstuvwxyz";
    let e = Encoded::new(&obs(Some(letter('z')), vec![], long));
    assert!(!e.tokens.contains(&tok::SEP));
    assert_eq!(e.buffer_start(), 0);
    assert_eq!(e.tokens[SEQ_LEN - 2], b'z');
}

#[test]
fn scalars_and_context() {
    let mut o = obs(
        Some(key(0, "a", Modifiers::SHIFT | Modifiers::OPTION)),
        vec![],
        "tieng",
    );
    o.vi = "tiêng".to_owned();
    o.valid_vi = true;
    o.viable_vi = true;
    o.shown = Lang::Vi;
    o.prev_langs = vec![Lang::Vi, Lang::En];
    o.secs_since_switch = 10.0;
    o.mask = ActionMask {
        append: true,
        pop: true,
        commit: false,
    };
    o.caret = CaretRel::new(0.99, 0.0);
    let e = Encoded::new(&o);
    let s = e.scalars;
    assert_eq!(s[scalar::VALID_VI], 1.0);
    assert_eq!(s[scalar::TELEX_CHANGED], 1.0);
    assert_eq!(
        s[scalar::SHOWN_VI],
        0.0,
        "shown is not a feature (label leak)"
    );
    assert_eq!(s[scalar::MODE_VI], 0.0);
    assert_eq!(
        &s[scalar::PREV_LANGS..scalar::PREV_LANGS + 3],
        &[1.0, -1.0, 0.0]
    );
    assert!((s[scalar::SWITCH_RECENCY] - 0.5).abs() < 1e-6);
    assert!((s[scalar::RAW_LEN] - 0.5).abs() < 1e-6);
    assert_eq!(
        [
            s[scalar::MASK_APPEND],
            s[scalar::MASK_POP],
            s[scalar::MASK_COMMIT]
        ],
        [1.0, 1.0, 0.0]
    );
    assert_eq!(
        [
            s[scalar::SHIFT],
            s[scalar::CMD],
            s[scalar::CTRL],
            s[scalar::OPT]
        ],
        [1.0, 0.0, 0.0, 1.0]
    );
    assert_eq!(e.caret, 7);
    assert_eq!(e.bundle, bundle_bucket("com.apple.TextEdit"));
    assert_eq!(Encoded::new(&obs(None, vec![], "")).caret, 64);
    assert_eq!(
        Encoded::new(&obs(None, vec![], "")).scalars[scalar::SWITCH_RECENCY],
        0.0
    );
}
