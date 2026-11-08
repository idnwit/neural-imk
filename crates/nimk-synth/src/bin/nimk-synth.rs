//! `nimk-synth gen --out DIR [--sessions N] [--words N] [--seed S]` writes a
//! synthetic event log; `nimk-synth pretrain --policy KIND --out DIR [...]`
//! pre-trains a model and stores it as the IME's starting snapshot.
//!
//! `--out` is required and never defaults to the IME's data directory: point
//! it there explicitly to install a pre-trained model.
#![forbid(unsafe_code)]
#![allow(clippy::print_stdout)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use nimk_engine::EngineConfig;
use nimk_policy::PolicyKind;
use nimk_storage::PlainFsStorage;
use nimk_synth::{Corpus, SynthConfig, generate, pretrain, write_log};

const USAGE: &str = "usage:
  nimk-synth gen --out DIR [--sessions N] [--words N] [--seed S]
  nimk-synth pretrain --policy baseline|linear|tiny_mlp --out DIR [--sessions N] [--words N] [--seed S]";

/// Parsed command line.
#[derive(Debug)]
enum Command {
    Gen(Common),
    Pretrain(Common, PolicyKind),
}

/// Options shared by both subcommands.
#[derive(Debug)]
struct Common {
    out: PathBuf,
    synth: SynthConfig,
}

fn value<T: FromStr>(flag: &str, v: Option<String>) -> Result<T, String> {
    let v = v.ok_or_else(|| format!("{flag} needs a value"))?;
    v.parse()
        .map_err(|_| format!("invalid value for {flag}: {v}"))
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let sub = args.next().ok_or("missing subcommand")?;
    let mut out = None;
    let mut policy = None;
    let mut synth = SynthConfig::default();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--out" => out = Some(PathBuf::from(value::<String>(&flag, args.next())?)),
            "--sessions" => synth.sessions = value(&flag, args.next())?,
            "--words" => synth.words_per_session = value(&flag, args.next())?,
            "--seed" => synth.seed = value(&flag, args.next())?,
            "--policy" if sub == "pretrain" => {
                let name: String = value(&flag, args.next())?;
                policy = Some(PolicyKind::from_str(&name).map_err(|e| e.to_string())?);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let common = Common {
        out: out.ok_or("--out DIR is required")?,
        synth,
    };
    match sub.as_str() {
        "gen" => Ok(Command::Gen(common)),
        "pretrain" => Ok(Command::Pretrain(
            common,
            policy.ok_or("--policy is required")?,
        )),
        other => Err(format!("unknown subcommand: {other}")),
    }
}

fn run(cmd: Command) -> Result<(), String> {
    let started = Instant::now();
    let (common, policy) = match cmd {
        Command::Gen(c) => (c, None),
        Command::Pretrain(c, p) => (c, Some(p)),
    };
    let storage = PlainFsStorage::open(&common.out)
        .map_err(|e| format!("cannot open {}: {e}", common.out.display()))?;
    let scripts = generate(&common.synth, &Corpus::builtin());
    let words: usize = scripts.iter().map(|s| s.words.len()).sum();
    match policy {
        None => {
            write_log(&scripts, &storage).map_err(|e| format!("write_log: {e}"))?;
            println!(
                "wrote {} sessions, {words} words to {} in {:.2}s",
                scripts.len(),
                common.out.display(),
                started.elapsed().as_secs_f64()
            );
        }
        Some(kind) => {
            let cfg = EngineConfig {
                policy: kind,
                ..EngineConfig::default()
            };
            let model = nimk_policy::build(kind, cfg.seed);
            let r = pretrain(&scripts, cfg, model, Arc::new(storage))
                .map_err(|e| format!("pretrain: {e}"))?;
            println!("policy         {kind:?}");
            println!("words          {}", r.words);
            println!("ambiguous      {}", r.ambiguous);
            println!("correct        {}", r.correct);
            println!("accuracy       {:.4}", r.accuracy());
            println!("head_accuracy  {:.4}", r.head_accuracy);
            println!("tail_accuracy  {:.4}", r.tail_accuracy);
            println!("wall_secs      {:.2}", started.elapsed().as_secs_f64());
            println!("snapshot       {}", common.out.display());
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let result = parse(std::env::args().skip(1)).and_then(run);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("nimk-synth: {e}\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> impl Iterator<Item = String> {
        s.split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn parses_pretrain() {
        let cmd = parse(args(
            "pretrain --policy linear --out /x --sessions 3 --words 7 --seed 9",
        ));
        match cmd {
            Ok(Command::Pretrain(c, PolicyKind::Linear)) => {
                assert_eq!(c.out, PathBuf::from("/x"));
                assert_eq!(
                    (c.synth.sessions, c.synth.words_per_session, c.synth.seed),
                    (3, 7, 9)
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn requires_out_and_policy() {
        assert!(parse(args("gen")).is_err());
        assert!(parse(args("pretrain --out /x")).is_err());
        assert!(parse(args("gen --out /x --policy linear")).is_err());
        assert!(parse(args("pretrain --policy nope --out /x")).is_err());
        assert!(parse(args("frob --out /x")).is_err());
        assert!(matches!(parse(args("gen --out /x")), Ok(Command::Gen(_))));
    }
}
