// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ANALYZE`: what the tables and their indexes hold, counted into the
//! rows of `sqlite_stat1`.
//!
//! `analyzeOneTable` in `src/analyze.c` walks every index of a table in
//! its order and counts, for each prefix of the index columns, how many
//! entries share the values of that prefix. The row it writes holds the
//! number of rows and then, per prefix, the number of rows a lookup of
//! that prefix is expected to answer, which is `statGet`.
//!
//! Counting one index is one pass over its entries in order, so a table
//! of `n` rows and `k` index columns costs O(n log n) to order and
//! O(n k) to count.

use alloc::vec::Vec;

use crate::db::{Database, Error};
use crate::schema::Index;
use crate::value::{Collation, Value};

/// One row of `sqlite_stat1`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stat {
    /// The table the row counts.
    pub table: Vec<u8>,
    /// The index it counts, and nothing where the table carries none.
    pub index: Option<Vec<u8>>,
    /// The counts, as `sqlite_stat1` holds them.
    pub stat: Vec<u8>,
}

/// The rows `ANALYZE` writes for one table: one per index, the newest
/// index first, and one naming no index where the table carries none.
/// `only` holds the run to the one index it names.
///
/// A table with no row is counted into no row at all, which is what
/// `analyzeOneTable` leaves when the loop over the table ends at once.
///
/// # Errors
///
/// [`Error`] names what reading the table refuses.
pub fn stats_of(
    database: &Database<'_>,
    table: &[u8],
    only: Option<&[u8]>,
) -> Result<Vec<Stat>, Error> {
    let rows = database.rows_of(table)?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let indexes = database.indexes(table);
    if indexes.is_empty() {
        return Ok(alloc::vec![Stat {
            table: table.to_vec(),
            index: None,
            stat: crate::number::integer_text(counted(rows.len())),
        }]);
    }
    let mut out = Vec::new();
    // `pTab->pIndex` carries the index made last first, which is the
    // order the rows are written in.
    for (index, _) in indexes.iter().rev() {
        if only.is_some_and(|name| !index.name.eq_ignore_ascii_case(name)) {
            continue;
        }
        out.push(Stat {
            table: table.to_vec(),
            index: Some(index.name.clone()),
            stat: stat_of(index, &rows),
        });
    }
    Ok(out)
}

/// The counts of one index: the number of rows, and per prefix of its
/// columns the number of rows a lookup of that prefix answers, which
/// `statGet` rounds up.
fn stat_of(index: &Index, rows: &[(i64, Vec<Value>)]) -> Vec<u8> {
    let collations = crate::change::collations_of(index);
    let mut keys: Vec<Vec<Value>> = rows
        .iter()
        .map(|(rowid, values)| crate::change::entry_of(index, values, &[Value::Int(*rowid)]))
        .collect();
    keys.sort_by(|one, other| crate::change::order_of_keys(one, other, &collations));
    let columns = index.columns.len();
    // One entry of its own is one value of its own for every prefix,
    // so every prefix begins at one and counts the entries that differ
    // from the one before them.
    let mut distinct = alloc::vec![1_u64; columns];
    for (one, other) in keys.iter().zip(keys.iter().skip(1)) {
        // The entries are in order, so the first column two of them
        // differ in is the first prefix they are two values of, and
        // every longer prefix is two values as well.
        let first = one
            .iter()
            .zip(other)
            .take(columns)
            .enumerate()
            .find(|(at, (mine, theirs))| {
                let collation = collations.get(*at).copied().unwrap_or(Collation::Binary);
                crate::value::compare(mine, theirs, collation) != core::cmp::Ordering::Equal
            })
            .map(|(at, _)| at);
        if let Some(at) = first {
            for count in distinct.iter_mut().skip(at) {
                *count = count.saturating_add(1);
            }
        }
    }
    let count = counted(rows.len());
    let mut out = crate::number::integer_text(count);
    for values in &distinct {
        out.push(b' ');
        // `(nRow + nDistinct - 1) / nDistinct`, which is the count
        // rounded up.
        let rounded = count
            .cast_unsigned()
            .saturating_add(values.saturating_sub(1))
            .checked_div(*values)
            .unwrap_or(0);
        out.extend_from_slice(&crate::number::integer_text(rounded.cast_signed()));
    }
    out
}

/// How many rows a count of `rows` is, as a number a row holds.
fn counted(rows: usize) -> i64 {
    i64::try_from(rows).unwrap_or(i64::MAX)
}
