use std::collections::HashSet;

use nimk_types::{KeyKind, Lang, Modifiers, keycode};

use super::*;

fn small_cfg(seed: u64) -> SynthConfig {
    SynthConfig {
        seed,
        sessions: 4,
        words_per_session: 300,
        ..SynthConfig::default()
    }
}

#[test]
fn same_seed_same_output() {
    let corpus = Corpus::builtin();
    let a = generate(&small_cfg(9), &corpus);
    let b = generate(&small_cfg(9), &corpus);
    assert_eq!(a, b);
    assert_ne!(a, generate(&small_cfg(10), &corpus));
    assert_ne!(a[0].words, a[1].words, "sessions should differ");
    assert_eq!(a[2].session, "synth-9-2");
}

#[test]
fn word_ids_and_counts() {
    let cfg = small_cfg(1);
    for s in generate(&cfg, &Corpus::builtin()) {
        assert_eq!(s.words.len(), cfg.words_per_session);
        for (i, w) in s.words.iter().enumerate() {
            assert_eq!(w.word_id, i as u64);
            assert!(!w.raw.is_empty());
        }
    }
}

#[test]
fn keys_spell_the_words_between_boundaries() {
    for s in generate(&small_cfg(2), &Corpus::builtin()) {
        let mut raws = Vec::new();
        let mut cur = String::new();
        for k in &s.keys {
            assert_eq!(k.kind, KeyKind::KeyDown);
            let c = k.chars.chars().next().unwrap_or(' ');
            if c.is_alphabetic() {
                cur.push(c);
            } else if !cur.is_empty() {
                raws.push(std::mem::take(&mut cur));
            }
        }
        assert!(cur.is_empty(), "session must end with a boundary");
        let expected: Vec<&str> = s.words.iter().map(|w| w.raw.as_str()).collect();
        assert_eq!(raws, expected);
        assert_eq!(s.keys.last().map(|k| k.keycode), Some(keycode::RETURN));
    }
}

#[test]
fn key_events_are_plausible() {
    for s in generate(&small_cfg(3), &Corpus::builtin()) {
        let mut prev = f64::MIN;
        for k in &s.keys {
            assert!(k.timestamp > prev);
            prev = k.timestamp;
            let c = k.chars.chars().next().unwrap_or(' ');
            let shifted = c.is_uppercase() || c == '?' || c == '!';
            assert_eq!(k.mods.has(Modifiers::SHIFT), shifted, "{c:?}");
            assert!(!k.mods.is_shortcut());
            if c == ' ' {
                assert_eq!(k.keycode, keycode::SPACE);
            }
        }
        let first = s
            .words
            .first()
            .map(|w| w.text.chars().next().unwrap_or('a'));
        assert!(
            first.is_some_and(char::is_uppercase),
            "sentences start capitalized"
        );
    }
}

#[test]
fn vi_raw_transforms_back_to_text() {
    let scripts = generate(&small_cfg(4), &Corpus::builtin());
    let mut n_vi = 0;
    for w in scripts.iter().flat_map(|s| &s.words) {
        match w.lang {
            Lang::Vi => {
                n_vi += 1;
                assert_eq!(nimk_telex::transform(&w.raw).text, w.text, "{}", w.raw);
            }
            Lang::En => assert_eq!(w.raw, w.text),
        }
    }
    assert!(n_vi > 0);
}

#[test]
fn ambiguous_share_matches_profile() {
    let corpus = Corpus::builtin();
    let amb: HashSet<&str> = corpus.en_ambiguous.iter().map(String::as_str).collect();
    let cfg = SynthConfig {
        sessions: 20,
        words_per_session: 500,
        mix: MixProfile {
            p_vi_sentence: 0.3,
            ..MixProfile::default()
        },
        ..SynthConfig::default()
    };
    let (mut en, mut hard) = (0usize, 0usize);
    for w in generate(&cfg, &corpus).iter().flat_map(|s| &s.words) {
        if w.lang == Lang::En {
            en += 1;
            if amb.contains(w.text.to_lowercase().as_str()) {
                hard += 1;
            }
        }
    }
    let share = hard as f64 / en as f64;
    assert!(
        (share - f64::from(cfg.mix.p_ambiguous)).abs() < 0.03,
        "share {share} over {en}"
    );
}

#[test]
fn language_mix_roughly_matches_profile() {
    let cfg = SynthConfig {
        sessions: 20,
        ..SynthConfig::default()
    };
    let words: Vec<_> = generate(&cfg, &Corpus::builtin())
        .into_iter()
        .flat_map(|s| s.words)
        .collect();
    let vi = words.iter().filter(|w| w.lang == Lang::Vi).count() as f64 / words.len() as f64;
    // 0.7 * 0.85 + 0.3 * 0.05 = 0.61
    assert!((vi - 0.61).abs() < 0.05, "vi share {vi}");
}

#[test]
fn empty_corpus_yields_empty_sessions() {
    let corpus = Corpus {
        vi: Vec::new(),
        en: Vec::new(),
        en_ambiguous: Vec::new(),
    };
    let scripts = generate(&small_cfg(5), &corpus);
    assert_eq!(scripts.len(), 4);
    assert!(
        scripts
            .iter()
            .all(|s| s.words.is_empty() && s.keys.is_empty())
    );
    let vi_only = Corpus {
        vi: vec!["một".to_owned()],
        en: Vec::new(),
        en_ambiguous: Vec::new(),
    };
    let scripts = generate(&small_cfg(5), &vi_only);
    assert!(
        scripts
            .iter()
            .flat_map(|s| &s.words)
            .all(|w| w.lang == Lang::Vi)
    );
}

#[test]
fn variant_probability_controls_the_typing_style() {
    let corpus = Corpus::builtin();
    let with = |p: f32| {
        let cfg = SynthConfig {
            mix: MixProfile {
                p_variant: p,
                ..MixProfile::default()
            },
            ..small_cfg(3)
        };
        generate(&cfg, &corpus)
    };
    let (off, on) = (with(0.0), with(1.0));
    for (scripts, expect_variants) in [(&off, false), (&on, true)] {
        let (mut variants, mut vi) = (0, 0);
        for w in scripts.iter().flat_map(|s| &s.words) {
            if w.lang == Lang::Vi {
                vi += 1;
                assert_eq!(nimk_telex::transform(&w.raw).text, w.text, "{}", w.raw);
                if nimk_telex::encode_variants(&w.text).contains(&w.raw) {
                    variants += 1;
                }
            }
        }
        assert_eq!(variants > 0, expect_variants, "variant words: {variants}");
        if expect_variants {
            // Most words have at least one variant.
            assert!(variants * 2 > vi, "{variants} of {vi}");
        }
    }
}

#[test]
fn script_of_types_the_given_words() {
    let s = script_of(&[(Lang::Vi, "thêm", "theme"), (Lang::En, "app", "app")]);
    assert_eq!(s.keys.len(), s.targets.len());
    assert_eq!(s.keys.len(), "theme app ".len());
    assert_eq!(s.words.len(), 2);
}
