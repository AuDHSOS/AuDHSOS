// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unit tests, kept out of the product sources so that coverage measures
//! product code only.

mod campaign;
mod case;
mod engine;
mod expr;
mod generate;
mod options;
mod oracle;
mod reduce;
mod report;
mod rng;
mod schema;
mod value;

use std::path::PathBuf;

use crate::case::{Case, JoinKind, Source};
use crate::engine::{Engine, Run};
use crate::error::Error;
use crate::expr::{ColumnRef, Expr};
use crate::schema::{Affinity, Column, Table};
use crate::value::Value;

/// An engine whose answer is computed from the script it is given.
pub(super) struct Fake<F> {
    /// What it answers with.
    pub(super) answer: F,
}

impl<F: FnMut(&str) -> Run> Engine for Fake<F> {
    fn run(&mut self, script: &str) -> Result<Run, Error> {
        Ok((self.answer)(script))
    }
}

/// The output of an engine that printed `rows` for the optimized query and
/// `sum` for the unoptimized one.
pub(super) fn answer(rows: &[&str], sum: &str) -> Run {
    let mut stdout = String::from("--norec:optimized--\n");
    for row in rows {
        stdout.push_str(row);
        stdout.push('\n');
    }
    stdout.push_str("--norec:unoptimized--\n");
    stdout.push_str(sum);
    stdout.push_str("\n--norec:end--\n");
    Run {
        stdout,
        stderr: String::new(),
        timed_out: false,
    }
}

/// An engine that agrees with itself over every case.
pub(super) fn agreeing() -> impl Engine {
    Fake {
        answer: |_: &str| answer(&["1"], "1"),
    }
}

/// A table `t0` with two integer columns and `rows` rows of small values.
pub(super) fn table(name: &str, rows: usize) -> Table {
    Table {
        name: name.to_owned(),
        columns: vec![
            Column {
                name: "c0".to_owned(),
                affinity: Affinity::Integer,
                collation: None,
            },
            Column {
                name: "c1".to_owned(),
                affinity: Affinity::Text,
                collation: None,
            },
        ],
        rows: (0..rows)
            .map(|row| {
                vec![
                    Value::Int(i64::try_from(row).unwrap_or(0)),
                    Value::Text(format!("r{row}")),
                ]
            })
            .collect(),
        indexes: Vec::new(),
    }
}

/// A column of the first table.
pub(super) fn column(table: usize, column: usize) -> Expr {
    Expr::Column(ColumnRef { table, column })
}

/// A case over one table with `rows` rows and the predicate `c0 = 1`.
pub(super) fn case(rows: usize) -> Case {
    Case {
        tables: vec![table("t0", rows)],
        source: Source {
            first: 0,
            joins: Vec::new(),
        },
        predicate: Expr::Binary {
            op: "=",
            left: Box::new(column(0, 0)),
            right: Box::new(Expr::Literal(Value::Int(1))),
        },
        analyze: false,
    }
}

/// A join of `t1` onto the case.
pub(super) fn joined(mut case: Case, kind: JoinKind) -> Case {
    case.tables.push(table("t1", 1));
    case.source.joins.push(crate::case::Join {
        kind,
        table: 1,
        on: match kind {
            JoinKind::Comma => None,
            _ => Some(Expr::Binary {
                op: "=",
                left: Box::new(column(0, 0)),
                right: Box::new(column(1, 0)),
            }),
        },
    });
    case
}

/// A directory of this test's own under the temporary directory.
pub(super) struct Scratch(pub(super) PathBuf);

impl Scratch {
    /// A fresh directory, named after the process and a counter.
    pub(super) fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("norec-{}-{id}", std::process::id()));
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
