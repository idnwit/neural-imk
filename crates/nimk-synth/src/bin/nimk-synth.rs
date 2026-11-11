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
use nimk_storage::{LayeredStorage, MemStorage, PlainFsStorage, ReadOnlyDirBlobs, Storage};
use nimk_synth::{
    Corpus, MixProfile, PretrainOptions, Script, SynthConfig, TuneOptions, TuneOutput,
    evaluate_snapshot, finetune, generate, held_out, pretrain, probe_word, scripts_from_log, tune,
    tune_pending, write_log,
};
use nimk_types::Lang;

const USAGE: &str = "usage:
  nimk-synth gen --out DIR [--sessions N] [--words N] [--seed S]
  nimk-synth pretrain --model linear_action|gru_action --out DIR [--sessions N] [--words N] [--seed S]
                    [--epochs N] [--eval-frac F] [--batch N]
  nimk-synth eval --model linear_action|gru_action --init DIR [--margins 0,0.05,0.2]
                  [--sessions N] [--words N] [--seed S] [--eval-frac F]
                  (DIR = a pretrain --out directory; same seed/sessions/words/eval-frac
                   as the pretrain call = the same held-out scripts)
  nimk-synth logeval --model linear_action|gru_action --init DIR --log DATA_DIR
                     [--train-frac 0.8] [--epochs N] [--batch N] [--mix N] [--seed S]
                     (chronological split of a real event log: the snapshot in DIR
                      is scored on the held-out tail, fine-tuned on the head, and
                      scored again; --mix N adds N synthetic sessions to the head)
  nimk-synth tune status|run --data DIR [--models DIR] [--model linear_action|gru_action]
                  [--min-words N] [--direct]
                  (status: new words since the last run. run: fine-tune on them and leave
                   the result for the IME to install; --direct writes the checkpoint
                   itself, only when the IME is not running. DIR = the IME data dir,
                   --models = its bundled models dir)";

/// Parsed command line.
#[derive(Debug)]
enum Command {
    Gen(Common),
    Pretrain(Common, ModelKind, PretrainOptions),
    /// Evaluates an existing snapshot at each hysteresis margin.
    Eval(Common, ModelKind, PretrainOptions, Vec<f32>),
    /// Scores and fine-tunes a snapshot on a real log's chronological split.
    LogEval(LogEval),
    /// Counts the new data, or fine-tunes on it.
    Tune(TuneArgs),
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

/// Arguments of `logeval`.
#[derive(Debug)]
struct LogEval {
    model: ModelKind,
    init: PathBuf,
    log: PathBuf,
    train_frac: f64,
    opts: PretrainOptions,
    mix: usize,
    seed: u64,
}

fn parse_logeval(mut args: impl Iterator<Item = String>) -> Result<LogEval, String> {
    let (mut model, mut init, mut log) = (None, None, None);
    let mut e = LogEval {
        model: ModelKind::GruAction,
        init: PathBuf::new(),
        log: PathBuf::new(),
        train_frac: 0.8,
        opts: PretrainOptions {
            epochs: 3,
            ..PretrainOptions::default()
        },
        mix: 0,
        seed: 1,
    };
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--model" => {
                let name: String = value(&flag, args.next())?;
                model = Some(ModelKind::from_str(&name).map_err(|e| e.to_string())?);
            }
            "--init" => init = Some(PathBuf::from(value::<String>(&flag, args.next())?)),
            "--log" => log = Some(PathBuf::from(value::<String>(&flag, args.next())?)),
            "--train-frac" => e.train_frac = value(&flag, args.next())?,
            "--epochs" => e.opts.epochs = value(&flag, args.next())?,
            "--batch" => e.opts.batch = value(&flag, args.next())?,
            "--mix" => e.mix = value(&flag, args.next())?,
            "--seed" => e.seed = value(&flag, args.next())?,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if !(0.0..1.0).contains(&e.train_frac) {
        return Err("--train-frac must be in [0, 1)".into());
    }
    e.model = model.ok_or("--model is required")?;
    e.init = init.ok_or("--init DIR is required")?;
    e.log = log.ok_or("--log DATA_DIR is required")?;
    Ok(e)
}

/// Arguments of `tune`.
#[derive(Debug)]
struct TuneArgs {
    run: bool,
    data: PathBuf,
    models: Option<PathBuf>,
    model: ModelKind,
    min_words: u64,
    direct: bool,
}

fn parse_tune(mut args: impl Iterator<Item = String>) -> Result<TuneArgs, String> {
    let run = match args.next().as_deref() {
        Some("status") => false,
        Some("run") => true,
        other => return Err(format!("tune needs status or run, got {other:?}")),
    };
    let mut t = TuneArgs {
        run,
        data: PathBuf::new(),
        models: None,
        model: ModelKind::GruAction,
        min_words: TuneOptions::default().min_new_words,
        direct: false,
    };
    let mut data = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--data" => data = Some(PathBuf::from(value::<String>(&flag, args.next())?)),
            "--models" => t.models = Some(PathBuf::from(value::<String>(&flag, args.next())?)),
            "--model" => {
                let name: String = value(&flag, args.next())?;
                t.model = ModelKind::from_str(&name).map_err(|e| e.to_string())?;
            }
            "--min-words" => t.min_words = value(&flag, args.next())?,
            "--direct" => t.direct = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    t.data = data.ok_or("--data DIR is required")?;
    Ok(t)
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| {
            u64::try_from(d.as_millis()).map_or(u64::MAX, |ms| ms)
        })
}

/// "never", "3 hours ago", "2 days ago".
fn ago(last_ms: Option<u64>) -> String {
    let Some(last) = last_ms else {
        return "never".to_owned();
    };
    let mins = now_unix_ms().saturating_sub(last) / 60_000;
    match mins {
        0..=59 => format!("{mins} min ago"),
        60..=2879 => format!("{} hours ago", mins / 60),
        _ => format!("{} days ago", mins / 1440),
    }
}

fn run_tune(a: &TuneArgs) -> Result<(), String> {
    let user =
        PlainFsStorage::open(&a.data).map_err(|e| format!("open {}: {e}", a.data.display()))?;
    let storage: Arc<dyn Storage> = match &a.models {
        Some(dir) => {
            let defaults =
                ReadOnlyDirBlobs::open(dir).map_err(|e| format!("open {}: {e}", dir.display()))?;
            Arc::new(LayeredStorage::new(user, defaults))
        }
        None => Arc::new(user),
    };
    let p = tune_pending(storage.as_ref()).map_err(|e| format!("count new data: {e}"))?;
    println!(
        "new data: {} words ({} corrections) of {} in the log; {} run(s), last {}",
        p.new_words,
        p.new_corrections,
        p.total_words,
        p.runs,
        ago(p.last_unix_ms)
    );
    if !a.run {
        return Ok(());
    }
    let started = Instant::now();
    let opts = TuneOptions {
        min_new_words: a.min_words,
        ..TuneOptions::default()
    };
    let output = if a.direct {
        TuneOutput::Latest
    } else {
        TuneOutput::Result
    };
    let r = tune(&storage, a.model, opts, now_unix_ms(), output)
        .map_err(|e| format!("tune {:?}: {e}", a.model))?;
    println!(
        "tuned {:?} on {} new + {} replayed words ({} keys) in {:.1}s; {}",
        a.model,
        r.new_words,
        r.replay_words,
        r.keys,
        started.elapsed().as_secs_f64(),
        if a.direct {
            "checkpoint written"
        } else {
            "result left for the IME to install"
        }
    );
    Ok(())
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    let sub = args.next().ok_or("missing subcommand")?;
    if sub == "tune" {
        return parse_tune(args).map(Command::Tune);
    }
    if sub == "logeval" {
        return parse_logeval(args).map(Command::LogEval);
    }
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

fn print_row(label: &str, e: &nimk_synth::EvalReport) {
    println!(
        "{label:<10} {:<8} {:<13.4} {:<11.4} {:<14.4} {:<13.4} {:.4}",
        e.words,
        e.mid_word_accuracy(),
        e.flips_per_word(),
        e.render_accuracy(),
        e.ambiguous_accuracy(),
        e.edit_accuracy()
    );
}

/// Copies the model snapshot of `kind` from `src` into a fresh in-memory store.
fn copy_snapshot(src: &dyn Storage, kind: ModelKind) -> Result<MemStorage, String> {
    let key = format!("model/{}/latest", kind.name());
    let bytes = src
        .blobs()
        .get(&key)
        .map_err(|e| format!("read {key}: {e}"))?
        .ok_or_else(|| format!("no {key} in the init directory"))?;
    let mem = MemStorage::new();
    mem.blobs()
        .put(&key, &bytes)
        .map_err(|e| format!("copy {key}: {e}"))?;
    Ok(mem)
}

fn run_logeval(a: &LogEval) -> Result<(), String> {
    let started = Instant::now();
    let log = PlainFsStorage::open(&a.log).map_err(|e| format!("open {}: {e}", a.log.display()))?;
    let source = log
        .event_source()
        .map_err(|e| format!("event source: {e}"))?;
    let read = scripts_from_log(source.as_ref()).map_err(|e| format!("read log: {e}"))?;
    let words: usize = read.scripts.iter().map(|s| s.words.len()).sum();
    println!(
        "log: {} sessions -> {} scripts, {words} words ({} words dropped: {} no word id, {} no language; {} malformed lines)",
        read.sessions,
        read.scripts.len(),
        read.dropped_no_id + read.dropped_no_lang,
        read.dropped_no_id,
        read.dropped_no_lang,
        read.malformed
    );
    // Split by words, not scripts: a few long scripts would otherwise decide
    // the whole split. The cut falls at a script boundary.
    let mut seen = 0usize;
    let cut = read
        .scripts
        .iter()
        .position(|s| {
            seen += s.words.len();
            seen as f64 >= words as f64 * a.train_frac
        })
        .map_or(read.scripts.len(), |i| i + 1);
    let (head, tail) = read.scripts.split_at(cut);
    let held: Vec<Script> = tail.to_vec();
    let head_words: usize = head.iter().map(|s| s.words.len()).sum();
    let tail_words: usize = held.iter().map(|s| s.words.len()).sum();
    println!(
        "train: {} scripts, {head_words} words; held-out: {} scripts, {tail_words} words",
        head.len(),
        held.len()
    );
    let init = PlainFsStorage::open(&a.init).map_err(|e| format!("open init: {e}"))?;
    let cfg = EngineConfig {
        policy: a.model,
        ..EngineConfig::default()
    };
    println!(
        "row        words    mid_word_acc  flips/word  render@commit  ambiguous_acc  edit_acc"
    );
    let base: Arc<dyn Storage> = Arc::new(copy_snapshot(&init, a.model)?);
    let e = evaluate_snapshot(&held, cfg.clone(), a.opts, &base)
        .map_err(|e| format!("eval base: {e}"))?;
    print_row("base", &e);

    let mut train: Vec<Script> = head.to_vec();
    if a.mix > 0 {
        let synth = SynthConfig {
            seed: a.seed,
            sessions: a.mix,
            ..SynthConfig::default()
        };
        // Interleave: one synthetic script after each user script (cycling).
        let extra = generate(&synth, &Corpus::builtin());
        let mut mixed = Vec::with_capacity(train.len() + extra.len());
        let mut extra = extra.into_iter().cycle();
        for s in train.drain(..) {
            mixed.push(s);
            if let Some(x) = extra.next() {
                mixed.push(x);
            }
        }
        train = mixed;
    }
    let tuned: Arc<dyn Storage> = Arc::new(copy_snapshot(&init, a.model)?);
    let keys = finetune(&train, cfg.clone(), a.opts, Arc::clone(&tuned))
        .map_err(|e| format!("finetune: {e}"))?;
    let e =
        evaluate_snapshot(&held, cfg, a.opts, &tuned).map_err(|e| format!("eval tuned: {e}"))?;
    print_row("finetuned", &e);
    println!(
        "fine-tuned on {keys} keys in {:.1}s",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn run(cmd: Command) -> Result<(), String> {
    let started = Instant::now();
    let (common, pre) = match cmd {
        Command::Gen(c) => (c, None),
        Command::Pretrain(c, m, o) => (c, Some((m, o))),
        Command::Eval(c, m, o, margins) => return run_eval(&c, m, o, &margins),
        Command::LogEval(a) => return run_logeval(&a),
        Command::Tune(a) => return run_tune(&a),
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
