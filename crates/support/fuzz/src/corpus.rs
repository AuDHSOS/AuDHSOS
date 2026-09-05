// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Replaying a corpus without a fuzzer.
//!
//! Invariants: the files of a run are visited in path order, so that a
//! failure names the same file on every machine; a directory contributes
//! every file below it and nothing else; a path the file system will not
//! describe is carried to the read, which is the one place a replay can
//! fail, so that a corpus that is not there is reported and not skipped.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Why a replay could not finish: the file it was reading and the reason.
#[derive(Debug)]
pub struct CorpusError {
    /// The file the run was reading.
    pub path: PathBuf,
    /// What the file system reported.
    pub source: std::io::Error,
}

impl fmt::Display for CorpusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "reading {}: {}", self.path.display(), self.source)
    }
}

impl std::error::Error for CorpusError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// What a replay amounted to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The number of files that were run.
    pub files: usize,
    /// The number of bytes those files held.
    pub bytes: usize,
}

/// Every file at or below `path`, in path order.
///
/// A `path` the file system does not list as a directory is one file,
/// whether it is there or not: the read reports what is wrong with it, so
/// that a corpus directory named by mistake fails the run instead of
/// passing it with nothing to do. An entry the file system will not
/// describe is left out.
#[must_use]
pub fn files_under(path: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect(path, &mut files);
    files.sort();
    files
}

/// Appends the files at or below `path` to `files`.
fn collect(path: &Path, files: &mut Vec<PathBuf>) {
    match std::fs::read_dir(path) {
        Ok(entries) => {
            for entry in entries.flatten() {
                collect(&entry.path(), files);
            }
        }
        Err(_) => files.push(path.to_path_buf()),
    }
}

/// Runs `body` over the content of every file at or below one of `paths`.
///
/// # Errors
///
/// [`CorpusError`] for the first file that cannot be read. A body that
/// panics ends the process, which is what a regression run wants.
pub fn replay_paths(
    paths: &[PathBuf],
    mut body: impl FnMut(&[u8]),
) -> Result<Outcome, CorpusError> {
    let mut outcome = Outcome::default();
    for path in paths {
        for file in files_under(path) {
            let bytes =
                std::fs::read(&file).map_err(|source| CorpusError { path: file, source })?;
            outcome.files = outcome.files.saturating_add(1);
            outcome.bytes = outcome.bytes.saturating_add(bytes.len());
            body(&bytes);
        }
    }
    Ok(outcome)
}

/// Runs `body` over every file at or below one of `args` and reports what
/// it did on the standard error output. This is what the `main` of a fuzz
/// target built without a fuzzer does with its command line.
pub fn replay_args(args: impl IntoIterator<Item = OsString>, body: impl FnMut(&[u8])) -> ExitCode {
    let paths: Vec<PathBuf> = args.into_iter().map(PathBuf::from).collect();
    match replay_paths(&paths, body) {
        Ok(outcome) => {
            eprintln!("replayed {} files, {} bytes", outcome.files, outcome.bytes);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
