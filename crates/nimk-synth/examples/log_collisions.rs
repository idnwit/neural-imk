//! Aggregate collision statistics over real event logs (no words printed).
//!
//! Usage: `cargo run -p nimk-synth --example log_collisions -- <events dir>`
//! Prints counts only: how many committed words are EN with a valid Telex
//! Vietnamese rendering ("collisions") and how many VI words have raw keys
//! that are English corpus words.
#![allow(clippy::print_stdout, clippy::expect_used, missing_docs)]

use std::collections::{HashMap, HashSet};

use nimk_synth::Corpus;
use nimk_types::{Event, Lang, Record, Target};

fn main() {
    let dir = std::env::args().nth(1).expect("events dir");
    let corpus = Corpus::builtin();
    let en: HashSet<&str> = corpus
        .en
        .iter()
        .chain(&corpus.en_ambiguous)
        .map(String::as_str)
        .collect();
    let (mut n, mut en_n, mut en_coll, mut vi_n, mut vi_en_word, mut vi_changed) =
        (0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .expect("dir")
        .flatten()
        .map(|e| e.path())
        .collect();
    paths.sort();
    for p in paths {
        let text = std::fs::read_to_string(p).unwrap_or_default();
        let mut commits: HashMap<(String, u64), (String, String, Lang)> = HashMap::new();
        let mut labels: HashMap<(String, u64), Lang> = HashMap::new();
        for rec in text
            .lines()
            .filter_map(|l| serde_json::from_str::<Record>(l).ok())
        {
            match rec.event {
                Event::WordCommit {
                    word_id,
                    raw,
                    vi,
                    mode,
                    ..
                } => {
                    commits.insert((rec.session.clone(), word_id), (raw, vi, mode));
                }
                Event::Label {
                    target: Target::Word(id),
                    label,
                    ..
                } => {
                    labels.insert((rec.session.clone(), id), label);
                }
                _ => {}
            }
        }
        for (k, (raw, vi, mode)) in commits {
            let lang = labels.get(&k).copied().unwrap_or(mode);
            n += 1;
            let t = nimk_telex::transform(&raw);
            let coll = t.has_telex && t.valid_vi && vi != raw;
            match lang {
                Lang::En => {
                    en_n += 1;
                    en_coll += u32::from(coll);
                }
                Lang::Vi => {
                    vi_n += 1;
                    vi_en_word += u32::from(en.contains(raw.to_lowercase().as_str()));
                    vi_changed += u32::from(vi != raw);
                }
            }
        }
    }
    println!("words {n}  en {en_n}  vi {vi_n}");
    println!("en words with valid-Telex VI rendering (collision): {en_coll}");
    println!("vi words whose raw keys are corpus-English: {vi_en_word}");
    println!("vi words changed by Telex: {vi_changed}");
}
