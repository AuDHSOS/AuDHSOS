// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the walks and the sorts of one statement count, and which
//! `ORDER BY` the walk answers without a sort.
//!
//! A walk of a whole table and a walk of a whole index each count one
//! step per row after the first, which is what `OP_Next` counts; a walk
//! held to a key counts none. A sort counts one, which is what
//! `OP_Sort` counts.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use crate::db::{Database, Stepped};

/// What `sql` counted over the fixture of `tests::index`.
fn counted(sql: &[u8]) -> Stepped {
    Database::open(super::INDEXED)
        .unwrap()
        .query(sql)
        .unwrap()
        .stepped
}

/// The steps and the sorts as a pair.
fn pair(sql: &[u8]) -> (u64, u64) {
    let held = counted(sql);
    (held.steps, held.sorts)
}

#[test]
fn a_walk_of_a_whole_table_counts_one_step_per_row_after_the_first() {
    assert_eq!(pair(b"SELECT * FROM m"), (4, 0));
    assert_eq!(pair(b"SELECT * FROM m WHERE q='A'"), (0, 0));
    assert_eq!(pair(b"SELECT * FROM m WHERE q='A' ORDER BY p"), (0, 1));
}

#[test]
fn a_walk_held_to_a_key_counts_no_step() {
    assert_eq!(pair(b"SELECT * FROM m WHERE q='b'"), (0, 0));
    assert_eq!(pair(b"SELECT * FROM m WHERE rowid>2"), (0, 0));
    assert_eq!(pair(b"SELECT * FROM m WHERE q='b' OR p=2"), (0, 0));
    assert_eq!(pair(b"SELECT * FROM m, k ON k.a=m.p"), (4, 0));
}

#[test]
fn an_order_by_the_walk_of_the_tables_own_tree_answers_needs_no_sort() {
    assert_eq!(pair(b"SELECT * FROM m ORDER BY rowid"), (4, 0));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY rowid, q"), (4, 0));
}

#[test]
fn an_order_by_the_walk_of_an_index_answers_needs_no_sort() {
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q"), (4, 0));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q, rowid"), (4, 0));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY p, q"), (4, 0));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY p"), (4, 0));
}

#[test]
fn what_order_by_the_walk_does_not_answer() {
    // `mr` holds its entries backwards, and no index holds `q` and `r`
    // in one order.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY r"), (4, 1));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q, r"), (4, 1));
    // A term written backwards, with its nulls moved, over an
    // expression, or over a column under a collation of its own.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q DESC"), (4, 1));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q NULLS LAST"), (4, 1));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY r+1"), (4, 1));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q COLLATE binary"), (4, 1));
    // A rowid between two columns leaves the terms after it unanswered.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q, rowid, p"), (4, 1));
    // A side already held to a key answers fewer rows than a walk of
    // the index whole.
    assert_eq!(pair(b"SELECT * FROM m WHERE q='A' ORDER BY p"), (0, 1));
    // A rowid the walk answers, where the walk is not of the table.
    assert_eq!(pair(b"SELECT * FROM m WHERE q='A' ORDER BY rowid"), (0, 1));
    // A table that keeps its rows in the key's own tree, a statement
    // inside the `FROM`, a join, a group, and a window.
    assert_eq!(pair(b"SELECT * FROM u ORDER BY a"), (1, 1));
    assert_eq!(pair(b"SELECT * FROM (SELECT * FROM m) ORDER BY p"), (4, 1));
    assert_eq!(pair(b"SELECT m.p FROM m, k ORDER BY m.p"), (499, 1));
    assert_eq!(pair(b"SELECT p FROM m GROUP BY p ORDER BY p"), (4, 1));
    assert_eq!(pair(b"SELECT count(*) OVER () FROM m ORDER BY 1"), (4, 1));
}

#[test]
fn an_index_that_holds_fewer_entries_than_the_table_has_rows_answers_no_order() {
    // `ea` holds what an expression answers, `eb` holds the rows a
    // `WHERE` answers, and `ec` holds its entries under another
    // collation than the column compares under.
    assert_eq!(pair(b"SELECT * FROM e ORDER BY a"), (2, 1));
    assert_eq!(pair(b"SELECT * FROM e ORDER BY b"), (2, 1));
}

#[test]
fn the_order_by_of_a_compound_statement_counts_one_sort() {
    assert_eq!(
        pair(b"SELECT p FROM m UNION ALL SELECT a FROM k ORDER BY 1"),
        (103, 1)
    );
}
