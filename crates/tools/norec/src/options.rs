// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The command line.

use std::path::PathBuf;
use std::time::Duration;

use crate::error::Error;

/// What the tool does when asked.
pub(crate) const USAGE: &str = "\
usage: norec [options]

  --seed N        seed of the first case (default 1)
  --runs N        how many cases to run (default 100)
  --sqlite PATH   the sqlite3 shell (default research/sqlite/sqlite3,
                  or $NOREC_SQLITE)
  --timeout SEC   how long one case may take (default 10)
  --reports DIR   where a finding is written (default target/norec)
  --reduce N      engine runs a finding may be shrunk with (default 400;
                  0 reports the case as generated)
  --stop          stop at the first finding
  --script        print the script of each case instead of running it
  --verbose       report every case, not only the findings
  --help          this text

The seed of a case is the first seed plus its number, so `--seed N --runs 1`
runs exactly the case that was reported as N.
";

/// The environment variable that names the shell.
pub(crate) const PROGRAM_VARIABLE: &str = "NOREC_SQLITE";

/// Where the shell built by `sh tools/sqlite.sh` lands.
const DEFAULT_PROGRAM: &str = "research/sqlite/sqlite3";

/// What a run does with the cases it generates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Run them against the engine.
    #[default]
    Run,
    /// Print the script of each, which is how a generated case is read
    /// without an engine.
    Script,
}

/// What a run was asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Options {
    /// Seed of the first case.
    pub(crate) seed: u64,
    /// How many cases to run.
    pub(crate) runs: u64,
    /// The shell to drive.
    pub(crate) program: PathBuf,
    /// How long one case may take.
    pub(crate) timeout: Duration,
    /// Where findings are written.
    pub(crate) reports: PathBuf,
    /// How many engine runs reduction may spend per finding.
    pub(crate) reduce: u32,
    /// What the run does with each case.
    pub(crate) mode: Mode,
    /// Whether to stop at the first finding.
    pub(crate) stop: bool,
    /// Whether every case is reported.
    pub(crate) verbose: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            seed: 1,
            runs: 100,
            program: std::env::var_os(PROGRAM_VARIABLE)
                .map_or_else(|| PathBuf::from(DEFAULT_PROGRAM), PathBuf::from),
            timeout: Duration::from_secs(10),
            reports: PathBuf::from("target/norec"),
            reduce: 400,
            mode: Mode::Run,
            stop: false,
            verbose: false,
        }
    }
}

/// What the command line asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Request {
    /// Print the usage and stop.
    Help,
    /// Run with these options.
    Run(Box<Options>),
}

/// Reads the command line.
pub(crate) fn parse(args: &[String]) -> Result<Request, Error> {
    let mut options = Options::default();
    let mut rest = args.iter();
    while let Some(argument) = rest.next() {
        let mut value = || {
            rest.next()
                .ok_or_else(|| Error::Usage(format!("{argument} wants a value")))
        };
        match argument.as_str() {
            "--help" | "-h" => return Ok(Request::Help),
            "--seed" => options.seed = whole(argument, value()?)?,
            "--runs" => options.runs = whole(argument, value()?)?,
            "--sqlite" => options.program = PathBuf::from(value()?),
            "--timeout" => options.timeout = Duration::from_secs(whole(argument, value()?)?),
            "--reports" => options.reports = PathBuf::from(value()?),
            "--reduce" => {
                options.reduce = u32::try_from(whole(argument, value()?)?)
                    .map_err(|_| Error::Usage("--reduce is too large".to_owned()))?;
            }
            "--stop" => options.stop = true,
            "--script" => options.mode = Mode::Script,
            "--verbose" => options.verbose = true,
            other => return Err(Error::Usage(format!("unknown option `{other}`"))),
        }
    }
    Ok(Request::Run(Box::new(options)))
}

/// A whole number, or a usage error naming the option it belongs to.
fn whole(option: &str, text: &str) -> Result<u64, Error> {
    text.parse::<u64>()
        .map_err(|_| Error::Usage(format!("{option} wants a whole number, got `{text}`")))
}
