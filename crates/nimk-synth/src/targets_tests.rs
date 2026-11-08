use nimk_types::{EditAction, Lang, Modifiers};

use super::*;

fn cfg(seed: u64, p_typo: f32, p_shortcut: f32) -> SynthConfig {
    SynthConfig {
        seed,
        sessions: 3,
        words_per_session: 200,
        mix: MixProfile {
            p_typo,
            p_shortcut,
            ..MixProfile::default()
        },
        ..SynthConfig::default()
    }
}

/// Replays targets over keys: returns committed (text, lang) per word.
fn replay(s: &Script) -> Vec<(String, Lang)> {
    let mut buf = String::new();
    let mut out = Vec::new();
    for (k, t) in s.keys.iter().zip(&s.targets) {
        match t.edit {
            EditAction::Append => buf.push_str(&k.chars),
            EditAction::Pop => {
                buf.pop();
            }
            EditAction::Commit => out.push((std::mem::take(&mut buf), t.render)),
            EditAction::Pass => assert!(buf.is_empty(), "Pass with non-empty buffer"),
        }
    }
    assert!(buf.is_empty());
    out
}

#[test]
fn one_target_per_key_and_words_commit_once() {
    for (p_typo, p_sc) in [(0.0, 0.0), (0.1, 0.2)] {
        for s in generate(&cfg(1, p_typo, p_sc), &Corpus::builtin()) {
            assert_eq!(s.targets.len(), s.keys.len());
            let committed = replay(&s);
            assert_eq!(committed.len(), s.words.len(), "one Commit per word");
            for ((text, lang), w) in committed.iter().zip(&s.words) {
                assert_eq!(text, &w.raw);
                assert_eq!(*lang, w.lang);
            }
        }
    }
}

#[test]
fn append_count_equals_raw_length_without_typos() {
    for s in generate(&cfg(2, 0.0, 0.0), &Corpus::builtin()) {
        let appends = s
            .targets
            .iter()
            .filter(|t| t.edit == EditAction::Append)
            .count();
        let raw: usize = s.words.iter().map(|w| w.raw.chars().count()).sum();
        assert_eq!(appends, raw);
        assert!(s.targets.iter().all(|t| t.edit != EditAction::Pop));
    }
}

#[test]
fn boundary_targets_follow_rules() {
    for s in generate(&cfg(3, 0.0, 0.0), &Corpus::builtin()) {
        let mut in_word = false;
        let mut prev = Lang::Vi;
        for (k, t) in s.keys.iter().zip(&s.targets) {
            match t.edit {
                EditAction::Append => {
                    in_word = true;
                    prev = t.render;
                }
                EditAction::Commit => {
                    assert!(in_word);
                    assert_eq!(t.render, prev);
                    in_word = false;
                }
                EditAction::Pass => {
                    assert!(!in_word, "{:?}", k.chars);
                    assert_eq!(t.render, prev, "sticky render");
                }
                EditAction::Pop => unreachable!("typos are off"),
            }
        }
    }
}

#[test]
fn typos_pop_after_wrong_letter() {
    let scripts = generate(&cfg(4, 0.15, 0.0), &Corpus::builtin());
    let mut pops = 0;
    for s in &scripts {
        for (i, t) in s.targets.iter().enumerate() {
            if t.edit == EditAction::Pop {
                pops += 1;
                assert!(i > 0 && s.targets[i - 1].edit == EditAction::Append);
                assert_eq!(s.keys[i].keycode, nimk_types::keycode::BACKSPACE);
                assert_eq!(t.render, s.targets[i - 1].render);
            }
        }
    }
    assert!(pops > 0);
}

#[test]
fn shortcuts_are_cmd_pass() {
    let mut n = 0;
    for s in generate(&cfg(5, 0.0, 0.3), &Corpus::builtin()) {
        for (k, t) in s.keys.iter().zip(&s.targets) {
            if k.mods.has(Modifiers::COMMAND) {
                n += 1;
                assert_eq!(t.edit, EditAction::Pass);
            }
        }
    }
    assert!(n > 0);
}

#[test]
fn realism_off_is_default_and_deterministic() {
    let corpus = Corpus::builtin();
    let off = generate(&cfg(6, 0.0, 0.0), &corpus);
    let default = generate(
        &SynthConfig {
            seed: 6,
            sessions: 3,
            words_per_session: 200,
            ..SynthConfig::default()
        },
        &corpus,
    );
    assert_eq!(off, default);
    assert_eq!(
        generate(&cfg(7, 0.1, 0.1), &corpus),
        generate(&cfg(7, 0.1, 0.1), &corpus)
    );
    // Keys spell raw words and boundaries only (no extra keys when off).
    for s in &off {
        assert!(s.keys.iter().all(|k| !k.mods.is_shortcut()));
    }
}
