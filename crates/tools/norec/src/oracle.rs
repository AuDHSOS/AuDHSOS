// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The NoREC check: the same predicate, once in a `WHERE` clause the
//! optimizer works on and once on every row of the same `FROM` clause,
//! must select the same number of rows.
//!
//! Manuel Rigger and Zhendong Su, *Detecting Optimization Bugs in Database
//! Engines via Non-Optimizing Reference Engine Construction*, ESEC/FSE
//! 2020, section 3. The copy this is written against is in
//! `docs/acm/README.md`.

use std::fmt::Write as _;

use crate::case::{Case, CountStyle};
use crate::engine::{Engine, Run};
use crate::error::Error;

/// What the two queries said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Both counted the same rows.
    Agree(u64),
    /// They disagree, which is a bug in the engine.
    Mismatch {
        /// What the `WHERE` clause selected.
        optimized: u64,
        /// What the predicate summed to over every row.
        unoptimized: u64,
    },
    /// The engine refused the case; the string is its first message.
    Failed(String),
}

/// The marker before the optimized query's rows.
const OPTIMIZED: &str = "--norec:optimized--";
/// The marker between the two answers.
const UNOPTIMIZED: &str = "--norec:unoptimized--";
/// The marker after the last answer.
const END: &str = "--norec:end--";

/// The script that builds the database and asks both questions. The
/// markers are printed by the engine itself, so a section is exactly what
/// the query before it produced, whatever the output holds.
pub(crate) fn script(case: &Case, style: CountStyle) -> String {
    let mut out = String::from(".mode list\n.headers off\n");
    for statement in case.setup() {
        out.push_str(&statement);
        out.push('\n');
    }
    let _ = writeln!(out, "SELECT '{OPTIMIZED}';");
    out.push_str(&case.optimized(style));
    out.push('\n');
    let _ = writeln!(out, "SELECT '{UNOPTIMIZED}';");
    out.push_str(&case.unoptimized());
    out.push('\n');
    let _ = writeln!(out, "SELECT '{END}';");
    out
}

/// Reads the verdict out of what the engine printed.
pub(crate) fn read(run: &Run, style: CountStyle) -> Result<Verdict, Error> {
    if run.timed_out {
        return Ok(Verdict::Failed("the engine did not finish".to_owned()));
    }
    if let Some(line) = run.stderr.lines().find(|line| !line.trim().is_empty()) {
        return Ok(Verdict::Failed(line.trim().to_owned()));
    }
    let Some((rows, sum)) = sections(&run.stdout) else {
        return Err(Error::Engine(
            "the engine printed no answer to the two queries".to_owned(),
        ));
    };
    let optimized = match style {
        CountStyle::Rows => u64::try_from(rows.len()).unwrap_or(u64::MAX),
        CountStyle::Count => number(rows.first().copied().unwrap_or(""))?,
    };
    // SUM over no row is NULL, which prints as nothing and counts as zero.
    let unoptimized = number(sum.first().copied().unwrap_or(""))?;
    Ok(if optimized == unoptimized {
        Verdict::Agree(optimized)
    } else {
        Verdict::Mismatch {
            optimized,
            unoptimized,
        }
    })
}

/// The lines of the two answers, split at the markers.
fn sections(stdout: &str) -> Option<(Vec<&str>, Vec<&str>)> {
    let mut rows: Vec<&str> = Vec::new();
    let mut sum: Vec<&str> = Vec::new();
    let mut at = Section::Before;
    for line in stdout.lines() {
        match line {
            OPTIMIZED => at = Section::Rows,
            UNOPTIMIZED => at = Section::Sum,
            END => at = Section::After,
            _ => match at {
                Section::Rows => rows.push(line),
                Section::Sum => sum.push(line),
                Section::Before | Section::After => {}
            },
        }
    }
    matches!(at, Section::After).then_some((rows, sum))
}

/// Which answer the lines being read belong to.
enum Section {
    /// Before the first marker.
    Before,
    /// The optimized query's output.
    Rows,
    /// The unoptimized query's output.
    Sum,
    /// After the last marker.
    After,
}

/// A count as the shell prints it. Empty is `NULL`, which is zero here.
fn number(line: &str) -> Result<u64, Error> {
    let text = line.trim();
    if text.is_empty() {
        return Ok(0);
    }
    text.parse::<u64>()
        .map_err(|_| Error::Engine(format!("`{text}` is not a row count")))
}

/// Runs one case and answers what the engine said.
pub(crate) fn check(
    engine: &mut dyn Engine,
    case: &Case,
    style: CountStyle,
) -> Result<Verdict, Error> {
    let run = engine.run(&script(case, style))?;
    read(&run, style)
}

/// Whether the case still shows a disagreement, which is what reduction
/// keeps asking.
pub(crate) fn mismatches(
    engine: &mut dyn Engine,
    case: &Case,
    style: CountStyle,
) -> Result<bool, Error> {
    Ok(matches!(
        check(engine, case, style)?,
        Verdict::Mismatch { .. }
    ))
}
