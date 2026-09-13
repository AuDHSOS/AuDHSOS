// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The premise modified condition/decision coverage rests on.
//!
//! The pinned toolchain emits no MC/DC records, so `cargo xtask coverage
//! --condition` measures condition coverage instead: every condition of
//! every decision evaluated both ways. Document 16, decision D4, derives
//! MC/DC from that measurement, and this check holds the two premises
//! the derivation needs.
//!
//! Premise 1, which masking MC/DC needs: every decision is built from
//! `&&` and `||` alone. Those evaluate the right operand only where the
//! left one does not settle the answer, so an operand is evaluated only
//! where every operand that could mask it has taken the value that does
//! not mask it, and it decides the outcome wherever it is evaluated. A
//! bitwise `&`, `|` or `^` over booleans breaks it: both operands are
//! evaluated whatever the first says, and the toolchain records no
//! condition for either.
//!
//! Premise 2, which unique-cause MC/DC needs beyond that: no decision
//! names one condition twice, because a condition that appears twice
//! cannot be varied on its own.
//!
//! `-Zunpretty=thir-tree` is what carries the types and the spans. A
//! reading of the source carries neither, and `&` there is a reference
//! as often as an operator.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::fs;
use crate::policy::COMPLETE;
use crate::process::Cmd;

/// The operators that evaluate both of their operands.
const BITWISE: [&str; 3] = ["op: BitAnd", "op: BitOr", "op: BitXor"];

/// How many levels of `Scope` one expression is followed through, which
/// no expression of a decision comes near.
const SCOPES: usize = 64;

/// What one level of the typed tree is indented by.
const STEP: usize = 4;

/// One crate of [`COMPLETE`] and what its typed tree said.
pub(crate) struct Report {
    /// The crate the tree came from.
    pub(crate) name: &'static str,
    /// How many short-circuit operators its decisions are built from.
    pub(crate) operators: usize,
    /// How many conditions those decisions hold.
    pub(crate) conditions: usize,
    /// One line per broken premise.
    pub(crate) violations: Vec<String>,
}

/// Reads the typed tree of every crate of [`COMPLETE`].
///
/// # Errors
///
/// [`Error`] names what the compiler refused and what a source the tree
/// points into would not answer.
pub(crate) fn check(root: &Path) -> Result<Vec<Report>, Error> {
    let mut reports = Vec::new();
    for name in COMPLETE {
        let tree = Cmd::cargo()
            .cwd(root)
            .args(["rustc", "-p", name, "--lib", "--", "-Zunpretty=thir-tree"])
            .capture()?;
        let lines = read(&tree);
        let mut violations = boolean_bitwise(&tree);
        let (conditions, repeated) = repeated_conditions(root, &lines)?;
        violations.extend(repeated);
        reports.push(Report {
            name,
            operators: lines
                .iter()
                .filter(|line| line.text == "LogicalOp {")
                .count(),
            conditions,
            violations,
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

/// One line of the typed tree, by how deep it lies and what it says.
pub(crate) struct Line<'a> {
    /// How many spaces it begins with, which is its depth times [`STEP`].
    indent: usize,
    /// The line without the spaces around it.
    text: &'a str,
}

/// The lines of a typed tree, without the empty ones.
pub(crate) fn read(tree: &str) -> Vec<Line<'_>> {
    tree.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| Line {
            indent: line.len().saturating_sub(line.trim_start().len()),
            text: line.trim(),
        })
        .collect()
}

/// How many conditions the decisions of `lines` hold, and one violation
/// per decision that names a condition twice.
pub(crate) fn repeated_conditions(
    root: &Path,
    lines: &[Line],
) -> Result<(usize, Vec<String>), Error> {
    let mut sources = Sources::new(root);
    let mut violations = Vec::new();
    let mut count: usize = 0;
    let nested = operators_of_operators(lines);
    for (at, line) in lines.iter().enumerate() {
        if line.text != "LogicalOp {" || nested.contains(&at) {
            continue;
        }
        let mut spans = Vec::new();
        conditions(lines, at, &mut spans);
        count = count.saturating_add(spans.len());
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for span in spans {
            let Some(text) = sources.text(span)? else {
                continue;
            };
            if !seen.insert(text.clone()) {
                violations.push(format!(
                    "{span}: `{text}` is named twice by one decision, \
                     so no test varies it on its own"
                ));
            }
        }
    }
    Ok((count, violations))
}

/// Every short-circuit operator that is an operand of another one,
/// which makes it a part of a decision rather than a decision.
fn operators_of_operators(lines: &[Line]) -> BTreeSet<usize> {
    let mut nested = BTreeSet::new();
    for (at, line) in lines.iter().enumerate() {
        if line.text != "LogicalOp {" {
            continue;
        }
        for side in ["lhs:", "rhs:"] {
            let Some(operand) = child(lines, at, side).and_then(|s| first(lines, s)) else {
                continue;
            };
            let inner = unwrapped(lines, operand);
            if let Some(k) = kind(lines, inner)
                && lines.get(k).is_some_and(|line| line.text == "LogicalOp {")
            {
                nested.insert(k);
            }
        }
    }
    nested
}

/// The spans of the conditions the operator at `at` is made of, which
/// are its operands and the operands of the operators among them.
fn conditions<'a>(lines: &[Line<'a>], at: usize, out: &mut Vec<&'a str>) {
    for side in ["lhs:", "rhs:"] {
        let Some(operand) = child(lines, at, side).and_then(|s| first(lines, s)) else {
            continue;
        };
        let inner = unwrapped(lines, operand);
        match kind(lines, inner) {
            Some(k) if lines.get(k).is_some_and(|line| line.text == "LogicalOp {") => {
                conditions(lines, k, out);
            }
            _ => out.extend(span(lines, operand)),
        }
    }
}

/// The lines one level below `at`, until that level closes.
fn below<'b>(lines: &'b [Line<'_>], at: usize) -> impl Iterator<Item = usize> + use<'b> {
    let depth = lines.get(at).map_or(0, |line| line.indent);
    lines
        .iter()
        .enumerate()
        .skip(at.saturating_add(1))
        .take_while(move |(_, line)| line.indent > depth)
        .filter(move |(_, line)| line.indent == depth.saturating_add(STEP))
        .map(|(index, _)| index)
}

/// The line one level below `at` that begins with `name`.
fn child(lines: &[Line], at: usize, name: &str) -> Option<usize> {
    below(lines, at).find(|index| {
        lines
            .get(*index)
            .is_some_and(|line| line.text == name || line.text.starts_with(name))
    })
}

/// The first line one level below `at`.
fn first(lines: &[Line], at: usize) -> Option<usize> {
    below(lines, at).next()
}

/// The span of the expression at `at`.
fn span<'a>(lines: &[Line<'a>], at: usize) -> Option<&'a str> {
    below(lines, at).find_map(|index| {
        lines
            .get(index)
            .and_then(|line| line.text.strip_prefix("span: "))
    })
}

/// What the expression at `at` is, which is the one line below its
/// `kind:`.
fn kind(lines: &[Line], at: usize) -> Option<usize> {
    child(lines, at, "kind:").and_then(|index| first(lines, index))
}

/// The expression at `at` with every `Scope` around it followed through,
/// because a scope holds the expression it names and says nothing else.
fn unwrapped(lines: &[Line], at: usize) -> usize {
    let mut at = at;
    for _ in 0..SCOPES {
        let Some(scope) = kind(lines, at)
            .filter(|index| lines.get(*index).is_some_and(|line| line.text == "Scope {"))
        else {
            return at;
        };
        let Some(inner) = child(lines, scope, "value:").and_then(|index| first(lines, index))
        else {
            return at;
        };
        at = inner;
    }
    at
}

/// The sources the spans point into, read once each.
struct Sources<'a> {
    /// Where the paths of the spans are relative to.
    root: &'a Path,
    /// One file by the path its span named, split into lines.
    held: BTreeMap<PathBuf, Vec<String>>,
}

impl<'a> Sources<'a> {
    /// A reader over `root`.
    const fn new(root: &'a Path) -> Self {
        Sources {
            root,
            held: BTreeMap::new(),
        }
    }

    /// The source `span` names, with the space in it evened out, or
    /// `None` where the span came from a macro rather than from a file.
    ///
    /// # Errors
    ///
    /// [`Error`] names the file that would not be read.
    fn text(&mut self, span: &str) -> Result<Option<String>, Error> {
        let Some(place) = Place::read(span) else {
            return Ok(None);
        };
        let path = self.root.join(&place.path);
        if !self.held.contains_key(&path) {
            let content = fs::read(&path)?;
            self.held.insert(
                path.clone(),
                content.lines().map(ToOwned::to_owned).collect(),
            );
        }
        let held = self.held.get(&path);
        let line = |number: usize| -> Vec<char> {
            held.and_then(|lines| lines.get(number.saturating_sub(1)))
                .map(|text| text.chars().collect())
                .unwrap_or_default()
        };
        let mut words = String::new();
        for number in place.from.0..=place.to.0 {
            let chars = line(number);
            let from = if number == place.from.0 {
                place.from.1.saturating_sub(1)
            } else {
                0
            };
            let to = if number == place.to.0 {
                place.to.1.saturating_sub(1)
            } else {
                chars.len()
            };
            for character in chars.into_iter().take(to).skip(from) {
                words.push(character);
            }
            words.push(' ');
        }
        Ok(Some(
            words.split_whitespace().collect::<Vec<&str>>().join(" "),
        ))
    }
}

/// Where in a file one span lies.
pub(crate) struct Place {
    /// The file, relative to the workspace root.
    pub(crate) path: String,
    /// The line and the column it begins at, both counted from one.
    pub(crate) from: (usize, usize),
    /// The line and the column it ends at.
    pub(crate) to: (usize, usize),
}

impl Place {
    /// Reads `path:line:column: line:column (#0)`, and answers `None`
    /// for a span the compiler wrote from a macro, which names the macro
    /// rather than the source and would read as one condition for every
    /// field a derive compares.
    pub(crate) fn read(span: &str) -> Option<Self> {
        let body = span.strip_suffix(" (#0)")?;
        let (head, rest) = body.split_once(": ")?;
        let mut back = head.rsplitn(3, ':');
        let column = back.next()?.parse().ok()?;
        let line = back.next()?.parse().ok()?;
        let path = back.next()?.to_owned();
        let (last_line, last_column) = rest.split_once(':')?;
        Some(Place {
            path,
            from: (line, column),
            to: (last_line.parse().ok()?, last_column.parse().ok()?),
        })
    }
}
