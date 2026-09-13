// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The expression tree a predicate is, and how it is written as SQL.
//!
//! Every node is parenthesized when it is written, so the text means what
//! the tree says and no precedence rule of SQLite has to be reproduced
//! here. The literal `NULL` is what an unknown name renders as, which
//! cannot arise from a generated tree and keeps rendering total.

use std::fmt::Write as _;

use crate::schema::Table;
use crate::value::Value;

/// A column of a table of the database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ColumnRef {
    /// Index into the tables of the case.
    pub(crate) table: usize,
    /// Index into the columns of that table.
    pub(crate) column: usize,
}

/// How a column name is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Names {
    /// `t0.c1`, which a query over several tables needs.
    Qualified,
    /// `c1`, which is what an index term may use.
    Bare,
}

/// One node of a predicate.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expr {
    /// A column.
    Column(ColumnRef),
    /// A constant.
    Literal(Value),
    /// `NOT x`, `-x`, `~x`.
    Prefix {
        /// The operator, as it is written.
        op: &'static str,
        /// What it applies to.
        operand: Box<Expr>,
    },
    /// `x = y`, `x AND y`, `x || y`.
    Binary {
        /// The operator, as it is written.
        op: &'static str,
        /// Left operand.
        left: Box<Expr>,
        /// Right operand.
        right: Box<Expr>,
    },
    /// `x IS NULL`, `x COLLATE NOCASE`.
    Suffix {
        /// The operator, as it is written.
        op: &'static str,
        /// What it applies to.
        operand: Box<Expr>,
    },
    /// `x BETWEEN low AND high`.
    Between {
        /// The value tested.
        value: Box<Expr>,
        /// Lower bound.
        low: Box<Expr>,
        /// Upper bound.
        high: Box<Expr>,
        /// Whether `NOT` precedes `BETWEEN`.
        negated: bool,
    },
    /// `x IN (a, b)`.
    In {
        /// The value tested.
        value: Box<Expr>,
        /// The list, never empty.
        list: Vec<Expr>,
        /// Whether `NOT` precedes `IN`.
        negated: bool,
    },
    /// `CAST(x AS INTEGER)`.
    Cast {
        /// What is converted.
        operand: Box<Expr>,
        /// The target type name.
        ty: &'static str,
    },
    /// `abs(x)`, `coalesce(x, y)`.
    Call {
        /// The function name.
        name: &'static str,
        /// Its arguments.
        args: Vec<Expr>,
    },
}

impl Expr {
    /// The node as SQL, fully parenthesized.
    pub(crate) fn render(&self, tables: &[Table], names: Names) -> String {
        match self {
            Expr::Column(reference) => column_name(tables, *reference, names),
            Expr::Literal(value) => value.literal(),
            Expr::Prefix { op, operand } => {
                format!("({op} {})", operand.render(tables, names))
            }
            Expr::Binary { op, left, right } => format!(
                "({} {op} {})",
                left.render(tables, names),
                right.render(tables, names)
            ),
            Expr::Suffix { op, operand } => {
                format!("({} {op})", operand.render(tables, names))
            }
            Expr::Between {
                value,
                low,
                high,
                negated,
            } => format!(
                "({} {}BETWEEN {} AND {})",
                value.render(tables, names),
                if *negated { "NOT " } else { "" },
                low.render(tables, names),
                high.render(tables, names)
            ),
            Expr::In {
                value,
                list,
                negated,
            } => {
                let mut out = format!(
                    "({} {}IN (",
                    value.render(tables, names),
                    if *negated { "NOT " } else { "" }
                );
                for (at, item) in list.iter().enumerate() {
                    if at > 0 {
                        out.push_str(", ");
                    }
                    let _ = write!(out, "{}", item.render(tables, names));
                }
                out.push_str("))");
                out
            }
            Expr::Cast { operand, ty } => {
                format!("CAST({} AS {ty})", operand.render(tables, names))
            }
            Expr::Call { name, args } => {
                let mut out = format!("{name}(");
                for (at, arg) in args.iter().enumerate() {
                    if at > 0 {
                        out.push_str(", ");
                    }
                    let _ = write!(out, "{}", arg.render(tables, names));
                }
                out.push(')');
                out
            }
        }
    }

    /// The subexpressions, in the order they are written.
    pub(crate) fn children(&self) -> Vec<&Expr> {
        match self {
            Expr::Column(_) | Expr::Literal(_) => Vec::new(),
            Expr::Prefix { operand, .. }
            | Expr::Suffix { operand, .. }
            | Expr::Cast { operand, .. } => vec![operand],
            Expr::Binary { left, right, .. } => vec![left, right],
            Expr::Between {
                value, low, high, ..
            } => vec![value, low, high],
            Expr::In { value, list, .. } => {
                let mut out = vec![value.as_ref()];
                out.extend(list.iter());
                out
            }
            Expr::Call { args, .. } => args.iter().collect(),
        }
    }

    /// The same node with child `at` replaced. Out of range, it is
    /// unchanged.
    pub(crate) fn with_child(&self, at: usize, child: Expr) -> Expr {
        let mut copy = self.clone();
        match &mut copy {
            Expr::Column(_) | Expr::Literal(_) => {}
            Expr::Prefix { operand, .. }
            | Expr::Suffix { operand, .. }
            | Expr::Cast { operand, .. } => {
                if at == 0 {
                    **operand = child;
                }
            }
            Expr::Binary { left, right, .. } => match at {
                0 => **left = child,
                1 => **right = child,
                _ => {}
            },
            Expr::Between {
                value, low, high, ..
            } => match at {
                0 => **value = child,
                1 => **low = child,
                2 => **high = child,
                _ => {}
            },
            Expr::In { value, list, .. } => {
                if at == 0 {
                    **value = child;
                } else if let Some(slot) = list.get_mut(at.saturating_sub(1)) {
                    *slot = child;
                }
            }
            Expr::Call { args, .. } => {
                if let Some(slot) = args.get_mut(at) {
                    *slot = child;
                }
            }
        }
        copy
    }

    /// Whether any node is a column. An index term that is only
    /// constants is not one: SQLite reads a string literal where a column
    /// name belongs as an identifier and refuses it.
    pub(crate) fn mentions_column(&self) -> bool {
        matches!(self, Expr::Column(_))
            || self.children().iter().any(|child| child.mentions_column())
    }

    /// The number of nodes. O(n).
    pub(crate) fn size(&self) -> usize {
        self.children()
            .iter()
            .fold(1, |total, child| total.saturating_add(child.size()))
    }

    /// Every one-step simplification, each strictly smaller than the tree
    /// it came from: the node replaced by one of its children or by a
    /// constant, and the same for each child in place.
    /// O(n) nodes each of size O(n), so O(n²) in the worst case, over
    /// trees the generator bounds to a handful of nodes.
    pub(crate) fn candidates(&self) -> Vec<Expr> {
        let mut out: Vec<Expr> = self.children().into_iter().cloned().collect();
        // A constant in place of a whole node, unless the node is one node
        // already: a candidate that is not smaller is never taken.
        if self.size() > 1 {
            out.push(Expr::Literal(Value::Null));
            out.push(Expr::Literal(Value::Int(1)));
        }
        for (at, child) in self.children().into_iter().enumerate() {
            for candidate in child.candidates() {
                out.push(self.with_child(at, candidate));
            }
        }
        out
    }
}

/// The name of a column, qualified or bare.
fn column_name(tables: &[Table], reference: ColumnRef, names: Names) -> String {
    let Some(table) = tables.get(reference.table) else {
        return "NULL".to_owned();
    };
    let Some(column) = table.columns.get(reference.column) else {
        return "NULL".to_owned();
    };
    match names {
        Names::Qualified => format!("{}.{}", table.name, column.name),
        Names::Bare => column.name.clone(),
    }
}
