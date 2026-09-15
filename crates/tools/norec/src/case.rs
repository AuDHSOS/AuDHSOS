// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One test case: a database, a `FROM` clause, and the predicate the two
//! queries of NoREC are built from.

use std::fmt::Write as _;

use crate::expr::{Expr, Names};
use crate::schema::Table;

/// How two tables are joined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JoinKind {
    /// `t0, t1`.
    Comma,
    /// `t0 JOIN t1 ON ...`.
    Inner,
    /// `t0 LEFT JOIN t1 ON ...`.
    Left,
}

/// A table added to the `FROM` clause.
#[derive(Clone, Debug)]
pub(crate) struct Join {
    /// How it is joined.
    pub(crate) kind: JoinKind,
    /// Index into the tables of the case.
    pub(crate) table: usize,
    /// The `ON` condition, which a comma join has not.
    pub(crate) on: Option<Expr>,
}

/// The `FROM` clause: one table and what is joined to it. Section 3.2 of
/// the paper copies it into the unoptimized query unchanged, which is what
/// this type exists for.
#[derive(Clone, Debug)]
pub(crate) struct Source {
    /// Index of the leading table.
    pub(crate) first: usize,
    /// The tables joined to it, in order.
    pub(crate) joins: Vec<Join>,
}

/// How the rows of the optimized query are counted. Section 3.3 of the
/// paper alternates between the two: the count is cheaper, and the plain
/// row set is the query an optimizer sees plainly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CountStyle {
    /// `SELECT *`, counted by the rows that come back.
    Rows,
    /// `SELECT COUNT(*)`, counted by the engine.
    Count,
}

/// A database, a `FROM` clause and a predicate.
#[derive(Clone, Debug)]
pub(crate) struct Case {
    /// Every table, whether or not the `FROM` clause names it.
    pub(crate) tables: Vec<Table>,
    /// What the two queries select from.
    pub(crate) source: Source,
    /// The predicate the optimized query filters with.
    pub(crate) predicate: Expr,
    /// Whether `ANALYZE` runs before the queries, which changes the plans
    /// the optimizer chooses.
    pub(crate) analyze: bool,
}

impl Case {
    /// The statements that build the database, in order.
    pub(crate) fn setup(&self) -> Vec<String> {
        let mut out = Vec::new();
        for table in &self.tables {
            out.push(table.create());
            if let Some(insert) = table.insert() {
                out.push(insert);
            }
            out.extend(table.index_statements(&self.tables));
        }
        if self.analyze {
            out.push("ANALYZE;".to_owned());
        }
        out
    }

    /// The `FROM` clause both queries share.
    pub(crate) fn source_clause(&self) -> String {
        let mut out = self
            .tables
            .get(self.source.first)
            .map_or_else(|| "sqlite_schema".to_owned(), |table| table.name.clone());
        for join in &self.source.joins {
            let name = self
                .tables
                .get(join.table)
                .map_or_else(|| "sqlite_schema".to_owned(), |table| table.name.clone());
            let on = join
                .on
                .as_ref()
                .map_or_else(|| "1".to_owned(), |expr| self.render(expr));
            match join.kind {
                JoinKind::Comma => {
                    let _ = write!(out, ", {name}");
                }
                JoinKind::Inner => {
                    let _ = write!(out, " JOIN {name} ON {on}");
                }
                JoinKind::Left => {
                    let _ = write!(out, " LEFT JOIN {name} ON {on}");
                }
            }
        }
        out
    }

    /// The optimized query: the predicate in the `WHERE` clause, where the
    /// optimizer works (section 3.1).
    pub(crate) fn optimized(&self, style: CountStyle) -> String {
        let selected = match style {
            CountStyle::Rows => "*",
            CountStyle::Count => "COUNT(*)",
        };
        format!(
            "SELECT {selected} FROM {} WHERE {};",
            self.source_clause(),
            self.render(&self.predicate)
        )
    }

    /// The unoptimized query: the same predicate evaluated on every row of
    /// the same `FROM` clause, summed (sections 3.2 and 3.3).
    pub(crate) fn unoptimized(&self) -> String {
        format!(
            "SELECT SUM(count) FROM (SELECT ({}) IS TRUE AS count FROM {});",
            self.render(&self.predicate),
            self.source_clause()
        )
    }

    /// An expression as SQL, with column names qualified by their table.
    pub(crate) fn render(&self, expr: &Expr) -> String {
        expr.render(&self.tables, Names::Qualified)
    }

    /// How large the case is: rows, indexes and predicate nodes. Reduction
    /// accepts a candidate only when this falls.
    pub(crate) fn size(&self) -> usize {
        self.tables
            .iter()
            .fold(self.predicate.size(), |total, table| {
                total
                    .saturating_add(table.rows.len())
                    .saturating_add(table.indexes.len())
            })
            .saturating_add(self.source.joins.len())
            .saturating_add(usize::from(self.analyze))
    }
}
