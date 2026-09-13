// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The premise modified condition/decision coverage rests on.
//!
//! The pinned toolchain emits no MC/DC records, so `cargo xtask coverage
//! --condition` measures condition coverage instead: every condition of
//! every decision evaluated both ways. For a decision built only from
//! the short-circuit operators `&&` and `||`, that is masking MC/DC,
//! because a condition is evaluated only where every condition that
//! could mask it has already taken the value that does not mask it, so
//! the condition decides the outcome where it is evaluated. A bitwise
//! `&`, `|` or `^` over booleans breaks the premise: both operands are
//! evaluated whatever the first says, and the toolchain records no
//! condition for either.
//!
//! This check reads the typed tree of each crate held to complete
//! coverage and reports every bitwise operator whose operands are
//! booleans. `-Zunpretty=thir-tree` is what carries the types; a
//! reading of the source carries none, and `&` there is a reference as
//! often as an operator.

use std::path::Path;

use crate::error::Error;
use crate::policy::COMPLETE;
use crate::process::Cmd;

/// The operators that evaluate both of their operands.
const BITWISE: [&str; 3] = ["op: BitAnd", "op: BitOr", "op: BitXor"];

/// One crate of [`COMPLETE`] and what its typed tree said.
pub(crate) struct Report {
    /// The crate the tree came from.
    pub(crate) name: &'static str,
    /// How many short-circuit operators its decisions are built from.
    pub(crate) operators: usize,
    /// One line per bitwise operator over booleans.
    pub(crate) violations: Vec<String>,
}

/// Reads the typed tree of every crate of [`COMPLETE`].
///
/// # Errors
///
/// [`Error`] names what the compiler refused.
pub(crate) fn check(root: &Path) -> Result<Vec<Report>, Error> {
    let mut reports = Vec::new();
    for name in COMPLETE {
        let tree = Cmd::cargo()
            .cwd(root)
            .args(["rustc", "-p", name, "--lib", "--", "-Zunpretty=thir-tree"])
            .capture()?;
        reports.push(Report {
            name,
            operators: tree
                .lines()
                .filter(|line| line.trim() == "LogicalOp {")
                .count(),
            violations: boolean_bitwise(&tree),
        });
    }
    Ok(reports)
}

/// Every bitwise operator of `tree` whose left operand is a boolean,
/// named by the span the compiler gave it.
///
/// The operand rather than the result is what says it: `a &= b` answers
/// nothing, and `a & b` over booleans answers a boolean either way.
pub(crate) fn boolean_bitwise(tree: &str) -> Vec<String> {
    let lines: Vec<&str> = tree.lines().map(str::trim).collect();
    let mut violations = Vec::new();
    for (at, line) in lines.iter().enumerate() {
        if !BITWISE.contains(line) {
            continue;
        }
        let Some(operand) = lines.iter().skip(at).position(|text| *text == "lhs:") else {
            continue;
        };
        let ty = lines
            .iter()
            .skip(at.saturating_add(operand))
            .find_map(|text| text.strip_prefix("ty: "));
        if ty != Some("bool") {
            continue;
        }
        let span = lines
            .iter()
            .take(at)
            .rev()
            .find_map(|text| text.strip_prefix("span: "))
            .unwrap_or("an unnamed span");
        let operator = line.strip_prefix("op: ").unwrap_or(line);
        violations.push(format!(
            "{span}: `{operator}` over booleans evaluates both operands, \
             so the toolchain records no condition for either"
        ));
    }
    violations
}
