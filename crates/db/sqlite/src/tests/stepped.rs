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
    // The walk of the tree from its last row back answers the rowids
    // from the largest down, which a `LIMIT` of one takes one row of.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY rowid DESC"), (4, 0));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY rowid DESC LIMIT 1"), (0, 0));
    // A walk held to a range of rowids is not that walk.
    assert_eq!(
        pair(b"SELECT * FROM m WHERE rowid>2 ORDER BY rowid DESC"),
        (0, 1)
    );
}

/// The rows a walk of the tree from its last row back answers, and the
/// rowids they carry. Every answer is the one the C library's shell
/// writes.
#[test]
fn what_the_walk_from_the_last_row_back_answers() {
    let rows = Database::open(super::INDEXED)
        .unwrap()
        .query(b"SELECT rowid FROM m ORDER BY rowid DESC")
        .unwrap()
        .rows;
    let held: Vec<i64> = rows
        .iter()
        .flatten()
        .map(|value| match value {
            crate::value::Value::Int(held) => *held,
            _ => panic!("a rowid is a whole number"),
        })
        .collect();
    assert_eq!(held, [5, 4, 3, 2, 1]);
}

#[test]
fn an_order_by_the_walk_of_an_index_answers_needs_no_sort() {
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q"), (4, 0));
    // A rowid stands once in the table, so the terms after one say
    // nothing about the order.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q, rowid, p"), (4, 0));
    // An entry of an index ends with the rowid, so a walk held to one
    // value of every column of the index answers the rowids in order.
    assert_eq!(pair(b"SELECT * FROM m WHERE q='A' ORDER BY rowid"), (0, 0));
    assert_eq!(
        pair(b"SELECT * FROM m WHERE q='A' ORDER BY rowid DESC"),
        (0, 0)
    );
    assert_eq!(pair(b"SELECT * FROM m WHERE p=1 ORDER BY rowid"), (0, 1));
    // A walk of several branches of an `OR` answers the rows of each
    // branch one after another, which is no order of rowids.
    assert_eq!(
        pair(b"SELECT * FROM m WHERE q='A' OR p=1 ORDER BY rowid"),
        (0, 1)
    );
    assert_eq!(pair(b"SELECT * FROM m ORDER BY rowid, q DESC"), (4, 0));
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
fn what_a_number_that_counts_to_the_key_of_the_table_asks_of_the_walk() {
    // `r` keeps its rows by `id`, so the walk of its own tree answers
    // the order a number counting to that column asks for, and the walk
    // answers no order of the column after it.
    let keyed = |sql: &[u8]| {
        let held = Database::open(super::KEYS)
            .unwrap()
            .query(sql)
            .unwrap()
            .stepped;
        (held.steps, held.sorts)
    };
    assert_eq!(keyed(b"SELECT * FROM r ORDER BY 1"), (2, 0));
    assert_eq!(keyed(b"SELECT * FROM r ORDER BY 2"), (2, 1));
}

#[test]
fn what_column_of_the_answer_a_term_of_an_order_by_counts_to() {
    // A whole number counts the answered columns from one, and a name
    // one of them is answered under names it.
    assert_eq!(pair(b"SELECT q FROM m ORDER BY 1"), (4, 0));
    assert_eq!(pair(b"SELECT q AS z FROM m ORDER BY z"), (4, 0));
    assert_eq!(pair(b"SELECT p, q FROM m ORDER BY 1, 2"), (4, 0));
    // The number counts the columns a `*` stands for, so `mq` answers
    // both of these orders.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY 2"), (4, 0));
    assert_eq!(pair(b"SELECT *, q FROM m ORDER BY 4"), (4, 0));
    // A statement of two sides sorts, no walk of one side answering an
    // order over the columns of both.
    assert_eq!(pair(b"SELECT * FROM m, k ORDER BY 2"), (499, 1));
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
    // Two terms running in two directions.
    assert_eq!(pair(b"SELECT * FROM m ORDER BY p, q DESC"), (4, 1));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY r+1"), (4, 1));
    assert_eq!(pair(b"SELECT * FROM m ORDER BY q COLLATE binary"), (4, 1));

    // A side already held to a key answers fewer rows than a walk of
    // the index whole.
    assert_eq!(pair(b"SELECT * FROM m WHERE q='A' ORDER BY p"), (0, 1));
    // A table that keeps its rows in the key's own tree, a statement
    // inside the `FROM`, a join, a group, and a window.
    assert_eq!(pair(b"SELECT * FROM u ORDER BY a"), (1, 1));
    assert_eq!(pair(b"SELECT * FROM (SELECT * FROM m) ORDER BY p"), (4, 1));
    assert_eq!(pair(b"SELECT m.p FROM m, k ORDER BY m.p"), (499, 1));
    assert_eq!(pair(b"SELECT p FROM m GROUP BY p, q ORDER BY q, p"), (4, 1));
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
fn the_order_the_groups_come_in_answers_an_order_by_that_names_them() {
    // The groups are ordered by the `GROUP BY` terms, so an `ORDER BY`
    // that names those terms in that order sorts nothing of its own.
    assert_eq!(pair(b"SELECT p FROM m GROUP BY p ORDER BY p"), (4, 0));
    assert_eq!(pair(b"SELECT p FROM m GROUP BY p, q ORDER BY p, q"), (4, 0));
    // A term written backwards, one with its nulls moved, one over an
    // expression, and a count of terms the `GROUP BY` does not name.
    assert_eq!(pair(b"SELECT p FROM m GROUP BY p ORDER BY p DESC"), (4, 1));
    assert_eq!(
        pair(b"SELECT p FROM m GROUP BY p ORDER BY p NULLS LAST"),
        (4, 1)
    );
    assert_eq!(pair(b"SELECT p FROM m GROUP BY p ORDER BY p+1"), (4, 1));
    assert_eq!(pair(b"SELECT p FROM m GROUP BY p, q ORDER BY p"), (4, 1));
    // A statement that groups nothing answers no order of groups.
    assert_eq!(pair(b"SELECT count(*) FROM m ORDER BY 1"), (4, 1));
}

#[test]
fn the_order_by_of_a_compound_statement_counts_one_sort() {
    assert_eq!(
        pair(b"SELECT p FROM m UNION ALL SELECT a FROM k ORDER BY 1"),
        (103, 1)
    );
}

/// What `sql` counted as searches.
fn searched(sql: &[u8]) -> i64 {
    counted(sql).searched
}

#[test]
fn what_a_walk_counts_as_a_search() {
    // A walk of a whole table counts one per row after the first.
    assert_eq!(searched(b"SELECT * FROM m"), 4);
    // A walk held to one rowid descends to it and counts nothing.
    assert_eq!(searched(b"SELECT * FROM m WHERE rowid=2"), 0);
}

/// What `sql` answers, as `value|` per value, and what its walks
/// counted as searches.
fn smallest(sql: &[u8]) -> (String, i64) {
    let answer = Database::open(super::INDEXED).unwrap().query(sql).unwrap();
    let mut shown = String::new();
    for value in answer.rows.iter().flatten() {
        shown.push_str(&String::from_utf8_lossy(
            &value.text().unwrap_or(b"NULL".to_vec()),
        ));
        shown.push('|');
    }
    (shown, answer.stepped.searched)
}

/// A statement of no `GROUP BY` and no `HAVING` whose one aggregate is a
/// `min` or a `max` of one value reads the rows in the order of that
/// value and stops at the first row that holds one. Every answer is the
/// one the C library's shell writes.
#[test]
fn what_a_bare_min_or_max_reads_of_the_walk() {
    // `mq` holds the values of `q` forwards, so a `min` reads its first
    // entry and a `max` its last; `m` holds one row whose `q` is null,
    // which the `min` steps over and the `max` never reaches.
    assert_eq!(smallest(b"SELECT min(q) FROM m"), ("A|".to_owned(), 1));
    assert_eq!(smallest(b"SELECT max(q) FROM m"), ("c|".to_owned(), 0));
    // `mr` holds the values of `r` backwards, so the walks swap.
    assert_eq!(smallest(b"SELECT min(r) FROM m"), ("10|".to_owned(), 1));
    assert_eq!(smallest(b"SELECT max(r) FROM m"), ("50|".to_owned(), 0));
    // The table's own tree holds the rows from the smallest rowid up,
    // which answers a `min` of the rowid, and the walk of it from the
    // last row back answers a `max` of the rowid.
    assert_eq!(smallest(b"SELECT min(rowid) FROM m"), ("1|".to_owned(), 0));
    assert_eq!(smallest(b"SELECT max(rowid) FROM m"), ("5|".to_owned(), 0));
    // The value stands where the aggregate is read for something else.
    assert_eq!(smallest(b"SELECT min(q)+0 FROM m"), ("0|".to_owned(), 1));
    assert_eq!(
        smallest(b"SELECT min(DISTINCT q) FROM m"),
        ("A|".to_owned(), 1)
    );
    // Two aggregates, a `GROUP BY`, a `HAVING`, an aggregate that is
    // neither, and an index over an expression: each reads every row.
    assert_eq!(
        smallest(b"SELECT min(p), max(p) FROM m"),
        ("1|3|".to_owned(), 4)
    );
    assert_eq!(
        smallest(b"SELECT min(q) FROM m GROUP BY p"),
        ("A|A|NULL|".to_owned(), 4)
    );
    assert_eq!(
        smallest(b"SELECT min(q) FROM m HAVING min(q)>'A'"),
        (String::new(), 4)
    );
    assert_eq!(smallest(b"SELECT count(*) FROM m"), ("5|".to_owned(), 4));
    assert_eq!(smallest(b"SELECT max(a) FROM e"), ("3|".to_owned(), 2));
    // A `WHERE` that holds the walk to a key leaves the value in no
    // order, so every row it keeps is read.
    assert_eq!(
        smallest(b"SELECT min(p) FROM m WHERE q='A'"),
        ("1|".to_owned(), 5)
    );
}

/// The walk answers the value of a bare `min` or `max` for one table of
/// the schema alone, and reads every row otherwise. Every answer is the
/// one the C library's shell writes.
#[test]
fn what_a_bare_min_or_max_answers_where_the_walk_holds_no_order() {
    // A join answers the product of its sides and a statement of a `FROM`
    // answers the rows it was read for, neither in the order of a value.
    assert_eq!(smallest(b"SELECT min(p) FROM m, k"), ("1|".to_owned(), 499));
    assert_eq!(
        smallest(b"SELECT min(x) FROM (SELECT p AS x FROM m)"),
        ("1|".to_owned(), 4)
    );
    // A table that keeps its rows in the key's own tree ends the entries
    // of its indexes with that key and not with a rowid.
    assert_eq!(smallest(b"SELECT min(b) FROM u"), ("1|".to_owned(), 1));
    // An expression is no column of an index of the table.
    assert_eq!(smallest(b"SELECT min(p+1) FROM m"), ("2|".to_owned(), 4));
}
