//! App-flavoured lines: shell command lines and code/comment lines typed
//! with punctuation keys, for terminal and IDE sessions.

use nimk_types::{EditAction, Lang};

use super::{Flavor, MixProfile, ScriptWord, Writer};
use crate::Corpus;

impl Writer {
    /// One command (terminal) or code/comment (IDE) line ending with Return.
    pub(super) fn line(&mut self, flavor: Flavor, mix: &MixProfile, corpus: &Corpus) {
        match flavor {
            Flavor::Terminal => self.command_line(mix, corpus),
            Flavor::Ide => self.code_line(mix, corpus),
            Flavor::Chat => {}
        }
        self.boundary('\r');
    }

    /// `git commit -m ./src/main`-shaped line.
    fn command_line(&mut self, mix: &MixProfile, corpus: &Corpus) {
        let mut parts: Vec<String> = Vec::new();
        parts.extend(self.choose(&corpus.cmds));
        if self.rng.chance(0.8) {
            parts.extend(self.choose(&corpus.subs));
        }
        for _ in 0..2 {
            if self.rng.chance(0.35) {
                parts.extend(self.choose(&corpus.flags));
            }
        }
        if self.rng.chance(0.5) {
            let depth = 1 + self.rng.below(3);
            let mut path = String::from(if self.rng.chance(0.5) { "./" } else { "/" });
            for d in 0..depth {
                if d > 0 {
                    path.push('/');
                }
                path.extend(self.choose(&corpus.dirs));
            }
            parts.push(path);
        }
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                self.boundary(' ');
            }
            self.type_text(p, mix.p_typo);
        }
    }

    /// A comment (chat words after `// `) or an identifier expression.
    fn code_line(&mut self, mix: &MixProfile, corpus: &Corpus) {
        match self.rng.below(3) {
            0 => {
                self.type_text("// ", mix.p_typo);
                let len = 3 + self.rng.below(6);
                let room = self.limit.saturating_sub(self.words.len());
                self.sentence(mix, corpus, len.min(room), false);
            }
            1 => {
                let a = self.ident(mix, corpus);
                let b = self.ident(mix, corpus);
                let c = self.ident(mix, corpus);
                self.type_text(&format!("{a}_{b}({c});"), mix.p_typo);
            }
            _ => {
                let a = self.ident(mix, corpus);
                let b = self.ident(mix, corpus);
                let c = self.ident(mix, corpus);
                let d = self.ident(mix, corpus);
                self.type_text(&format!("self.{a}.{b}({c}, {d})"), mix.p_typo);
            }
        }
    }

    /// An English word to use inside an identifier.
    fn ident(&mut self, mix: &MixProfile, corpus: &Corpus) -> String {
        self.pick_in(Lang::En, mix, corpus)
            .map_or_else(|| "x".to_owned(), |(_, w)| w)
    }

    fn choose(&mut self, list: &[String]) -> Option<String> {
        if list.is_empty() {
            return None;
        }
        list.get(self.rng.below(list.len())).cloned()
    }

    /// Types `s`: letter runs are English words, every other character is a
    /// boundary key. Leaves a trailing word's buffer open. Letters beyond the
    /// session's word limit are dropped.
    fn type_text(&mut self, s: &str, p_typo: f32) {
        let mut cur = String::new();
        for c in s.chars() {
            if c.is_ascii_alphabetic() {
                cur.push(c);
            } else {
                self.flush_word(&mut cur, p_typo);
                self.boundary(c);
            }
        }
        self.flush_word(&mut cur, p_typo);
    }

    fn flush_word(&mut self, cur: &mut String, p_typo: f32) {
        let raw = std::mem::take(cur);
        if raw.is_empty() || self.words.len() >= self.limit {
            return;
        }
        self.type_word(&raw, Lang::En, p_typo);
        self.words.push(ScriptWord {
            word_id: self.words.len() as u64,
            lang: Lang::En,
            text: raw.clone(),
            raw,
        });
    }

    /// A boundary key: `Commit` when it ends a word, `Pass` otherwise.
    pub(super) fn boundary(&mut self, c: char) {
        let edit = if self.buf_open {
            EditAction::Commit
        } else {
            EditAction::Pass
        };
        self.key(c, 0, edit);
    }
}
