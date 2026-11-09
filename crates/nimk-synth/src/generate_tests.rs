use std::collections::HashSet;

use nimk_types::{EditAction, KeyKind, Lang, Modifiers, keycode};

use super::*;

fn small_cfg(seed: u64) -> SynthConfig {
    SynthConfig {
        seed,
        sessions: 4,
        words_per_session: 300,
        ..SynthConfig::default()
    }
}

/// Like [`small_cfg`] with chat sessions only (plain words and sentences).
fn chat_cfg(seed: u64) -> SynthConfig {
    SynthConfig {
        mix: MixProfile {
            p_terminal: 0.0,
            p_ide: 0.0,
            ..MixProfile::default()
        },
        ..small_cfg(seed)
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
    for s in generate(&chat_cfg(2), &Corpus::builtin()) {
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
    for s in generate(&chat_cfg(3), &Corpus::builtin()) {
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
            ..MixProfile::legacy()
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
        mix: MixProfile::legacy(),
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
        ..Corpus::empty()
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
        ..Corpus::empty()
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
                ..MixProfile::legacy()
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

/// Chat-only realistic profile without phrases (pure run model).
fn runs_only() -> MixProfile {
    MixProfile {
        p_phrase: 0.0,
        p_terminal: 0.0,
        p_ide: 0.0,
        ..MixProfile::default()
    }
}

/// Mean run length per language over every session's word sequence.
fn mean_runs(scripts: &[Script]) -> (f64, f64) {
    let (mut sum, mut cnt) = ([0usize; 2], [0usize; 2]);
    for s in scripts {
        let mut i = 0;
        while i < s.words.len() {
            let lang = s.words[i].lang;
            let mut j = i;
            while j < s.words.len() && s.words[j].lang == lang {
                j += 1;
            }
            let k = usize::from(lang == Lang::En);
            sum[k] += j - i;
            cnt[k] += 1;
            i = j;
        }
    }
    (sum[0] as f64 / cnt[0] as f64, sum[1] as f64 / cnt[1] as f64)
}

#[test]
fn run_lengths_match_the_profile() {
    let mix = MixProfile {
        mean_vi_run: 5.0,
        mean_en_run: 2.0,
        ..runs_only()
    };
    let cfg = SynthConfig {
        sessions: 40,
        mix,
        ..SynthConfig::default()
    };
    let (vi, en) = mean_runs(&generate(&cfg, &Corpus::builtin()));
    assert!((vi - 5.0).abs() < 0.6, "vi run {vi}");
    assert!((en - 2.0).abs() < 0.3, "en run {en}");
}

#[test]
fn phrases_are_inserted_in_order() {
    let corpus = Corpus::builtin();
    let cfg = |p_phrase| SynthConfig {
        sessions: 10,
        mix: MixProfile {
            p_phrase,
            ..runs_only()
        },
        ..SynthConfig::default()
    };
    let count = |scripts: &[Script]| {
        let mut n = 0;
        for s in scripts {
            for pair in s.words.windows(2) {
                let a = pair[0].text.to_lowercase();
                let b = pair[1].text.to_lowercase();
                if (a == "pull" && b == "request") || (a == "dark" && b == "theme") {
                    n += 1;
                }
            }
        }
        n
    };
    let with = count(&generate(&cfg(0.5), &corpus));
    let without = count(&generate(&cfg(0.0), &corpus));
    assert!(with >= 3, "phrases inserted: {with}");
    assert!(with > without * 3, "{with} vs {without}");
    // Phrase tokens keep their language labels.
    for w in generate(&cfg(0.5), &corpus).iter().flat_map(|s| &s.words) {
        if w.text.eq_ignore_ascii_case("deploy") {
            assert_eq!(w.lang, Lang::En);
        }
    }
}

#[test]
fn frequent_words_are_zipf_sampled() {
    let corpus = Corpus::builtin();
    let cfg = SynthConfig {
        sessions: 20,
        mix: runs_only(),
        ..SynthConfig::default()
    };
    let (mut en, mut to, mut rare) = (0usize, 0usize, 0usize);
    let tail: HashSet<&str> = corpus.en_freq.iter().skip(40).map(String::as_str).collect();
    for w in generate(&cfg, &corpus).iter().flat_map(|s| &s.words) {
        if w.lang == Lang::En {
            en += 1;
            let t = w.text.to_lowercase();
            to += usize::from(t == "to");
            rare += usize::from(tail.contains(t.as_str()));
        }
    }
    // Rank 2 of ~65 gets about 0.5 * (1/2) / H(65) = 5% of English words.
    let share = to as f64 / en as f64;
    assert!(share > 0.03, "`to` share {share}");
    assert!(rare < to * 3, "tail {rare} vs to {to}");
}

#[test]
fn flavours_set_bundle_and_keep_word_contract() {
    let corpus = Corpus::builtin();
    let cfg = SynthConfig {
        sessions: 60,
        words_per_session: 200,
        ..SynthConfig::default()
    };
    let scripts = generate(&cfg, &corpus);
    let ids: HashSet<&str> = scripts.iter().map(|s| s.bundle_id.as_str()).collect();
    for id in ["synthetic.chat", "synthetic.terminal", "synthetic.ide"] {
        assert!(ids.contains(id), "{id} missing from {ids:?}");
    }
    for s in &scripts {
        assert_eq!(s.words.len(), cfg.words_per_session, "{}", s.bundle_id);
        assert_eq!(s.keys.len(), s.targets.len());
        assert_eq!(s.keys.last().map(|k| k.keycode), Some(keycode::RETURN));
        let typed: String = s.keys.iter().flat_map(|k| k.chars.chars()).collect();
        match s.bundle_id.as_str() {
            "synthetic.terminal" => assert!(typed.contains('-') || typed.contains('/')),
            "synthetic.ide" => {
                assert!(typed.contains('_') || typed.contains('(') || typed.contains("//"))
            }
            _ => {}
        }
        if s.bundle_id != "synthetic.chat" {
            assert!(!typed.is_empty());
        }
    }
    // No flavours means the single legacy bundle id.
    let legacy = generate(
        &SynthConfig {
            mix: MixProfile::legacy(),
            ..small_cfg(1)
        },
        &corpus,
    );
    assert!(
        legacy
            .iter()
            .all(|s| s.bundle_id == crate::SYNTHETIC_BUNDLE_ID)
    );
}

#[test]
fn legacy_profile_is_deterministic_and_differs_from_default() {
    let corpus = Corpus::builtin();
    let cfg = |mix| SynthConfig {
        mix,
        ..small_cfg(4)
    };
    let a = generate(&cfg(MixProfile::legacy()), &corpus);
    assert_eq!(a, generate(&cfg(MixProfile::legacy()), &corpus));
    assert_ne!(a, generate(&cfg(MixProfile::default()), &corpus));
}

#[test]
fn flavoured_targets_agree_with_the_engine_word_walk() {
    let cfg = SynthConfig {
        sessions: 30,
        words_per_session: 150,
        ..SynthConfig::default()
    };
    for s in generate(&cfg, &Corpus::builtin()) {
        let walked: Vec<_> = crate::log::walk(&s.keys).collect();
        let ends = walked.iter().filter(|k| k.ends_word.is_some()).count();
        assert_eq!(ends, s.words.len(), "{}", s.bundle_id);
        for (k, t) in walked.iter().zip(&s.targets) {
            let expect = if k.ends_word.is_some() {
                EditAction::Commit
            } else if k.word_id.is_some() {
                EditAction::Append
            } else {
                EditAction::Pass
            };
            assert_eq!(t.edit, expect, "{} {:?}", s.bundle_id, k.key.chars);
        }
    }
}

#[test]
fn command_text_after_a_trigger_is_raw_and_unlabelled() {
    let cfg = SynthConfig {
        sessions: 80,
        words_per_session: 150,
        mix: MixProfile {
            p_terminal: 1.0,
            p_ide: 0.0,
            ..MixProfile::default()
        },
        ..SynthConfig::default()
    };
    let (mut raw_letters, mut raw_spans, mut typed_words) = (0usize, 0usize, 0usize);
    for s in generate(&cfg, &Corpus::builtin()) {
        assert_eq!(s.bundle_id, "synthetic.terminal");
        let mut letters_append = String::new();
        let mut after_trigger = false;
        for (k, t) in s.keys.iter().zip(&s.targets) {
            let c = k.chars.chars().next().unwrap_or(' ');
            if c.is_ascii_alphabetic() {
                if t.edit == EditAction::Append {
                    letters_append.push(c);
                } else {
                    assert_eq!(t.edit, EditAction::Pass, "raw letters are passed");
                    raw_letters += 1;
                    if after_trigger {
                        raw_spans += 1;
                        after_trigger = false;
                    }
                }
            } else {
                after_trigger = matches!(c, '/' | '@' | '!') && t.edit == EditAction::Pass;
            }
        }
        // Only composed letters spell the labelled words.
        let spelled: String = s.words.iter().flat_map(|w| w.raw.chars()).collect();
        assert_eq!(letters_append, spelled);
        typed_words += s.words.len();
    }
    assert!(
        raw_letters > 100 && raw_spans > 20,
        "{raw_letters} {raw_spans}"
    );
    assert_eq!(typed_words, 80 * 150);
}
