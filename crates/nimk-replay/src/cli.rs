//! Command-line parsing for the `nimk-replay` binary (std only).

use std::ffi::OsString;
use std::path::PathBuf;
use std::str::FromStr;

use nimk_policy::ModelKind;

use crate::ReplayConfig;

/// Usage text.
pub const USAGE: &str = "usage: nimk-replay --data DIR [--model linear_action,gru_action] \
[--from SEGMENT_PREFIX] [--report report.json] [--seed N] [--init DIR] [--frozen]";

/// Parsed arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// `PlainFsStorage` root (contains `events/`).
    pub data: PathBuf,
    /// Where to write `report.json`, if anywhere.
    pub report: Option<PathBuf>,
    /// Replay settings.
    pub replay: ReplayConfig,
}

/// Outcome of parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Print [`USAGE`].
    Help,
    /// Run the replay.
    Run(Args),
}

/// Argument errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CliError {
    /// Unknown flag or stray positional argument.
    #[error("unexpected argument `{0}`")]
    Unexpected(String),
    /// A flag without its value.
    #[error("`{0}` needs a value")]
    MissingValue(String),
    /// A value that does not parse.
    #[error("invalid value for `{flag}`: {reason}")]
    Invalid {
        /// The flag.
        flag: String,
        /// Why.
        reason: String,
    },
    /// `--data` was not given.
    #[error("`--data` is required")]
    NoData,
}

fn utf8(flag: &str, v: OsString) -> Result<String, CliError> {
    v.into_string().map_err(|_| CliError::Invalid {
        flag: flag.to_owned(),
        reason: "not UTF-8".to_owned(),
    })
}

fn parse_models(flag: &str, list: &str) -> Result<Vec<ModelKind>, CliError> {
    let mut out = Vec::new();
    for name in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let kind = ModelKind::from_str(name).map_err(|e| CliError::Invalid {
            flag: flag.to_owned(),
            reason: e.to_string(),
        })?;
        if !out.contains(&kind) {
            out.push(kind);
        }
    }
    if out.is_empty() {
        return Err(CliError::Invalid {
            flag: flag.to_owned(),
            reason: "empty model list".to_owned(),
        });
    }
    Ok(out)
}

/// Parses arguments (without the program name). Flags accept `--flag value`
/// and `--flag=value`.
///
/// # Errors
/// Fails on unknown flags, missing values or invalid values.
pub fn parse<I>(args: I) -> Result<Command, CliError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut data = None;
    let mut report = None;
    let mut replay = ReplayConfig {
        models: vec![ModelKind::LinearAction, ModelKind::GruAction],
        from: None,
        seed: None,
        init: None,
        frozen: false,
    };
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let arg = utf8("argument", arg)?;
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_owned(), Some(OsString::from(v))),
            _ => (arg.clone(), None),
        };
        if flag == "-h" || flag == "--help" {
            return Ok(Command::Help);
        }
        if flag == "--frozen" && inline.is_none() {
            replay.frozen = true;
            continue;
        }
        if !matches!(
            flag.as_str(),
            "--data" | "--model" | "--from" | "--report" | "--seed" | "--init"
        ) {
            return Err(CliError::Unexpected(arg));
        }
        let value = inline
            .or_else(|| it.next())
            .ok_or_else(|| CliError::MissingValue(flag.clone()))?;
        match flag.as_str() {
            "--data" => data = Some(PathBuf::from(value)),
            "--report" => report = Some(PathBuf::from(value)),
            "--init" => replay.init = Some(PathBuf::from(value)),
            "--model" => replay.models = parse_models(&flag, &utf8(&flag, value)?)?,
            "--from" => replay.from = Some(utf8(&flag, value)?),
            _ => {
                let v = utf8(&flag, value)?;
                let seed = v.parse::<u64>().map_err(|e| CliError::Invalid {
                    flag: flag.clone(),
                    reason: e.to_string(),
                })?;
                replay.seed = Some(seed);
            }
        }
    }
    let data = data.ok_or(CliError::NoData)?;
    Ok(Command::Run(Args {
        data,
        report,
        replay,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Command, CliError> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn parses_all_flags() {
        let cmd = p(&[
            "--data",
            "/d",
            "--model=gru_action,linear_action,gru_action",
            "--from",
            "2026-11-01",
            "--report",
            "r.json",
            "--seed=7",
            "--init=/m",
            "--frozen",
        ])
        .unwrap();
        let Command::Run(a) = cmd else {
            panic!("expected run");
        };
        assert_eq!(a.data, PathBuf::from("/d"));
        assert_eq!(a.report, Some(PathBuf::from("r.json")));
        assert_eq!(
            a.replay.models,
            [ModelKind::GruAction, ModelKind::LinearAction]
        );
        assert_eq!(a.replay.from.as_deref(), Some("2026-11-01"));
        assert_eq!(a.replay.seed, Some(7));
        assert_eq!(a.replay.init, Some(PathBuf::from("/m")));
        assert!(a.replay.frozen);
    }

    #[test]
    fn defaults_and_errors() {
        let Command::Run(a) = p(&["--data", "x"]).unwrap() else {
            panic!("expected run");
        };
        assert_eq!(a.replay.models.len(), 2);
        assert!(!a.replay.frozen);
        assert_eq!(p(&["--help"]).unwrap(), Command::Help);
        assert_eq!(p(&[]).unwrap_err(), CliError::NoData);
        assert!(matches!(p(&["--data"]), Err(CliError::MissingValue(_))));
        assert!(matches!(p(&["--bogus"]), Err(CliError::Unexpected(_))));
        assert!(matches!(
            p(&["--data", "x", "--model", "gpt"]),
            Err(CliError::Invalid { .. })
        ));
        assert!(matches!(
            p(&["--data", "x", "--seed", "-1"]),
            Err(CliError::Invalid { .. })
        ));
    }
}
