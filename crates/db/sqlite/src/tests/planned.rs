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

/// The lines of a tree, each written on its own.
fn tree(lines: &[&str]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
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
        tree(&[
            "`--COMPOUND QUERY",
            "   |--LEFT-MOST SUBQUERY",
            "   |  `--SCAN m USING COVERING INDEX mpq",
            "   `--UNION ALL",
            "      `--SCAN k USING COVERING INDEX ka",
        ])
    );
    assert_eq!(
        plan(
            super::INDEXED,
            b"SELECT 1 UNION ALL SELECT 2 UNION ALL SELECT 3"
        ),
        tree(&[
            "`--COMPOUND QUERY",
            "   |--LEFT-MOST SUBQUERY",
            "   |  `--SCAN CONSTANT ROW",
            "   |--UNION ALL",
            "   |  `--SCAN CONSTANT ROW",
            "   `--UNION ALL",
            "      `--SCAN CONSTANT ROW",
        ])
    );
}

#[test]
fn what_a_statement_written_inside_a_from_is_named() {
    // The alias names it, and its own place where it carries none.
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM (SELECT p FROM m) AS z"),
        tree(&["|--SCAN m USING COVERING INDEX mpq", "`--SCAN z",])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM (SELECT p FROM m)"),
        tree(&["|--SCAN m USING COVERING INDEX mpq", "`--SCAN (subquery-0)",])
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
    // `UNION ALL` joins, is answered by a merge of its cores: `MERGE`
    // with the operator stands over `LEFT` and `RIGHT`, and the split
    // takes one core for the right side.
    assert_eq!(
        plan(super::INDEXED, b"SELECT 1 UNION SELECT 2"),
        tree(&[
            "`--MERGE (UNION)",
            "   |--LEFT",
            "   |  `--SCAN CONSTANT ROW",
            "   `--RIGHT",
            "      `--SCAN CONSTANT ROW",
        ])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT 1 EXCEPT SELECT 2 EXCEPT SELECT 3"),
        tree(&[
            "`--MERGE (EXCEPT)",
            "   |--LEFT",
            "   |  `--MERGE (EXCEPT)",
            "   |     |--LEFT",
            "   |     |  `--SCAN CONSTANT ROW",
            "   |     `--RIGHT",
            "   |        `--SCAN CONSTANT ROW",
            "   `--RIGHT",
            "      `--SCAN CONSTANT ROW",
        ])
    );
}

#[test]
fn where_a_run_of_union_longer_than_three_cores_is_split() {
    // `SQLITE_BalancedMerge` splits a run of `UNION` or of `UNION ALL`
    // longer than three cores near its middle, where every other
    // operator takes one core for the right side.
    assert_eq!(
        plan(
            super::INDEXED,
            b"SELECT 1 UNION SELECT 2 UNION SELECT 3 UNION SELECT 4"
        ),
        tree(&[
            "`--MERGE (UNION)",
            "   |--LEFT",
            "   |  `--MERGE (UNION)",
            "   |     |--LEFT",
            "   |     |  `--SCAN CONSTANT ROW",
            "   |     `--RIGHT",
            "   |        `--SCAN CONSTANT ROW",
            "   `--RIGHT",
            "      `--MERGE (UNION)",
            "         |--LEFT",
            "         |  `--SCAN CONSTANT ROW",
            "         `--RIGHT",
            "            `--SCAN CONSTANT ROW",
        ])
    );
}

#[test]
fn what_the_cores_of_a_chain_of_two_operators_hang_under() {
    // The levels that write an operator are the trailing run of
    // `UNION ALL`, and the levels above it merge, so the operator of that
    // run hangs beside the merge and not under it.
    assert_eq!(
        plan(
            super::INDEXED,
            b"SELECT 1 UNION ALL SELECT 2 UNION SELECT 3 UNION ALL SELECT 4"
        ),
        tree(&[
            "|--MERGE (UNION)",
            "|  |--LEFT",
            "|  |  `--MERGE (UNION ALL)",
            "|  |     |--LEFT",
            "|  |     |  `--SCAN CONSTANT ROW",
            "|  |     `--RIGHT",
            "|  |        `--SCAN CONSTANT ROW",
            "|  `--RIGHT",
            "|     `--SCAN CONSTANT ROW",
            "`--UNION ALL",
            "   `--SCAN CONSTANT ROW",
        ])
    );
}

#[test]
fn what_the_sorter_a_core_of_a_merge_builds_takes() {
    // Every core of a merge sorts its rows by the order of the merge,
    // which is every column of the answer for an operator that tells two
    // rows apart, so the walk of an index over the columns in front
    // leaves the sorter the columns after them.
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM m UNION SELECT * FROM m"),
        tree(&[
            "`--MERGE (UNION)",
            "   |--LEFT",
            "   |  |--SCAN m USING INDEX mpq",
            "   |  `--USE TEMP B-TREE FOR LAST TERM OF ORDER BY",
            "   `--RIGHT",
            "      |--SCAN m USING INDEX mpq",
            "      `--USE TEMP B-TREE FOR LAST TERM OF ORDER BY",
        ])
    );
    // A core of no `FROM` answers one row and sorts nothing; a core the
    // order of the merge names no column of sorts by every term.
    assert_eq!(
        plan(super::INDEXED, b"SELECT q FROM m UNION SELECT b FROM k"),
        tree(&[
            "`--MERGE (UNION)",
            "   |--LEFT",
            "   |  `--SCAN m USING COVERING INDEX mq",
            "   `--RIGHT",
            "      |--SCAN k USING COVERING INDEX kb",
            "      `--USE TEMP B-TREE FOR ORDER BY",
        ])
    );
}

#[test]
fn what_the_sorter_takes_where_the_walk_answers_part_of_an_order() {
    // The walk that answers the longest run of terms from the first is
    // taken, and the line names how many terms are left to sort by.
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM m ORDER BY q, +p"),
        tree(&[
            "|--SCAN m USING INDEX mq",
            "`--USE TEMP B-TREE FOR LAST TERM OF ORDER BY",
        ])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM m ORDER BY q, p, r"),
        tree(&[
            "|--SCAN m USING INDEX mq",
            "`--USE TEMP B-TREE FOR LAST 2 TERMS OF ORDER BY",
        ])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM m ORDER BY r, q"),
        tree(&[
            "|--SCAN m USING INDEX mr",
            "`--USE TEMP B-TREE FOR LAST TERM OF ORDER BY",
        ])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM m ORDER BY q DESC, p"),
        tree(&[
            "|--SCAN m USING INDEX mq",
            "`--USE TEMP B-TREE FOR LAST TERM OF ORDER BY",
        ])
    );
    // A walk that answers every term leaves no sorter, and one that
    // answers none leaves every term.
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM m ORDER BY q"),
        "`--SCAN m USING INDEX mq\n"
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM m ORDER BY +q, p"),
        tree(&["|--SCAN m", "`--USE TEMP B-TREE FOR ORDER BY",])
    );
}

#[test]
fn what_the_order_of_a_merge_reads_of_one_core() {
    // The order counts the columns of the answer, so a term reaches the
    // rowid, a column of the side, or nothing at all where the column
    // came from an expression.
    assert_eq!(
        plan(super::INDEXED, b"SELECT rowid FROM m UNION SELECT a FROM k"),
        tree(&[
            "`--MERGE (UNION)",
            "   |--LEFT",
            "   |  `--SCAN m",
            "   `--RIGHT",
            "      `--SCAN k USING COVERING INDEX ka",
        ])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT q+1 FROM m UNION SELECT b FROM k"),
        tree(&[
            "`--MERGE (UNION)",
            "   |--LEFT",
            "   |  |--SCAN m USING COVERING INDEX mq",
            "   |  `--USE TEMP B-TREE FOR ORDER BY",
            "   `--RIGHT",
            "      `--SCAN k USING COVERING INDEX kb",
        ])
    );
    // A table that keeps its rows in the key's own tree, and a term whose
    // nulls are put where the order does not put them, each leave the
    // whole order to the sorter.
    assert_eq!(
        plan(super::INDEXED, b"SELECT a FROM u UNION SELECT q FROM m"),
        tree(&[
            "`--MERGE (UNION)",
            "   |--LEFT",
            "   |  |--SCAN u",
            "   |  `--USE TEMP B-TREE FOR ORDER BY",
            "   `--RIGHT",
            "      |--SCAN m USING COVERING INDEX mq",
            "      `--USE TEMP B-TREE FOR ORDER BY",
        ])
    );
    assert_eq!(
        plan(
            super::INDEXED,
            b"SELECT p FROM m UNION SELECT a FROM k ORDER BY 1 NULLS LAST"
        ),
        tree(&[
            "`--MERGE (UNION)",
            "   |--LEFT",
            "   |  |--SCAN m USING COVERING INDEX mpq",
            "   |  `--USE TEMP B-TREE FOR ORDER BY",
            "   `--RIGHT",
            "      |--SCAN k USING COVERING INDEX ka",
            "      `--USE TEMP B-TREE FOR ORDER BY",
        ])
    );
}
