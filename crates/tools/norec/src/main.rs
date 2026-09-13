// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A NoREC fuzzer: random SQL against a database engine, checked against
//! the same query in a form that engine cannot optimize.
//!
//! One case is a random database, a random predicate, and two queries over
//! them. The first puts the predicate in a `WHERE` clause, which is what an
//! optimizer works on; the second evaluates the same predicate on every row
//! of the same `FROM` clause and sums the truths, which leaves an optimizer
//! nothing to do. Both must answer the same number. When they do not, the
//! engine optimized a query into a different query, and the case is written
//! out as a file that reproduces it.
//!
//! The technique is NoREC: Manuel Rigger and Zhendong Su, *Detecting
//! Optimization Bugs in Database Engines via Non-Optimizing Reference
//! Engine Construction*, ESEC/FSE 2020. The copy this is written against,
//! and what is cited from it, are in `docs/acm/README.md`.

#![forbid(unsafe_code)]
#![expect(
    clippy::doc_markdown,
    reason = "the prose of this crate names NoREC, SQLite and SQLancer, which are names and not identifiers"
)]

mod case;
mod engine;
mod error;
mod expr;
mod generate;
mod options;
mod oracle;
mod reduce;
mod report;
mod rng;
mod schema;
mod value;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::case::{Case, CountStyle};
use crate::engine::{Engine, Sqlite3};
use crate::error::Error;
use crate::options::{Mode, Options, Request, USAGE};
use crate::oracle::Verdict;
use crate::rng::Rng;

/// What a whole run found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    /// Cases run.
    pub(crate) cases: u64,
    /// Cases both queries agreed on.
    pub(crate) agreed: u64,
    /// Cases the engine refused, by the message it refused with.
    pub(crate) refused: BTreeMap<String, u64>,
    /// The findings, in the order they were made.
    pub(crate) findings: Vec<Finding>,
}

impl Summary {
    /// How many cases were refused, over all messages.
    pub(crate) fn refusals(&self) -> u64 {
        self.refused
            .values()
            .fold(0, |total, count| total.saturating_add(*count))
    }
}

/// One disagreement, after reduction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    /// The seed the case was generated from.
    pub(crate) seed: u64,
    /// What the `WHERE` clause selected.
    pub(crate) optimized: u64,
    /// What the predicate summed to.
    pub(crate) unoptimized: u64,
    /// Where the reproducer was written.
    pub(crate) path: PathBuf,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match start(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(Error::Usage(message)) => {
            eprintln!("{message}\n{USAGE}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs what the command line asked for. `false` means something was
/// found, which is a failing exit status and not an error.
fn start(args: &[String]) -> Result<bool, Error> {
    let options = match options::parse(args)? {
        Request::Help => {
            print!("{USAGE}");
            return Ok(true);
        }
        Request::Run(options) => *options,
    };
    if options.mode == Mode::Script {
        for number in 0..options.runs {
            let seed = options.seed.wrapping_add(number);
            let (case, style) = generated(seed, number);
            println!("-- seed {seed}");
            print!("{}", oracle::script(&case, style));
        }
        return Ok(true);
    }
    let mut engine = Sqlite3::new(&options.program, options.timeout);
    let summary = campaign(&mut engine, &options)?;
    report_summary(&summary);
    Ok(summary.findings.is_empty())
}

/// Runs every case of the campaign.
pub(crate) fn campaign(engine: &mut dyn Engine, options: &Options) -> Result<Summary, Error> {
    let mut summary = Summary::default();
    for number in 0..options.runs {
        let seed = options.seed.wrapping_add(number);
        let (case, style) = generated(seed, number);
        summary.cases = summary.cases.saturating_add(1);
        match oracle::check(engine, &case, style)? {
            Verdict::Agree(rows) => {
                summary.agreed = summary.agreed.saturating_add(1);
                if options.verbose {
                    println!("norec: seed {seed} agreed on {rows} row(s)");
                }
            }
            Verdict::Failed(message) => {
                let count = summary.refused.entry(message.clone()).or_insert(0);
                *count = count.saturating_add(1);
                if options.verbose {
                    println!("norec: seed {seed} refused: {message}");
                }
            }
            Verdict::Mismatch {
                optimized,
                unoptimized,
            } => {
                let finding = record(engine, options, &case, style, seed, optimized, unoptimized)?;
                println!(
                    "norec: seed {seed} disagreed, {} against {}, written to {}",
                    finding.optimized,
                    finding.unoptimized,
                    finding.path.display()
                );
                summary.findings.push(finding);
                if options.stop {
                    break;
                }
            }
        }
    }
    Ok(summary)
}

/// The case of one seed, and how its optimized query is counted. The two
/// ways alternate, as section 3.3 of the paper has it: `COUNT(*)` is
/// cheaper, and the plain row set is the one an optimizer is given plainly.
fn generated(seed: u64, number: u64) -> (Case, CountStyle) {
    let style = if number.checked_rem(2) == Some(0) {
        CountStyle::Rows
    } else {
        CountStyle::Count
    };
    (generate::case(&mut Rng::from_seed(seed)), style)
}

/// Shrinks a disagreement and writes it out.
fn record(
    engine: &mut dyn Engine,
    options: &Options,
    case: &Case,
    style: CountStyle,
    seed: u64,
    optimized: u64,
    unoptimized: u64,
) -> Result<Finding, Error> {
    let mut verdict = Verdict::Mismatch {
        optimized,
        unoptimized,
    };
    let mut smallest = case.clone();
    if options.reduce > 0 {
        smallest = reduce::reduce(engine, case, style, options.reduce)?;
        // The reduced case is read again, so that the numbers in the report
        // are the numbers of the case in the report.
        if let mismatch @ Verdict::Mismatch { .. } = oracle::check(engine, &smallest, style)? {
            verdict = mismatch;
        }
    }
    let text = report::reproducer(&smallest, style, &verdict, seed);
    let path = report::write(&options.reports, seed, &text)?;
    let (optimized, unoptimized) = match verdict {
        Verdict::Mismatch {
            optimized,
            unoptimized,
        } => (optimized, unoptimized),
        Verdict::Agree(_) | Verdict::Failed(_) => (optimized, unoptimized),
    };
    Ok(Finding {
        seed,
        optimized,
        unoptimized,
        path,
    })
}

/// The last lines of a run: what agreed, what was refused, what was found.
fn report_summary(summary: &Summary) {
    println!(
        "norec: {} case(s), {} agreed, {} refused, {} finding(s)",
        summary.cases,
        summary.agreed,
        summary.refusals(),
        summary.findings.len()
    );
    let mut reasons: Vec<(&String, &u64)> = summary.refused.iter().collect();
    reasons.sort_by(|left, right| right.1.cmp(left.1));
    for (message, count) in reasons.iter().take(3) {
        println!("norec:   refused {count} time(s): {message}");
    }
}
