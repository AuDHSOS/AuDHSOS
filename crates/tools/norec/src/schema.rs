// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The database a test case creates: tables, their columns and rows, and
//! the indexes over them.

use std::fmt::Write as _;

use crate::expr::{Expr, Names};
use crate::value::{Value, tuple};

/// The type affinity a column is declared with. It decides how a stored
/// value is converted, and a comparison across two affinities is where
/// SQLite's conversion rules become visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Affinity {
    /// `INTEGER`.
    Integer,
    /// `REAL`.
    Real,
    /// `TEXT`.
    Text,
    /// `NUMERIC`.
    Numeric,
    /// `BLOB`.
    Blob,
    /// No declared type, which is `BLOB` affinity under another name and
    /// is written differently.
    None,
}

impl Affinity {
    /// The type name as it is declared, empty for a column without one.
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            Affinity::Integer => "INTEGER",
            Affinity::Real => "REAL",
            Affinity::Text => "TEXT",
            Affinity::Numeric => "NUMERIC",
            Affinity::Blob => "BLOB",
            Affinity::None => "",
        }
    }
}

/// One column.
#[derive(Clone, Debug)]
pub(crate) struct Column {
    /// Its name, `c0` upwards.
    pub(crate) name: String,
    /// Its declared affinity.
    pub(crate) affinity: Affinity,
    /// A collating sequence, when one is declared.
    pub(crate) collation: Option<&'static str>,
}

/// One index over a table.
#[derive(Clone, Debug)]
pub(crate) struct Index {
    /// Its name, `i0` upwards.
    pub(crate) name: String,
    /// Whether it is `UNIQUE`.
    pub(crate) unique: bool,
    /// The indexed terms: a column, or an expression over columns.
    pub(crate) terms: Vec<Expr>,
    /// The `WHERE` clause of a partial index.
    pub(crate) filter: Option<Expr>,
}

/// One table with its rows.
#[derive(Clone, Debug)]
pub(crate) struct Table {
    /// Its name, `t0` upwards.
    pub(crate) name: String,
    /// Its columns, at least one.
    pub(crate) columns: Vec<Column>,
    /// Its rows, each as wide as `columns`.
    pub(crate) rows: Vec<Vec<Value>>,
    /// The indexes over it.
    pub(crate) indexes: Vec<Index>,
}

impl Table {
    /// The `CREATE TABLE` statement.
    pub(crate) fn create(&self) -> String {
        let mut out = format!("CREATE TABLE {}(", self.name);
        for (at, column) in self.columns.iter().enumerate() {
            if at > 0 {
                out.push_str(", ");
            }
            out.push_str(&column.name);
            if !column.affinity.keyword().is_empty() {
                let _ = write!(out, " {}", column.affinity.keyword());
            }
            if let Some(collation) = column.collation {
                let _ = write!(out, " COLLATE {collation}");
            }
        }
        out.push_str(");");
        out
    }

    /// The one `INSERT` that fills the table, or nothing when it is empty.
    pub(crate) fn insert(&self) -> Option<String> {
        if self.rows.is_empty() {
            return None;
        }
        let mut out = format!("INSERT INTO {} VALUES ", self.name);
        for (at, row) in self.rows.iter().enumerate() {
            if at > 0 {
                out.push_str(", ");
            }
            out.push_str(&tuple(row));
        }
        out.push(';');
        Some(out)
    }

    /// The `CREATE INDEX` statements, in order.
    pub(crate) fn index_statements(&self, tables: &[Table]) -> Vec<String> {
        self.indexes
            .iter()
            .map(|index| {
                let mut out = String::from("CREATE ");
                if index.unique {
                    out.push_str("UNIQUE ");
                }
                let _ = write!(out, "INDEX {} ON {}(", index.name, self.name);
                for (at, term) in index.terms.iter().enumerate() {
                    if at > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&term.render(tables, Names::Bare));
                }
                out.push(')');
                if let Some(filter) = &index.filter {
                    let _ = write!(out, " WHERE {}", filter.render(tables, Names::Bare));
                }
                out.push(';');
                out
            })
            .collect()
    }
}
