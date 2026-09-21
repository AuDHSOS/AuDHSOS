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
fn an_order_by_the_walk_of_an_index_read_backwards_answers_needs_no_sort() {
    // `mq` holds `q` forwards and `mr` holds `r` backwards, so each
    // answers the other order read from its last entry to its first.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q DESC"), (4, 0));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY r"), (4, 0));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY p DESC, q DESC"), (4, 0));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q DESC, rowid DESC"), (4, 0));
}

#[test]
fn what_column_of_the_answer_a_term_of_an_order_by_counts_to() {
    // A whole number counts the answered columns from one, and a name
    // one of them is answered under names it.
    assert_eq!(pair(b"SELECT q FROM m ORDER BY 1"), (4, 0));
    assert_eq!(pair(b"SELECT q AS z FROM m ORDER BY z"), (4, 0));
    assert_eq!(pair(b"SELECT p, q FROM m ORDER BY 1, 2"), (4, 0));
    // A `*` answers as many columns as its table has, so the number
    // counts to a column no result column stands for.
    assert_eq!(pair(b"SELECT *, q FROM m ORDER BY 4"), (4, 1));
    // The name answers what the alias names and not the column of that
    // name, so `mr` and not `mq` answers this order.
    assert_eq!(pair(b"SELECT r AS q FROM m ORDER BY q"), (4, 0));
    // A name no alias of the statement spells is the column of that
    // name.
    assert_eq!(pair(b"SELECT q AS z, p FROM m ORDER BY p"), (4, 0));
}

#[test]
fn what_collation_a_term_of_an_order_by_compares_under() {
    // `q` compares under `NOCASE` and `mq` holds it under that
    // collation, so a term naming it answers the walk.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q COLLATE nocase"), (4, 0));
    assert_eq!(
        pair(b"SELECT * FROM m ORDER BY q COLLATE NOCASE DESC"),
        (4, 0)
    );
    // A collation the index does not hold, one over the rowid, and one
    // the connection does not define.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q COLLATE rtrim"), (4, 1));
    assert_eq!(
        pair(b"SELECT * FROM m ORDER BY rowid COLLATE binary"),
        (4, 1)
    );
    assert!(
        Database::open(super::INDEXED)
            .unwrap()
            .query(b"SELECT * FROM m ORDER BY q COLLATE nope")
            .is_err()
    );
}

#[test]
fn what_a_term_over_the_rowid_after_the_columns_answers() {
    // The rowid stands after every column of the index and runs from
    // the smallest up.
    assert_eq!(pair(b"SELECT * FROM m WHERE p=1 ORDER BY q, rowid"), (0, 0));
    assert_eq!(
        pair(b"SELECT * FROM m WHERE p=1 ORDER BY q, rowid DESC"),
        (0, 1)
    );
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q, rowid DESC"), (4, 1));
    // `mpq` holds `q` after `p`, so the rowid does not come next, and
    // a walk of it held between bounds reaches no further.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY p, rowid"), (4, 1));
    assert_eq!(pair(b"SELECT * FROM m WHERE p>1 ORDER BY p, rowid"), (0, 1));
}

#[test]
fn what_order_by_the_walk_does_not_answer() {
    // No index holds `q` and `r` in one order.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q, r"), (4, 1));
    // A term with its nulls moved, over an expression, or over a column
    // under a collation of its own.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q NULLS LAST"), (4, 1));
    // Two terms running in two directions, and a rowid read backwards.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY p, q DESC"), (4, 1));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY rowid DESC"), (4, 1));
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
