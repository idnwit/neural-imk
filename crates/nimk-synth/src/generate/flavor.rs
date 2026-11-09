//! App-flavoured lines: shell command lines and code/comment lines typed
//! with punctuation keys, for terminal and IDE sessions.

use nimk_types::Lang;

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

    /// A shell or coding-assistant input line. Triggers (`/ @ !`) only start
    /// raw mode when nothing is marked (ADR 0012), so slash commands and
    /// mentions at the line start are raw while the same characters after a
    /// word and a space are not.
    fn command_line(&mut self, mix: &MixProfile, corpus: &Corpus) {
        let r = self.rng.unit();
        if r < 0.15 && !corpus.slashes.is_empty() {
            // `/help`, optionally followed by chat words.
            let c = self.choose_str(&corpus.slashes);
            self.type_text(&format!("/{c}"), mix.p_typo);
            if self.rng.chance(0.4) {
                self.boundary(' ');
                let len = 2 + self.rng.below(4);
                let room = self.limit.saturating_sub(self.words.len());
                self.sentence(mix, corpus, len.min(room), false);
            }
        } else if r < 0.27 {
            // `@src/lib.rs` mention, then words.
            let d = self.choose_str(&corpus.dirs);
            let f = self.choose_str(&corpus.files);
            self.type_text(&format!("@{d}/{f}"), mix.p_typo);
            self.boundary(' ');
            let len = 2 + self.rng.below(5);
            let room = self.limit.saturating_sub(self.words.len());
            self.sentence(mix, corpus, len.min(room), false);
        } else if r < 0.37 {
            // `!ls -la`
            let c = self.choose_str(&corpus.cmds);
            self.type_text(&format!("!{c}"), mix.p_typo);
            if self.rng.chance(0.5) {
                self.boundary(' ');
                let f = self.choose_str(&corpus.flags);
                self.type_text(&f, mix.p_typo);
            }
        } else if r < 0.47 {
            // `cd ~/Docs`
            let d = self.choose_str(&corpus.dirs);
            self.type_text(&format!("cd ~/{d}"), mix.p_typo);
        } else {
            self.shell_command(mix, corpus);
        }
    }

    /// `git commit -m ./src/main`-shaped line.
    fn shell_command(&mut self, mix: &MixProfile, corpus: &Corpus) {
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

    /// Like [`Self::choose`], empty when the list is.
    fn choose_str(&mut self, list: &[String]) -> String {
        let i = self.rng.below(list.len().max(1));
        list.get(i).map_or_else(String::new, Clone::clone)
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
        // Letters typed in raw (command) mode are passed through: no word.
        let is_word = !self.track.is_raw();
        self.type_word(&raw, Lang::En, p_typo);
        if is_word {
            self.words.push(ScriptWord {
                word_id: self.words.len() as u64,
                lang: Lang::En,
                text: raw.clone(),
                raw,
            });
        }
    }

    /// A boundary key; its edit comes from the state machine.
    pub(super) fn boundary(&mut self, c: char) {
        self.key(c, 0);
    }
}
