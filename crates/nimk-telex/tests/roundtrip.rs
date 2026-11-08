//! Property test: `transform(encode(w)) == w` over every valid syllable built
//! combinatorially from initials x vowel groups x finals x tones, with the
//! documented "new style" tone placement restated independently here.

use nimk_telex::{encode, is_valid_syllable, is_viable_prefix, transform};

const INITIALS: [&str; 28] = [
    "", "b", "c", "ch", "d", "đ", "g", "gh", "gi", "h", "k", "kh", "l", "m", "n", "ng", "ngh",
    "nh", "p", "ph", "qu", "r", "s", "t", "th", "tr", "v", "x",
];

const GROUPS: [&str; 54] = [
    "a", "ă", "â", "e", "ê", "i", "o", "ô", "ơ", "u", "ư", "y", "ai", "ao", "au", "ay", "âu", "ây",
    "eo", "êu", "ia", "iê", "iu", "oa", "oă", "oe", "oi", "ôi", "ơi", "oo", "ua", "uâ", "uê", "ui",
    "uô", "uơ", "uy", "ưa", "ưi", "ươ", "ưu", "yê", "iêu", "oai", "oay", "oeo", "uây", "uôi",
    "uya", "uyê", "uyu", "ươi", "ươu", "yêu",
];

const FINALS: [&str; 9] = ["", "c", "ch", "m", "n", "ng", "nh", "p", "t"];

/// Tone forms of each vowel: [sắc, huyền, hỏi, ngã, nặng].
fn toned(v: char, tone: usize) -> char {
    let rows = [
        "aáàảãạ",
        "ăắằẳẵặ",
        "âấầẩẫậ",
        "eéèẻẽẹ",
        "êếềểễệ",
        "iíìỉĩị",
        "oóòỏõọ",
        "ôốồổỗộ",
        "ơớờởỡợ",
        "uúùủũụ",
        "ưứừửữự",
        "yýỳỷỹỵ",
    ];
    rows.iter()
        .map(|r| r.chars().collect::<Vec<_>>())
        .find(|r| r[0] == v)
        .map_or(v, |r| r[tone])
}

/// Index in `group` that carries the tone (new style).
fn tone_index(group: &[char], has_final: bool) -> usize {
    if let Some(p) = group.iter().rposition(|c| "âăêôơư".contains(*c)) {
        return p;
    }
    let s: String = group.iter().collect();
    match (group.len(), has_final) {
        (1, _) => 0,
        (n, true) => n - 1,
        (2, false) if ["oa", "oe", "ue", "uy"].contains(&s.as_str()) => 1,
        (2, false) => 0,
        _ => 1,
    }
}

fn valid_syllables() -> Vec<String> {
    let mut out = Vec::new();
    for ini in INITIALS {
        for group in GROUPS {
            // `g` + `i...` re-parses as `gi`; `uơ` is typeable after h kh th qu only.
            if (ini == "g" && group.starts_with('i') && group.chars().count() > 1)
                || (group == "uơ" && !["qu", "h", "kh", "th"].contains(&ini))
            {
                continue;
            }
            let g: Vec<char> = group.chars().collect();
            for fin in FINALS {
                let at = tone_index(&g, !fin.is_empty());
                for tone in 0..6 {
                    let nucleus: String = g
                        .iter()
                        .enumerate()
                        .map(|(i, &c)| if i == at { toned(c, tone) } else { c })
                        .collect();
                    let w = format!("{ini}{nucleus}{fin}");
                    if is_valid_syllable(&w) {
                        out.push(w);
                    }
                }
            }
        }
    }
    out
}

fn title(w: &str) -> String {
    let mut cs = w.chars();
    cs.next()
        .map(|c| c.to_uppercase().chain(cs).collect())
        .unwrap_or_default()
}

#[test]
fn encode_round_trips_every_valid_syllable() {
    let words = valid_syllables();
    assert!(
        words.len() > 5000,
        "only {} syllables generated",
        words.len()
    );
    for w in &words {
        for form in [w.clone(), title(w), w.to_uppercase()] {
            let raw = encode(&form);
            let t = transform(&raw);
            assert_eq!(t.text, form, "raw {raw}");
            assert!(t.valid_vi, "{form} not valid after transform");
        }
    }
}

#[test]
fn spot_check_generated_set() {
    let words = valid_syllables();
    for w in [
        "tiếng",
        "việt",
        "đường",
        "nghiêng",
        "khuỷu",
        "quyết",
        "giường",
        "xoóng",
        "hoà",
        "thuý",
    ] {
        assert!(words.iter().any(|x| x == w), "{w} missing");
    }
}

#[test]
fn every_prefix_of_a_valid_syllable_is_viable() {
    for w in valid_syllables() {
        let chars: Vec<char> = w.chars().collect();
        for n in 0..=chars.len() {
            let prefix: String = chars[..n].iter().collect();
            assert!(is_viable_prefix(&prefix), "{prefix} (of {w})");
            let raw_prefix: String = encode(&w).chars().take(n).collect();
            assert!(
                transform(&raw_prefix).viable_vi,
                "raw {raw_prefix} (of {w})"
            );
        }
    }
}

#[test]
fn every_variant_types_the_word() {
    let mut with_variant = 0;
    for w in valid_syllables() {
        for form in [w.clone(), title(&w), w.to_uppercase()] {
            let variants = nimk_telex::encode_variants(&form);
            assert!(variants.len() <= 8);
            for raw in variants {
                with_variant += 1;
                let t = transform(&raw);
                assert_eq!(t.text, form, "variant {raw}");
                assert!(t.valid_vi);
                assert_ne!(raw, encode(&form));
            }
        }
    }
    assert!(with_variant > 20_000, "only {with_variant} variants");
}
