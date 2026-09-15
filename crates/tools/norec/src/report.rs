// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a finding leaves behind: a file that reproduces it with nothing
//! but the shell.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::case::{Case, CountStyle};
use crate::error::Error;
use crate::oracle::Verdict;

/// The reproducer: the database, then the two queries with what each
/// answered.
pub(crate) fn reproducer(case: &Case, style: CountStyle, verdict: &Verdict, seed: u64) -> String {
    let mut out = String::from("-- norec: the two queries disagree over the same predicate.\n");
    let _ = writeln!(out, "-- seed {seed}");
    if let Verdict::Mismatch {
        optimized,
        unoptimized,
    } = verdict
    {
        let _ = writeln!(
            out,
            "-- the WHERE clause selected {optimized} row(s); the predicate was true {unoptimized} time(s)"
        );
    }
    out.push_str("-- run: sqlite3 -batch :memory: < this file\n\n");
    for statement in case.setup() {
        out.push_str(&statement);
        out.push('\n');
    }
    out.push('\n');
    let _ = writeln!(out, "{}", case.optimized(style));
    let _ = writeln!(out, "{}", case.unoptimized());
    out
}

/// Writes the reproducer under `directory` and answers where it went.
pub(crate) fn write(directory: &Path, seed: u64, text: &str) -> Result<PathBuf, Error> {
    fs::create_dir_all(directory).map_err(|source| Error::path("creating", directory, source))?;
    let path = directory.join(format!("norec-{seed}.sql"));
    fs::write(&path, text).map_err(|source| Error::path("writing", &path, source))?;
    Ok(path)
}
