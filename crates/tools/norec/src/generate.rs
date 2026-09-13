// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The random test case: a database, a `FROM` clause and a predicate.
//!
//! Three kinds of expression are never generated, following section 3.4 of
//! the paper. A subquery can be ambiguous, so the two queries would be
//! allowed to disagree. A function of the clock or of a random source
//! answers differently per call, so they would disagree for a second
//! reason. And `DISTINCT`, aggregates and window functions compute over
//! several rows, which the translation of section 3.2 does not carry.

use crate::case::{Case, Join, JoinKind, Source};
use crate::expr::{ColumnRef, Expr};
use crate::rng::Rng;
use crate::schema::{Affinity, Column, Index, Table};
use crate::value::Value;

/// The affinities a column is declared with.
const AFFINITIES: [Affinity; 6] = [
    Affinity::Integer,
    Affinity::Real,
    Affinity::Text,
    Affinity::Numeric,
    Affinity::Blob,
    Affinity::None,
];

/// The collating sequences SQLite has built in.
const COLLATIONS: [&str; 3] = ["BINARY", "NOCASE", "RTRIM"];

/// Text values the generator draws from, so that comparisons meet. Short
/// and without a newline, which keeps one row on one line of output.
const WORDS: [&str; 10] = ["", "a", "A", "b", " a", "a ", "0", "1", "0.0", "-1"];

/// Integers that are drawn more often than the range they lie in.
const EDGE_INTS: [i64; 6] = [i64::MIN, i64::MAX, 0, 1, -1, 2];

/// Doubles that are drawn more often than the range they lie in.
const EDGE_REALS: [f64; 6] = [0.0, -0.0, 0.5, -1.5, 1e300, 2.0];

/// Comparison and boolean operators.
const COMPARISONS: [&str; 12] = [
    "=", "==", "<>", "!=", "<", "<=", ">", ">=", "IS", "IS NOT", "LIKE", "GLOB",
];

/// Operators that compute a value rather than a truth.
const ARITHMETIC: [&str; 10] = ["+", "-", "*", "/", "%", "||", "&", "|", "<<", ">>"];

/// Functions that are deterministic, by name and arity.
const FUNCTIONS: [(&str, usize); 17] = [
    ("abs", 1),
    ("length", 1),
    ("lower", 1),
    ("upper", 1),
    ("ltrim", 1),
    ("rtrim", 1),
    ("trim", 1),
    ("hex", 1),
    ("quote", 1),
    ("typeof", 1),
    ("unicode", 1),
    ("round", 1),
    ("coalesce", 2),
    ("ifnull", 2),
    ("nullif", 2),
    ("max", 2),
    ("min", 2),
];

/// The type names a `CAST` converts to.
const TYPES: [&str; 5] = ["INTEGER", "REAL", "TEXT", "NUMERIC", "BLOB"];

/// What an expression may refer to.
struct Context<'a> {
    /// Every table of the case.
    tables: &'a [Table],
    /// The tables the `FROM` clause names, by index into `tables`.
    visible: Vec<usize>,
}

/// One random case.
pub(crate) fn case(rng: &mut Rng) -> Case {
    let count = if rng.chance(1, 3) { 2 } else { 1 };
    let mut tables = Vec::new();
    for index in 0..count {
        tables.push(table(rng, index));
    }
    for index in 0..tables.len() {
        let indexes = indexes_for(rng, &tables, index);
        if let Some(table) = tables.get_mut(index) {
            table.indexes = indexes;
        }
    }
    let source = source(rng, &tables);
    let visible = std::iter::once(source.first)
        .chain(source.joins.iter().map(|join| join.table))
        .collect();
    let context = Context {
        tables: &tables,
        visible,
    };
    let predicate = predicate(rng, &context, 3);
    Case {
        tables,
        source,
        predicate,
        analyze: rng.chance(1, 2),
    }
}

/// One table with its columns and rows, without indexes.
fn table(rng: &mut Rng, index: usize) -> Table {
    let width = usize::try_from(rng.range(1, 4)).unwrap_or(1);
    let columns: Vec<Column> = (0..width)
        .map(|at| Column {
            name: format!("c{at}"),
            affinity: rng.pick(&AFFINITIES).copied().unwrap_or(Affinity::None),
            collation: if rng.chance(1, 6) {
                rng.pick(&COLLATIONS).copied()
            } else {
                None
            },
        })
        .collect();
    let height = usize::try_from(rng.range(0, 6)).unwrap_or(0);
    let rows = (0..height)
        .map(|_| (0..width).map(|_| value(rng)).collect())
        .collect();
    Table {
        name: format!("t{index}"),
        columns,
        rows,
        indexes: Vec::new(),
    }
}

/// One stored value. Small and repetitive on purpose: values that never
/// meet make every predicate false and every case uninteresting.
fn value(rng: &mut Rng) -> Value {
    match rng.below(6) {
        0 => Value::Null,
        1 => Value::Int(rng.pick(&EDGE_INTS).copied().unwrap_or(0)),
        2 => Value::Real(rng.pick(&EDGE_REALS).copied().unwrap_or(0.0)),
        3 | 4 => Value::Text(
            rng.pick(&WORDS)
                .map_or_else(String::new, |word| (*word).to_owned()),
        ),
        _ => {
            let span = i64::try_from(rng.below(9)).unwrap_or(0);
            Value::Int(span.saturating_sub(4))
        }
    }
}

/// The indexes of one table. A `UNIQUE` index is only built over plain
/// columns whose rows are distinct, because the statement that creates one
/// over duplicates fails and the case is then never run.
fn indexes_for(rng: &mut Rng, tables: &[Table], at: usize) -> Vec<Index> {
    let Some(table) = tables.get(at) else {
        return Vec::new();
    };
    let count = rng.below(3);
    let context = Context {
        tables,
        visible: vec![at],
    };
    (0..count)
        .map(|number| {
            let width = usize::try_from(rng.range(1, 2)).unwrap_or(1);
            let terms: Vec<Expr> = (0..width)
                .map(|_| {
                    let term = if rng.chance(3, 4) {
                        column(rng, &context)
                    } else {
                        expression(rng, &context, 1)
                    };
                    if term.mentions_column() {
                        term
                    } else {
                        column(rng, &context)
                    }
                })
                .collect();
            Index {
                name: format!("i{at}_{number}"),
                unique: rng.chance(1, 4) && distinct(table, &terms),
                terms,
                filter: if rng.chance(1, 3) {
                    Some(predicate(rng, &context, 1)).filter(Expr::mentions_column)
                } else {
                    None
                },
            }
        })
        .collect()
}

/// Whether a `UNIQUE` index over `terms` is safe to create: the rows must
/// be distinct as SQLite compares them, and a statement that fails would
/// cost the whole case. The test is deliberately narrow — one plain column
/// without a collation, values whose comparison needs no conversion — and
/// answers `false` wherever it cannot be sure. O(rows²) over at most six
/// rows.
fn distinct(table: &Table, terms: &[Expr]) -> bool {
    let [Expr::Column(reference)] = terms else {
        return false;
    };
    let Some(column) = table.columns.get(reference.column) else {
        return false;
    };
    if column.collation.is_some() {
        return false;
    }
    let mut seen: Vec<String> = Vec::new();
    for row in &table.rows {
        match row.get(reference.column) {
            // A NULL is distinct from every other value in a unique index.
            Some(Value::Null) => {}
            Some(value) => match unique_key(column.affinity, value) {
                Some(key) if !seen.contains(&key) => seen.push(key),
                _ => return false,
            },
            None => return false,
        }
    }
    true
}

/// How one value compares against another of the same column, or nothing
/// when the answer depends on a conversion this test does not model.
/// INTEGER and REAL compare numerically whatever the affinity, so both
/// answer the same key; a value an affinity would convert answers none.
fn unique_key(affinity: Affinity, value: &Value) -> Option<String> {
    match (affinity, value) {
        // Under TEXT affinity a number is stored as its text, where it can
        // meet a text value that was never a number.
        (Affinity::Text, Value::Text(text)) => Some(format!("t{text}")),
        // Under a numeric affinity text that looks like a number is stored
        // as one, and which text does is SQLite's rule and not this one.
        (Affinity::Text, _)
        | (_, Value::Null)
        | (Affinity::Integer | Affinity::Real | Affinity::Numeric, Value::Text(_)) => None,
        (_, Value::Int(number)) => i32::try_from(*number)
            .ok()
            .map(|small| numeric_key(f64::from(small))),
        (_, Value::Real(number)) if number.is_finite() => Some(numeric_key(*number)),
        (_, Value::Real(_)) => None,
        (_, Value::Text(text)) => Some(format!("t{text}")),
    }
}

/// The key of a number. Negative zero equals zero, so both answer the same.
fn numeric_key(number: f64) -> String {
    if number == 0.0 {
        "n0".to_owned()
    } else {
        format!("n{number}")
    }
}

/// The `FROM` clause.
fn source(rng: &mut Rng, tables: &[Table]) -> Source {
    let mut joins = Vec::new();
    if tables.len() > 1 {
        let context = Context {
            tables,
            visible: (0..tables.len()).collect(),
        };
        let kind = match rng.below(3) {
            0 => JoinKind::Comma,
            1 => JoinKind::Inner,
            _ => JoinKind::Left,
        };
        joins.push(Join {
            kind,
            table: 1,
            on: match kind {
                JoinKind::Comma => None,
                _ => Some(predicate(rng, &context, 2)),
            },
        });
    }
    Source { first: 0, joins }
}

/// A predicate: an expression whose root compares or combines, so that the
/// case tests a filter rather than a constant.
fn predicate(rng: &mut Rng, context: &Context<'_>, depth: u32) -> Expr {
    if depth == 0 || rng.chance(1, 4) {
        return expression(rng, context, depth);
    }
    let next = depth.saturating_sub(1);
    match rng.below(8) {
        0 => Expr::Binary {
            op: "AND",
            left: Box::new(predicate(rng, context, next)),
            right: Box::new(predicate(rng, context, next)),
        },
        1 => Expr::Binary {
            op: "OR",
            left: Box::new(predicate(rng, context, next)),
            right: Box::new(predicate(rng, context, next)),
        },
        2 => Expr::Prefix {
            op: "NOT",
            operand: Box::new(predicate(rng, context, next)),
        },
        3 => Expr::Suffix {
            op: if rng.chance(1, 2) {
                "IS NULL"
            } else {
                "IS NOT NULL"
            },
            operand: Box::new(expression(rng, context, next)),
        },
        4 => Expr::Between {
            value: Box::new(expression(rng, context, next)),
            low: Box::new(expression(rng, context, next)),
            high: Box::new(expression(rng, context, next)),
            negated: rng.chance(1, 3),
        },
        5 => {
            let width = usize::try_from(rng.range(1, 3)).unwrap_or(1);
            Expr::In {
                value: Box::new(expression(rng, context, next)),
                list: (0..width).map(|_| expression(rng, context, next)).collect(),
                negated: rng.chance(1, 3),
            }
        }
        _ => Expr::Binary {
            op: rng.pick(&COMPARISONS).copied().unwrap_or("="),
            left: Box::new(expression(rng, context, next)),
            right: Box::new(expression(rng, context, next)),
        },
    }
}

/// An expression of any type.
fn expression(rng: &mut Rng, context: &Context<'_>, depth: u32) -> Expr {
    if depth == 0 {
        return leaf(rng, context);
    }
    let next = depth.saturating_sub(1);
    match rng.below(10) {
        0 | 1 => leaf(rng, context),
        2 | 3 => Expr::Binary {
            op: rng.pick(&ARITHMETIC).copied().unwrap_or("+"),
            left: Box::new(expression(rng, context, next)),
            right: Box::new(expression(rng, context, next)),
        },
        4 => Expr::Prefix {
            op: match rng.below(3) {
                0 => "-",
                1 => "+",
                _ => "~",
            },
            operand: Box::new(expression(rng, context, next)),
        },
        5 => Expr::Suffix {
            op: match rng.below(3) {
                0 => "COLLATE BINARY",
                1 => "COLLATE NOCASE",
                _ => "COLLATE RTRIM",
            },
            operand: Box::new(expression(rng, context, next)),
        },
        6 => Expr::Cast {
            operand: Box::new(expression(rng, context, next)),
            ty: rng.pick(&TYPES).copied().unwrap_or("TEXT"),
        },
        7 => {
            let (name, arity) = rng.pick(&FUNCTIONS).copied().unwrap_or(("abs", 1));
            Expr::Call {
                name,
                args: (0..arity).map(|_| expression(rng, context, next)).collect(),
            }
        }
        _ => predicate(rng, context, next),
    }
}

/// A column or a literal. The literal is drawn from the stored data half
/// the time, so that comparisons are met rather than always false.
fn leaf(rng: &mut Rng, context: &Context<'_>) -> Expr {
    if rng.chance(1, 2) {
        return column(rng, context);
    }
    if rng.chance(1, 2)
        && let Some(value) = stored(rng, context)
    {
        return Expr::Literal(value);
    }
    Expr::Literal(value(rng))
}

/// A reference to one visible column.
fn column(rng: &mut Rng, context: &Context<'_>) -> Expr {
    let table = rng.pick(&context.visible).copied().unwrap_or(0);
    let width = context
        .tables
        .get(table)
        .map_or(1, |table| table.columns.len());
    let at = rng.index(width).unwrap_or(0);
    Expr::Column(ColumnRef { table, column: at })
}

/// A value that is stored in one of the visible tables.
fn stored(rng: &mut Rng, context: &Context<'_>) -> Option<Value> {
    let table = context.tables.get(rng.pick(&context.visible).copied()?)?;
    let row = rng.pick(&table.rows)?;
    rng.pick(row).cloned()
}
