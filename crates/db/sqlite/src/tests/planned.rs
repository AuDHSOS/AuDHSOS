// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The tree of lines an `EXPLAIN QUERY PLAN` answers.
//!
//! Every line names the line it hangs under, and the harness of the C
//! library's own suite draws the two as a tree, which is what these
//! tests compare against.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use alloc::string::String;
use alloc::vec::Vec;

use crate::db::Database;
use crate::value::Value;

/// The plan of `sql` drawn as `query_plan_graph` of the suite's
/// `tester.tcl` draws it.
fn plan(bytes: &[u8], sql: &[u8]) -> String {
    let mut held = Vec::from(b"EXPLAIN QUERY PLAN ".as_slice());
    held.extend_from_slice(sql);
    let rows = Database::open(bytes).unwrap().query(&held).unwrap().rows;
    let mut out = String::new();
    drawn(&rows, 0, "", &mut out);
    out
}

/// Writes the lines that hang under `parent`, each behind the marks the
/// tree draws in front of it.
fn drawn(rows: &[Vec<Value>], parent: i64, prefix: &str, out: &mut String) {
    let under: Vec<&Vec<Value>> = rows
        .iter()
        .filter(|row| row[1] == Value::Int(parent))
        .collect();
    for (at, row) in under.iter().enumerate() {
        let last = at + 1 == under.len();
        let (branch, below) = if last { ("`--", "   ") } else { ("|--", "|  ") };
        let Value::Text(detail) = &row[3] else {
            continue;
        };
        out.push_str(prefix);
        out.push_str(branch);
        out.push_str(&String::from_utf8_lossy(detail));
        out.push('\n');
        let Value::Int(id) = row[0] else { continue };
        drawn(rows, id, &alloc::format!("{prefix}{below}"), out);
    }
}

#[test]
fn what_a_statement_that_names_no_table_reads() {
    // `sqlite3WhereBegin` names the one row a statement of no `FROM`
    // answers, and a `VALUES` of one row is such a statement.
    assert_eq!(plan(super::INDEXED, b"SELECT 1"), "`--SCAN CONSTANT ROW\n");
    assert_eq!(plan(super::INDEXED, b"VALUES(1)"), "`--SCAN CONSTANT ROW\n");
}

#[test]
fn what_a_values_of_several_rows_is_named() {
    // The plan names the clause and not the alias, which is `%!S` of
    // `research/sqlite/src/printf.c:1024`, and the clause stands for no
    // plan of its own where a `FROM` reads it.
    assert_eq!(
        plan(super::INDEXED, b"VALUES(1),(2)"),
        "`--SCAN 2-ROW VALUES CLAUSE\n"
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM (VALUES(1),(2),(3)) AS z"),
        "`--SCAN 3-ROW VALUES CLAUSE\n"
    );
}

#[test]
fn what_the_cores_of_a_union_all_hang_under() {
    // `multiSelect` writes `COMPOUND QUERY` once, however many cores the
    // chain holds, with the core on the left under `LEFT-MOST SUBQUERY`
    // and every core after it under the operator that joins it.
    assert_eq!(
        plan(super::INDEXED, b"SELECT p FROM m UNION ALL SELECT a FROM k"),
        "`--COMPOUND QUERY\n   \
         |--LEFT-MOST SUBQUERY\n   \
         |  `--SCAN m\n   \
         `--UNION ALL\n      \
         `--SCAN k\n"
    );
    assert_eq!(
        plan(
            super::INDEXED,
            b"SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3"
        ),
        "`--COMPOUND QUERY\n   \
         |--LEFT-MOST SUBQUERY\n   \
         |  `--SCAN CONSTANT ROW\n   \
         |--UNION ALL\n   \
         |  `--SCAN CONSTANT ROW\n   \
         `--UNION ALL\n      \
         `--SCAN CONSTANT ROW\n"
    );
}

#[test]
fn what_a_statement_written_inside_a_from_is_named() {
    // The alias names it, and its own place where it carries none.
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM (SELECT p FROM m) AS z"),
        "|--SCAN m USING COVERING INDEX mpq\n`--SCAN z\n"
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM (SELECT p FROM m)"),
        "|--SCAN m USING COVERING INDEX mpq\n`--SCAN (subquery-0)\n"
    );
}

#[test]
fn what_a_values_of_several_rows_answers_where_a_from_reads_it() {
    // The rows are the ones the clause holds, whether or not a plan is
    // being named.
    let rows = Database::open(super::INDEXED)
        .unwrap()
        .query(b"SELECT * FROM (VALUES(1),(2)) AS z")
        .unwrap()
        .rows;
    assert_eq!(
        rows,
        alloc::vec![alloc::vec![Value::Int(1)], alloc::vec![Value::Int(2)]]
    );
}

#[test]
fn what_the_cores_of_a_compound_that_merges_hang_under() {
    // A compound an `ORDER BY` reaches, and one any operator but
    // `UNION ALL` joins, is answered by a merge of its cores, whose lines
    // item 273 of document 16 records as missing: the cores hang where
    // the statement hangs.
    assert_eq!(
        plan(
            super::INDEXED,
            b"SELECT p FROM m UNION ALL SELECT a FROM k ORDER BY 1"
        ),
        "|--SCAN m USING INDEX mpq\n`--SCAN k\n"
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT p FROM m UNION SELECT a FROM k"),
        "|--SCAN m\n`--SCAN k\n"
    );
}

#[test]
fn what_the_sorter_takes_where_the_walk_answers_part_of_an_order() {
    // The walk that answers the longest run of terms from the first is
    // taken, and the line names how many terms are left to sort by.
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM m ORDER BY q, +p"),
        "|--SCAN m USING INDEX mq\n`--USE TEMP B-TREE FOR LAST TERM OF ORDER BY\n"
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM m ORDER BY q, p, r"),
        "|--SCAN m USING INDEX mq\n`--USE TEMP B-TREE FOR LAST 2 TERMS OF ORDER BY\n"
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM m ORDER BY r, q"),
        "|--SCAN m USING INDEX mr\n`--USE TEMP B-TREE FOR LAST TERM OF ORDER BY\n"
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM m ORDER BY q DESC, p"),
        "|--SCAN m USING INDEX mq\n`--USE TEMP B-TREE FOR LAST TERM OF ORDER BY\n"
    );
    // A walk that answers every term leaves no sorter, and one that
    // answers none leaves every term.
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM m ORDER BY q"),
        "`--SCAN m USING INDEX mq\n"
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM m ORDER BY +q, p"),
        "|--SCAN m\n`--USE TEMP B-TREE FOR ORDER BY\n"
    );
}
