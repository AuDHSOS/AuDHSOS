// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a run measures, and the command line that says so.

/// What the tool answers to `--help`.
pub(crate) const USAGE: &str = "\
usage: membench [--size <mebibytes>] [--chase <mebibytes>]
                [--passes <count>] [--steps <count>]

  --size    mebibytes per buffer of the bandwidth measurements, of which
            there are two (default 256)
  --chase   largest working set of the pointer chase, in mebibytes; the
            smaller ones are measured as well (default 512)
  --passes  passes over the buffer per bandwidth measurement (default 3)
  --steps   steps of one pointer chase (default 5000000)
";

/// Mebibytes per buffer of a run that names no size.
const DEFAULT_MEBIBYTES: u64 = 256;

/// Mebibytes of the largest working set of the chase.
const DEFAULT_CHASE_MEBIBYTES: u64 = 512;

/// Passes per bandwidth measurement.
const DEFAULT_PASSES: u64 = 3;

/// Steps of one pointer chase.
const DEFAULT_STEPS: u64 = 5_000_000;

/// Bits between a byte count and the mebibytes it makes.
pub(crate) const MEBIBYTE_SHIFT: u32 = 20;

/// Bytes in a mebibyte.
pub(crate) const MEBIBYTE: u64 = 1 << MEBIBYTE_SHIFT;

/// The largest size a run may ask for, in mebibytes. Above this the number
/// is a typo and not a machine: two stream buffers would be 128 GiB.
const MAX_MEBIBYTES: u64 = 65_536;

/// What one run measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Config {
    /// Bytes per buffer of the bandwidth measurements.
    pub(crate) bytes: u64,
    /// Largest working set of the pointer chase, in bytes.
    pub(crate) chase_bytes: u64,
    /// Passes over the buffer per bandwidth measurement.
    pub(crate) passes: u64,
    /// Steps of one pointer chase.
    pub(crate) steps: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bytes: DEFAULT_MEBIBYTES.saturating_mul(MEBIBYTE),
            chase_bytes: DEFAULT_CHASE_MEBIBYTES.saturating_mul(MEBIBYTE),
            passes: DEFAULT_PASSES,
            steps: DEFAULT_STEPS,
        }
    }
}

impl Config {
    /// The configuration the arguments name, or `None` when they ask for
    /// the usage.
    ///
    /// # Errors
    ///
    /// The message for an unknown option, a missing value, a value that is
    /// not a number, a zero where a count is wanted, and a size above what
    /// any machine this runs on holds.
    pub(crate) fn parse(arguments: &[String]) -> Result<Option<Config>, String> {
        let mut config = Config::default();
        let mut rest = arguments.iter();
        while let Some(argument) = rest.next() {
            let field = match argument.as_str() {
                "--help" | "-h" => return Ok(None),
                "--size" => Field::Size,
                "--chase" => Field::Chase,
                "--passes" => Field::Passes,
                "--steps" => Field::Steps,
                other => return Err(format!("unknown option `{other}`")),
            };
            let Some(text) = rest.next() else {
                return Err(format!("{argument} wants a number"));
            };
            let value = count(argument, text)?;
            match field {
                Field::Size => config.bytes = mebibytes(argument, value)?,
                Field::Chase => config.chase_bytes = mebibytes(argument, value)?,
                Field::Passes => config.passes = value,
                Field::Steps => config.steps = value,
            }
        }
        Ok(Some(config))
    }
}

/// What an option sets.
#[derive(Clone, Copy, Debug)]
enum Field {
    /// The bytes per buffer of the bandwidth measurements.
    Size,
    /// The largest working set of the chase.
    Chase,
    /// The passes per bandwidth measurement.
    Passes,
    /// The steps of one pointer chase.
    Steps,
}

/// A count above zero, or the message that names what was read.
fn count(option: &str, text: &str) -> Result<u64, String> {
    match text.parse::<u64>() {
        Ok(0) | Err(_) => Err(format!("{option} wants a number above zero, not `{text}`")),
        Ok(value) => Ok(value),
    }
}

/// A count of mebibytes as bytes, refused above what a machine holds.
fn mebibytes(option: &str, value: u64) -> Result<u64, String> {
    if value > MAX_MEBIBYTES {
        return Err(format!(
            "{option} {value} is above the {MAX_MEBIBYTES} MiB this measures at most"
        ));
    }
    Ok(value.saturating_mul(MEBIBYTE))
}
