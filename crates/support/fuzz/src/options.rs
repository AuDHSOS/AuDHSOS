// SPDX-License-Identifier: AGPL-3.0-only AND Apache-2.0 WITH LLVM-exception
// Copyright (C) 2026 Manuel Baesler and contributors
// Copyright (C) the LLVM Project contributors, under Apache-2.0 WITH LLVM-exception
// Ported from LLVM's libFuzzer; see NOTICE at the root of this repository.

//! What a run was asked to do.
//!
//! The flags are libFuzzer's, spelled the way libFuzzer spells them, so
//! that what is written down about running a fuzz target keeps working and
//! so that a corpus directory can be handed to either engine. Flags this
//! engine does not have are refused by name rather than ignored: a run
//! that was asked for something it will not do should say so and stop.
//!
//! Invariant: a path on the command line is a path, and everything that
//! begins with `-` is a flag. This is libFuzzer's rule and it is why a
//! corpus directory whose name begins with a dash cannot be fuzzed.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

/// What is wrong with a command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionError {
    /// A flag this engine does not have.
    Unknown(String),
    /// A flag whose value is not a number.
    NotANumber(String),
    /// A flag this engine knows of but does not implement.
    Unsupported(String),
    /// A mode that needs paths and was given none.
    MissingPath(String),
}

impl fmt::Display for OptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(flag) => write!(f, "unknown flag `-{flag}`"),
            Self::NotANumber(flag) => write!(f, "`-{flag}` needs a number"),
            Self::Unsupported(flag) => write!(
                f,
                "`-{flag}` is a libFuzzer flag this engine does not implement"
            ),
            Self::MissingPath(mode) => write!(f, "`{mode}` needs a path"),
        }
    }
}

impl std::error::Error for OptionError {}

/// What the run is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Mutate and run, which is what a fuzzing run does.
    #[default]
    Fuzz,
    /// Run every file once and report, without mutating.
    RunOnce,
    /// Fold the later corpora into the first, keeping what adds coverage.
    Merge,
    /// Shrink one crashing input while it still crashes.
    MinimizeCrash,
}

/// Everything a run was told.
#[derive(Clone, Debug)]
pub struct Options {
    /// What the run is for.
    pub mode: Mode,
    /// The corpus directories and files named on the command line. The
    /// first is where a fuzzing run writes what it finds.
    pub paths: Vec<PathBuf>,
    /// Seconds to run for, or `None` for as long as it takes.
    pub max_total_time: Option<u64>,
    /// Runs to make, or `None` for as many as there is time for.
    pub runs: Option<u64>,
    /// The longest input the mutator may produce.
    pub max_len: usize,
    /// How reluctantly the length limit grows: the number of runs without
    /// a find, times the logarithm of the present limit, that must pass
    /// before it rises. Zero holds the limit where it starts.
    pub len_control: u64,
    /// The seed of the run, or `None` to take one from the clock.
    pub seed: Option<u64>,
    /// Seconds one input may take before the run is called hung.
    pub timeout: u64,
    /// Where a crashing input is written.
    pub artifact_prefix: PathBuf,
    /// A file of words to paste, one per line, in libFuzzer's format.
    pub dictionary: Option<PathBuf>,
    /// Whether to keep the value profile, which costs about a third of the
    /// run's speed and finds comparisons that no counter shows.
    pub value_profile: bool,
    /// Whether an input that reaches a feature in fewer bytes takes it
    /// over from the input that reached it first.
    pub reduce_inputs: bool,
    /// Whether to print the totals at the end.
    pub print_final_stats: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Fuzz,
            paths: Vec::new(),
            max_total_time: None,
            runs: None,
            max_len: DEFAULT_MAX_LEN,
            len_control: DEFAULT_LEN_CONTROL,
            seed: None,
            timeout: DEFAULT_TIMEOUT,
            artifact_prefix: PathBuf::from("./"),
            dictionary: None,
            value_profile: false,
            reduce_inputs: true,
            print_final_stats: false,
        }
    }
}

/// The longest input a run produces unless it is told otherwise.
pub const DEFAULT_MAX_LEN: usize = 4096;

/// How reluctantly the length limit grows unless a run says otherwise.
pub const DEFAULT_LEN_CONTROL: u64 = 100;

/// How long one input may take unless a run says otherwise.
pub const DEFAULT_TIMEOUT: u64 = 1200;

/// The flags libFuzzer has that this engine has not. A run that asks for
/// one of them is refused by name, because silently doing something else
/// is worse than stopping.
const UNSUPPORTED: &[&str] = &[
    "jobs",
    "workers",
    "fork",
    "detect_leaks",
    "rss_limit_mb",
    "malloc_limit_mb",
    "merge_control_file",
    "data_flow_trace",
    "collect_data_flow",
    "features_dir",
    "exact_artifact_path",
];

/// Reads a command line.
///
/// # Errors
///
/// [`OptionError`] for a flag that is unknown, unimplemented, or given
/// something other than the number it wants.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Options, OptionError> {
    let mut options = Options::default();
    for argument in args {
        let text = argument.to_string_lossy().into_owned();
        let Some(flag) = text.strip_prefix('-') else {
            options.paths.push(PathBuf::from(argument));
            continue;
        };
        let (name, value) = match flag.split_once('=') {
            Some((name, value)) => (name, value),
            None => (flag, ""),
        };
        apply(&mut options, name, value)?;
    }
    if options.mode == Mode::MinimizeCrash && options.paths.is_empty() {
        return Err(OptionError::MissingPath("-minimize_crash=1".to_owned()));
    }
    if options.mode == Mode::Merge && options.paths.len() < 2 {
        return Err(OptionError::MissingPath("-merge=1".to_owned()));
    }
    Ok(options)
}

/// Applies one flag.
fn apply(options: &mut Options, name: &str, value: &str) -> Result<(), OptionError> {
    if UNSUPPORTED.contains(&name) {
        return Err(OptionError::Unsupported(name.to_owned()));
    }
    match name {
        "max_total_time" => options.max_total_time = Some(number(name, value)?).filter(|s| *s > 0),
        "runs" => {
            let runs = number(name, value)?;
            options.runs = if runs == u64::MAX { None } else { Some(runs) };
        }
        "max_len" => options.max_len = usize::try_from(number(name, value)?).unwrap_or(usize::MAX),
        "len_control" => options.len_control = number(name, value)?,
        "seed" => options.seed = Some(number(name, value)?).filter(|s| *s > 0),
        "timeout" => options.timeout = number(name, value)?,
        "artifact_prefix" => options.artifact_prefix = PathBuf::from(value),
        "dict" => options.dictionary = Some(PathBuf::from(value)),
        "use_value_profile" => options.value_profile = flag(name, value)?,
        "reduce_inputs" => options.reduce_inputs = flag(name, value)?,
        "print_final_stats" => options.print_final_stats = flag(name, value)?,
        "runs_once" => options.mode = Mode::RunOnce,
        "merge" => {
            if flag(name, value)? {
                options.mode = Mode::Merge;
            }
        }
        "minimize_crash" => {
            if flag(name, value)? {
                options.mode = Mode::MinimizeCrash;
            }
        }
        other => return Err(OptionError::Unknown(other.to_owned())),
    }
    Ok(())
}

/// The number a flag was given.
fn number(name: &str, value: &str) -> Result<u64, OptionError> {
    if value == "-1" {
        return Ok(u64::MAX);
    }
    value
        .parse()
        .map_err(|_| OptionError::NotANumber(name.to_owned()))
}

/// The yes or no a flag was given, where anything but zero is yes.
fn flag(name: &str, value: &str) -> Result<bool, OptionError> {
    Ok(number(name, value)? != 0)
}

/// The words of a dictionary file in libFuzzer's format: one entry per
/// line, `name="value"` or just `"value"`, with `\xAB`, `\\` and `\"` as
/// the escapes, and `#` starting a comment.
#[must_use]
pub fn parse_dictionary(text: &str) -> Vec<Vec<u8>> {
    text.lines().filter_map(parse_dictionary_line).collect()
}

/// The word of one dictionary line, if it holds one.
fn parse_dictionary_line(line: &str) -> Option<Vec<u8>> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let opening = trimmed.find('"')?;
    let body = trimmed.get(opening.saturating_add(1)..)?;
    let closing = body.rfind('"')?;
    unescape(body.get(..closing)?)
}

/// The bytes a quoted dictionary value stands for.
fn unescape(body: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(body.len());
    let mut bytes = body.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        match bytes.next()? {
            b'\\' => out.push(b'\\'),
            b'"' => out.push(b'"'),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'x' => {
                let high = digit(bytes.next()?)?;
                let low = digit(bytes.next()?)?;
                out.push(high.wrapping_shl(4) | low);
            }
            other => out.push(other),
        }
    }
    Some(out)
}

/// The value of one hexadecimal digit.
fn digit(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}
