//! `nimk-synth gen --out DIR [--sessions N] [--words N] [--seed S]` writes a
//! synthetic event log; `nimk-synth pretrain --model KIND --out DIR [--epochs N] [--eval-frac F] [...]`
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
use nimk_policy::ModelKind;
use nimk_storage::{PlainFsStorage, Storage};
use nimk_synth::{
    Corpus, MixProfile, PretrainOptions, SynthConfig, evaluate_snapshot, generate, held_out,
    pretrain, probe_word, write_log,
};
use nimk_types::Lang;

const USAGE: &str = "usage:
  nimk-synth gen --out DIR [--sessions N] [--words N] [--seed S]
  nimk-synth pretrain --model linear_action|gru_action --out DIR [--sessions N] [--words N] [--seed S]
                    [--epochs N] [--eval-frac F] [--batch N]
  nimk-synth eval --model linear_action|gru_action --init DIR [--margins 0,0.05,0.2]
                  [--sessions N] [--words N] [--seed S] [--eval-frac F]
                  (DIR = a pretrain --out directory; same seed/sessions/words/eval-frac
                   as the pretrain call = the same held-out scripts)";

/// Parsed command line.
#[derive(Debug)]
enum Command {
    Gen(Common),
    Pretrain(Common, ModelKind, PretrainOptions),
    /// Evaluates an existing snapshot at each hysteresis margin.
    Eval(Common, ModelKind, PretrainOptions, Vec<f32>),
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
    let mut model = None;
    let mut opts = PretrainOptions::default();
    let mut synth = SynthConfig::default();
    let mut margins = vec![EngineConfig::default().switch_margin];
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--out" if sub != "eval" => {
                out = Some(PathBuf::from(value::<String>(&flag, args.next())?))
            }
            "--sessions" => synth.sessions = value(&flag, args.next())?,
            "--words" => synth.words_per_session = value(&flag, args.next())?,
            "--seed" => synth.seed = value(&flag, args.next())?,
            "--init" if sub == "eval" => {
                out = Some(PathBuf::from(value::<String>(&flag, args.next())?));
            }
            "--margins" if sub == "eval" => {
                let list: String = value(&flag, args.next())?;
                margins = list
                    .split(',')
                    .map(|m| m.parse::<f32>().map_err(|_| format!("bad margin: {m}")))
                    .collect::<Result<_, _>>()?;
            }
            "--model" if sub == "pretrain" || sub == "eval" => {
                let name: String = value(&flag, args.next())?;
                model = Some(ModelKind::from_str(&name).map_err(|e| e.to_string())?);
            }
            "--epochs" if sub == "pretrain" => opts.epochs = value(&flag, args.next())?,
            "--batch" if sub == "pretrain" => opts.batch = value(&flag, args.next())?,
            "--eval-frac" if sub == "pretrain" || sub == "eval" => {
                opts.eval_frac = value(&flag, args.next())?;
                if !(0.0..1.0).contains(&opts.eval_frac) {
                    return Err("--eval-frac must be in [0, 1)".into());
                }
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let common = Common {
        out: out.ok_or("--out DIR (--init DIR for eval) is required")?,
        synth,
    };
    match sub.as_str() {
        "gen" => Ok(Command::Gen(common)),
        "pretrain" => Ok(Command::Pretrain(
            common,
            model.ok_or("--model is required")?,
            opts,
        )),
        "eval" => Ok(Command::Eval(
            common,
            model.ok_or("--model is required")?,
            opts,
            margins,
        )),
        other => Err(format!("unknown subcommand: {other}")),
    }
}

/// Evaluates the snapshot in `common.out` on the held-out tail of the
/// generated scripts at every margin and prints one table row each.
fn run_eval(
    common: &Common,
    kind: ModelKind,
    opts: PretrainOptions,
    margins: &[f32],
) -> Result<(), String> {
    let storage: Arc<dyn Storage> = Arc::new(
        PlainFsStorage::open(&common.out)
            .map_err(|e| format!("cannot open {}: {e}", common.out.display()))?,
    );
    let scripts = generate(&common.synth, &Corpus::builtin());
    let held = held_out(&scripts, opts.eval_frac);
    println!("model {kind:?}, {} held-out scripts", held.len());
    println!("margin  mid_word_acc  flips/word  render@commit  fresh@commit  edit_acc");
    for &switch_margin in margins {
        let cfg = EngineConfig {
            policy: kind,
            switch_margin,
            ..EngineConfig::default()
        };
        let e = evaluate_snapshot(held, cfg, opts, &storage)
            .map_err(|e| format!("eval at margin {switch_margin}: {e}"))?;
        println!(
            "{switch_margin:<7} {:<13.4} {:<11.4} {:<14.4} {:<13.4} {:.4}",
            e.mid_word_accuracy(),
            e.flips_per_word(),
            e.render_accuracy(),
            e.render_fresh_accuracy(),
            e.edit_accuracy()
        );
    }
    Ok(())
}

fn run(cmd: Command) -> Result<(), String> {
    let started = Instant::now();
    let (common, pre) = match cmd {
        Command::Gen(c) => (c, None),
        Command::Pretrain(c, m, o) => (c, Some((m, o))),
        Command::Eval(c, m, o, margins) => return run_eval(&c, m, o, &margins),
    };
    let storage = PlainFsStorage::open(&common.out)
        .map_err(|e| format!("cannot open {}: {e}", common.out.display()))?;
    let scripts = generate(&common.synth, &Corpus::builtin());
    let words: usize = scripts.iter().map(|s| s.words.len()).sum();
    match pre {
        None => {
            write_log(&scripts, &storage).map_err(|e| format!("write_log: {e}"))?;
            println!(
                "wrote {} sessions, {words} words to {} in {:.2}s",
                scripts.len(),
                common.out.display(),
                started.elapsed().as_secs_f64()
            );
        }
        Some((kind, opts)) => {
            let cfg = EngineConfig {
                policy: kind,
                ..EngineConfig::default()
            };
            let storage: Arc<dyn Storage> = Arc::new(storage);
            let r = pretrain(&scripts, cfg.clone(), opts, Arc::clone(&storage))
                .map_err(|e| format!("pretrain: {e}"))?;
            // Unseen scripts where every Vietnamese word is variant-typed.
            let variant = SynthConfig {
                seed: common.synth.seed.wrapping_add(1_000_003),
                sessions: 10,
                words_per_session: 300,
                mix: MixProfile {
                    p_variant: 1.0,
                    ..common.synth.mix.clone()
                },
                ..common.synth.clone()
            };
            let v = evaluate_snapshot(
                &generate(&variant, &Corpus::builtin()),
                cfg.clone(),
                opts,
                &storage,
            )
            .map_err(|e| format!("variant eval: {e}"))?;
            println!(
                "variant_render_acc {:.4}  ({} words)",
                v.render_accuracy(),
                v.words
            );
            println!(
                "variant_ambig_acc  {:.4}  ({} words)",
                v.ambiguous_accuracy(),
                v.ambiguous
            );
            // `theme` (thêm) after Vietnamese and English context.
            let vi_ctx = [(Lang::Vi, "Tôi"), (Lang::Vi, "muốn")];
            let en_ctx = [(Lang::En, "change"), (Lang::En, "the"), (Lang::En, "dark")];
            for (name, ctx, raw) in [
                ("theme/vi-context", &vi_ctx[..], "theme"),
                ("theme/en-context", &en_ctx[..], "theme"),
                ("theem/vi-context", &vi_ctx[..], "theem"),
                ("theem/en-context", &en_ctx[..], "theem"),
            ] {
                let r = probe_word(cfg.clone(), opts, &storage, ctx, raw)
                    .map_err(|e| format!("probe: {e}"))?;
                let p = r.p_vi.map_or_else(|| "-".to_owned(), |p| format!("{p:.3}"));
                println!("probe {name:<17} p_vi {p}  chosen {:?}", r.chosen);
            }
            let e = &r.eval;
            println!("model             {kind:?}");
            println!("epochs            {}", r.epochs);
            println!("train_scripts     {}", r.train_scripts);
            println!("train_keys        {}", r.train_keys);
            println!("eval_scripts      {}", e.scripts);
            println!("eval_keys         {}", e.keys);
            println!("edit_accuracy     {:.4}", e.edit_accuracy());
            println!(
                "render_accuracy   {:.4}  ({} words)",
                e.render_accuracy(),
                e.words
            );
            println!(
                "ambiguous_acc     {:.4}  ({} words)",
                e.ambiguous_accuracy(),
                e.ambiguous
            );
            println!(
                "ambiguous_en_acc  {:.4}  ({} words)",
                e.ambiguous_en_accuracy(),
                e.ambiguous_en
            );
            println!("render_fresh_acc  {:.4}", e.render_fresh_accuracy());
            println!("mid_word_acc      {:.4}", e.mid_word_accuracy());
            println!("flips_per_word    {:.4}", e.flips_per_word());
            let pos: Vec<String> = e
                .append_accuracy_by_pos()
                .iter()
                .map(|a| format!("{a:.3}"))
                .collect();
            println!("render_by_pos     {}", pos.join(" "));
            println!("commit_p_hist vi  {:?}", e.commit_p_hist[0]);
            println!("commit_p_hist en  {:?}", e.commit_p_hist[1]);
            println!("text_match_rate   {:.4}", e.text_match_rate());
            println!("script_exact_rate {:.4}", e.script_exact_rate());
            println!("wall_secs         {:.2}", started.elapsed().as_secs_f64());
            println!("snapshot          {}", common.out.display());
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
            "pretrain --model linear_action --out /x --sessions 3 --words 7 --seed 9 --epochs 3 --eval-frac 0.2",
        ));
        match cmd {
            Ok(Command::Pretrain(c, ModelKind::LinearAction, o)) => {
                assert_eq!((o.epochs, o.eval_frac), (3, 0.2));
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
    fn requires_out_and_model() {
        assert!(parse(args("gen")).is_err());
        assert!(parse(args("pretrain --out /x")).is_err());
        assert!(parse(args("gen --out /x --model linear_action")).is_err());
        assert!(parse(args("pretrain --model nope --out /x")).is_err());
        assert!(parse(args("pretrain --model gru_action --out /x --eval-frac 1")).is_err());
        assert!(parse(args("frob --out /x")).is_err());
        assert!(matches!(parse(args("gen --out /x")), Ok(Command::Gen(_))));
    }
}
