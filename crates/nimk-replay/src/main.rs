//! `nimk-replay`: replays logged sessions through one or more action models and
//! reports metrics (see docs/05-storage-and-log.md and the library docs).
#![forbid(unsafe_code)]
// The report is printed to stdout by design.
#![allow(clippy::print_stdout)]

use std::process::ExitCode;

use nimk_replay::cli::{self, Command, USAGE};
use nimk_replay::{render_table, replay_dir};

fn main() -> ExitCode {
    let args = match cli::parse(std::env::args_os().skip(1)) {
        Ok(Command::Help) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Run(args)) => args,
        Err(e) => {
            eprintln!("nimk-replay: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let report = match replay_dir(&args.data, &args.replay) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("nimk-replay: {e}");
            return ExitCode::FAILURE;
        }
    };
    print!("{}", render_table(&report));
    for w in &report.meta.warnings {
        eprintln!("warning: {w}");
    }
    if let Some(path) = &args.report {
        let written = serde_json::to_vec_pretty(&report)
            .map_err(std::io::Error::from)
            .and_then(|json| {
                // arch-lint: allow(no-sync-io) reason="synchronous CLI, no async runtime"
                std::fs::write(path, json)
            });
        if let Err(e) = written {
            eprintln!("nimk-replay: cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
