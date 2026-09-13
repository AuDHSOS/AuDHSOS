// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The values a generated table holds, and how each is written as a
//! literal.

use std::fmt::Write as _;

/// One stored value. SQLite stores any type in any column, so this is the
/// storage class and not the column's affinity.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    /// `NULL`.
    Null,
    /// A 64-bit integer.
    Int(i64),
    /// A double. Never a NaN or an infinity: neither has a literal that
    /// reads back as itself.
    Real(f64),
    /// Text, from the generator's alphabet.
    Text(String),
}

impl Value {
    /// The value as SQL. A text value is quoted with `''` for the quote,
    /// which is the only escape SQLite string literals have.
    pub(crate) fn literal(&self) -> String {
        match self {
            Value::Int(value) => value.to_string(),
            Value::Real(value) if value.is_finite() => {
                let mut text = format!("{value:?}");
                if !text.contains(['.', 'e', 'E']) {
                    text.push_str(".0");
                }
                text
            }
            // A non-finite double cannot be written as a literal that reads
            // back; the generator makes none, and this is the safe answer.
            Value::Null | Value::Real(_) => "NULL".to_owned(),
            Value::Text(text) => {
                let mut out = String::with_capacity(text.len().saturating_add(2));
                out.push('\'');
                for character in text.chars() {
                    if character == '\'' {
                        out.push('\'');
                    }
                    out.push(character);
                }
                out.push('\'');
                out
            }
        }
    }
}

/// A row as a `VALUES` tuple.
pub(crate) fn tuple(row: &[Value]) -> String {
    let mut out = String::from("(");
    for (at, value) in row.iter().enumerate() {
        if at > 0 {
            out.push_str(", ");
        }
        let _ = write!(out, "{}", value.literal());
    }
    out.push(')');
    out
}
