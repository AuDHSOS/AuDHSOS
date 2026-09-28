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
    // A statement the plan writes into the one above it is named no line
    // of its own, whether or not it carries an alias.
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM (SELECT p FROM m) AS z"),
        tree(&["`--SCAN m USING COVERING INDEX mpq"])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM (SELECT p FROM m)"),
        tree(&["`--SCAN m USING COVERING INDEX mpq"])
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

#[test]
fn what_a_walk_held_to_a_bare_min_or_max_is_named() {
    // `sqlite3WhereExplainOneScan` writes `SEARCH` for a walk held to the
    // one row a bare `min` or `max` reads, as it does for a key or a
    // bound, and `SCAN` where two aggregates read every row.
    assert_eq!(
        plan(super::INDEXED, b"SELECT max(q) FROM m"),
        tree(&["`--SEARCH m USING COVERING INDEX mq"])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT min(r) FROM m"),
        tree(&["`--SEARCH m USING COVERING INDEX mr"])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT max(rowid) FROM m"),
        tree(&["`--SEARCH m"])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT min(rowid) FROM m"),
        tree(&["`--SEARCH m"])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT min(q), max(q) FROM m"),
        tree(&["`--SCAN m USING COVERING INDEX mq"])
    );
}

#[test]
fn how_a_statement_written_inside_a_from_is_read() {
    // A statement the plan does not write into the one above it is read a
    // row at a time, which `CO-ROUTINE` names, and kept in a table of its
    // own where a statement stands in front of it, which `MATERIALIZE`
    // names. An aggregate statement is never written into the one above
    // it.
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM (SELECT 1), k"),
        tree(&[
            "|--CO-ROUTINE (subquery-0)",
            "|  `--SCAN CONSTANT ROW",
            "|--SCAN (subquery-0)",
            "`--SCAN k",
        ])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM (SELECT count(*) FROM m)"),
        tree(&[
            "|--CO-ROUTINE (subquery-0)",
            // The C library reads the rows out of `mr`, which holds as
            // few columns as `mq`: item 269 of document 16 records the
            // costing this tie is broken by.
            "|  `--SCAN m USING COVERING INDEX mq",
            "`--SCAN (subquery-0)",
        ])
    );
    assert_eq!(
        plan(
            super::INDEXED,
            b"SELECT * FROM (SELECT count(*) FROM m), (SELECT count(*) FROM k)"
        ),
        tree(&[
            "|--CO-ROUTINE (subquery-0)",
            "|  `--SCAN m USING COVERING INDEX mq",
            "|--MATERIALIZE (subquery-1)",
            "|  `--SCAN k USING COVERING INDEX kb",
            "|--SCAN (subquery-0)",
            "`--SCAN (subquery-1)",
        ])
    );
}

#[test]
fn where_the_lines_of_a_statement_written_into_the_one_above_it_stand() {
    // They stand in the place that statement stands in the `FROM`, and a
    // `LIMIT` on it stops nothing where the statement above it reads one
    // side and writes no `WHERE`.
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM k, (SELECT p FROM m)"),
        tree(&["|--SCAN k", "`--SCAN m USING COVERING INDEX mpq"])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM (SELECT p FROM m LIMIT 3)"),
        tree(&["`--SCAN m USING COVERING INDEX mpq"])
    );
}

#[test]
fn what_stops_a_statement_of_a_from_being_written_into_the_one_above_it() {
    // One statement per condition `flattenSubquery` reads: the plan names
    // the statement `CO-ROUTINE` or `MATERIALIZE` where it is left as it
    // stands, and the C library leaves each of these as it stands too.
    for sql in [
        // (7) it reads no table of its own.
        b"SELECT * FROM (SELECT 1)".as_slice(),
        // (4) it keeps the rows that differ.
        b"SELECT * FROM (SELECT DISTINCT p FROM m)",
        // (17) it is a compound.
        b"SELECT * FROM (SELECT p FROM m UNION ALL SELECT a FROM k)",
        // It gathers groups or reads an aggregate over them.
        b"SELECT * FROM (SELECT count(*) FROM m)",
        b"SELECT * FROM (SELECT p FROM m GROUP BY p)",
        // (25) either statement calls a window function.
        b"SELECT * FROM (SELECT row_number() OVER () FROM m)",
        b"SELECT row_number() OVER () FROM (SELECT p FROM m)",
        // (14)(13)(19)(21)(8)(9)(15): what a `LIMIT` on it stops.
        b"SELECT * FROM (SELECT p FROM m LIMIT 3 OFFSET 1)",
        b"SELECT * FROM (SELECT p FROM m LIMIT 3) LIMIT 2",
        b"SELECT * FROM (SELECT p FROM m LIMIT 3) WHERE p>0",
        b"SELECT DISTINCT * FROM (SELECT p FROM m LIMIT 3)",
        b"SELECT * FROM (SELECT p FROM m LIMIT 3), k",
        b"SELECT count(*) FROM (SELECT p FROM m LIMIT 3)",
        b"SELECT * FROM (SELECT p FROM m LIMIT 3) UNION SELECT a FROM k",
        // (11)(16): what an `ORDER BY` on it stops.
        b"SELECT * FROM (SELECT p FROM m ORDER BY q) ORDER BY 1",
        b"SELECT count(*) FROM (SELECT p FROM m ORDER BY q)",
        // (3a)(3d)(26): it stands on the far side of an outer join.
        b"SELECT * FROM k LEFT JOIN (SELECT p FROM m, f) ON 1",
        b"SELECT DISTINCT * FROM k LEFT JOIN (SELECT p FROM m) ON 1",
        b"SELECT * FROM k RIGHT JOIN (SELECT p FROM m) ON 1",
    ] {
        let held = plan(super::INDEXED, sql);
        assert!(
            held.contains("CO-ROUTINE") || held.contains("MATERIALIZE"),
            "{}: {held}",
            String::from_utf8_lossy(sql)
        );
    }
    // A `WINDOW` clause no call names is no window function, so the
    // statement is written into the one above it.
    assert_eq!(
        plan(
            super::INDEXED,
            b"SELECT * FROM (SELECT p FROM m WINDOW w AS ())"
        ),
        tree(&["`--SCAN m USING COVERING INDEX mpq"])
    );
    // An alias names the statement where the plan names how its rows are
    // read.
    assert_eq!(
        plan(
            super::INDEXED,
            b"SELECT * FROM (SELECT count(*) FROM m) AS z"
        ),
        tree(&[
            "|--CO-ROUTINE z",
            "|  `--SCAN m USING COVERING INDEX mq",
            "`--SCAN z",
        ])
    );
}

#[test]
fn what_the_lines_under_a_statement_written_into_the_one_above_it_hang_under() {
    // A line that hung under another line of that statement hangs under it
    // still, which the branches of an `OR` hang under.
    assert_eq!(
        plan(
            super::INDEXED,
            b"SELECT * FROM (SELECT * FROM m WHERE q='b' OR p=2)"
        ),
        tree(&[
            "`--MULTI-INDEX OR",
            "   |--INDEX 1",
            "   |  `--SEARCH m USING INDEX mq (q=?)",
            "   `--INDEX 2",
            "      `--SEARCH m USING INDEX mpq (p=?)",
        ])
    );
    // A `HAVING` with no `GROUP BY` in front of it aggregates, so the
    // statement is left as it stands.
    assert_eq!(
        plan(
            super::INDEXED,
            b"SELECT * FROM (SELECT count(*) FROM m HAVING count(*)>0)"
        ),
        tree(&[
            "|--CO-ROUTINE (subquery-0)",
            "|  `--SCAN m USING COVERING INDEX mq",
            "`--SCAN (subquery-0)",
        ])
    );
}

#[test]
fn what_the_terms_hold_the_walk_of_a_table_that_keeps_its_rows_in_the_key_to() {
    // The tree is the index, which `IsPrimaryKeyIndex` reads, so the plan
    // names `PRIMARY KEY` and the terms hold the walk of the table itself.
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM u WHERE a='b'"),
        tree(&["`--SEARCH u USING PRIMARY KEY (a=?)"])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM u WHERE a>'a' AND a<'z'"),
        tree(&["`--SEARCH u USING PRIMARY KEY (a>? AND a<?)"])
    );
    // An index over such a table ends its entries with that key and not
    // with a rowid, so the walk of one is not read, and a walk the terms
    // hold to nothing reads every row.
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM u WHERE b=1"),
        tree(&["`--SCAN u"])
    );
    assert_eq!(
        plan(super::INDEXED, b"SELECT * FROM u"),
        tree(&["`--SCAN u"])
    );
}

#[test]
fn what_a_walk_of_the_key_s_own_tree_answers() {
    // The entry is the row, so the walk answers it without descending the
    // table, and every column of it stands where the table holds it.
    let database = Database::open(super::INDEXED).unwrap();
    assert_eq!(
        database.query(b"SELECT * FROM u WHERE a='x'").unwrap().rows,
        alloc::vec![alloc::vec![Value::Text(b"x".to_vec()), Value::Int(1)]]
    );
    assert_eq!(
        database
            .query(b"SELECT b, a FROM u WHERE a>'x'")
            .unwrap()
            .rows,
        alloc::vec![alloc::vec![Value::Int(2), Value::Text(b"y".to_vec())]]
    );
    assert!(
        database
            .query(b"SELECT * FROM u WHERE a='q'")
            .unwrap()
            .rows
            .is_empty()
    );
}

/// A key of two columns is held by one term per column, and the row the
/// walk answers holds the columns the table declares in that order,
/// where the entry holds the key's columns first.
#[test]
fn what_the_terms_hold_a_key_of_two_columns_to() {
    let mut writer = crate::change::Writer::new(1024, 0, crate::header::Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a,b,c,PRIMARY KEY(b,c)) WITHOUT ROWID")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES(1,2,3)").unwrap();
    let image = writer.written();
    assert_eq!(
        plan(&image, b"SELECT * FROM t WHERE b=2 AND c=3"),
        tree(&["`--SEARCH t USING PRIMARY KEY (b=? AND c=?)"])
    );
    assert_eq!(
        Database::open(&image)
            .unwrap()
            .query(b"SELECT * FROM t WHERE b=2 AND c=3")
            .unwrap()
            .rows,
        alloc::vec![alloc::vec![Value::Int(1), Value::Int(2), Value::Int(3)]]
    );
}

/// Two branches of an `OR` that name the same row of a table that keeps
/// its rows in the key's own tree answer it once, the walk carrying no
/// rowid to tell the rows apart by.
#[test]
fn what_two_branches_that_name_one_row_of_the_key_s_own_tree_answer() {
    let database = Database::open(super::INDEXED).unwrap();
    assert_eq!(
        database
            .query(b"SELECT * FROM u WHERE a='x' OR a='x'")
            .unwrap()
            .rows,
        alloc::vec![alloc::vec![Value::Text(b"x".to_vec()), Value::Int(1)]]
    );
    assert_eq!(
        database
            .query(b"SELECT * FROM u WHERE a='x' OR a='y'")
            .unwrap()
            .rows,
        alloc::vec![
            alloc::vec![Value::Text(b"x".to_vec()), Value::Int(1)],
            alloc::vec![Value::Text(b"y".to_vec()), Value::Int(2)]
        ]
    );
}

/// A walk that answers one row answers every term of an `ORDER BY` and
/// leaves the rows of a `DISTINCT` where they stand, and one that answers
/// more than one builds a tree for each.
#[test]
fn what_a_walk_that_answers_one_row_leaves_unbuilt() {
    for (sql, lines) in [
        // A term that names one rowid, and one whose value is no whole
        // number, each answer one row.
        (
            b"SELECT * FROM k WHERE rowid=1 ORDER BY b".as_slice(),
            tree(&["`--SEARCH k USING INTEGER PRIMARY KEY (rowid=?)"]),
        ),
        (
            b"SELECT * FROM k WHERE rowid='x' ORDER BY b",
            tree(&["`--SEARCH k USING INTEGER PRIMARY KEY (rowid=?)"]),
        ),
        // A key that holds every column of a unique index answers one
        // row, and the rows of a `DISTINCT` are that one row.
        (
            b"SELECT * FROM k WHERE b='x' ORDER BY a",
            tree(&["`--SEARCH k USING INDEX kb (b=?)"]),
        ),
        (
            b"SELECT DISTINCT * FROM k WHERE rowid=1",
            tree(&["`--SEARCH k USING INTEGER PRIMARY KEY (rowid=?)"]),
        ),
        (
            b"SELECT DISTINCT a FROM k WHERE b='x'",
            tree(&["`--SEARCH k USING INDEX kb (b=?)"]),
        ),
        // The key of a table that keeps its rows in the key's own tree is
        // unique, so a key that holds every column of it answers one row.
        (
            b"SELECT * FROM u WHERE a='x' ORDER BY b",
            tree(&["`--SEARCH u USING PRIMARY KEY (a=?)"]),
        ),
        // An index that is no unique one holds as many entries per value
        // as the table has rows of it.
        (
            b"SELECT * FROM k WHERE a=1 ORDER BY b",
            tree(&[
                "|--SEARCH k USING INDEX ka (a=?)",
                "`--USE TEMP B-TREE FOR ORDER BY",
            ]),
        ),
        // A key of fewer columns than the index holds reaches every entry
        // that begins with it.
        (
            b"SELECT * FROM m WHERE p=1 AND q='a' ORDER BY r",
            tree(&[
                "|--SEARCH m USING INDEX mpq (p=? AND q=?)",
                "`--USE TEMP B-TREE FOR ORDER BY",
            ]),
        ),
        // Two rows that hold no value at a column of a unique index stand
        // under the same key, so a key that names a null value reaches
        // both.
        (
            b"SELECT * FROM k WHERE b IS NULL ORDER BY a",
            tree(&[
                "|--SEARCH k USING INDEX kb (b=?)",
                "`--USE TEMP B-TREE FOR ORDER BY",
            ]),
        ),
        // A range of rowids whose two bounds name one rowid is no term
        // written `=`.
        (
            b"SELECT * FROM k WHERE rowid>=1 AND rowid<=1 ORDER BY b",
            tree(&[
                "|--SEARCH k USING INTEGER PRIMARY KEY (rowid=?)",
                "`--USE TEMP B-TREE FOR ORDER BY",
            ]),
        ),
    ] {
        assert_eq!(plan(super::INDEXED, sql), lines, "{sql:?}");
    }
}

/// What the rows a `DISTINCT` keeps one of ask of the walk: a column of
/// the list stands where an index holds it, whatever place the list writes
/// it in, and a list no two rows of the table stand under the same values
/// of leaves the `DISTINCT` nothing to keep one of.
#[test]
fn what_the_rows_a_distinct_keeps_one_of_ask_of_the_walk() {
    for (sql, lines) in [
        // A column a term holds at one value stands alike in every row the
        // walk answers, and the columns of the list stand after it in the
        // index.
        (
            b"SELECT DISTINCT q FROM m WHERE p=1".as_slice(),
            tree(&["`--SEARCH m USING COVERING INDEX mpq (p=?)"]),
        ),
        (
            b"SELECT DISTINCT p FROM m WHERE p=1",
            tree(&["`--SEARCH m USING COVERING INDEX mpq (p=?)"]),
        ),
        // The list writes its columns in the other order than the index
        // holds them, which groups the rows all the same.
        (
            b"SELECT DISTINCT q, p FROM m",
            tree(&["`--SCAN m USING COVERING INDEX mpq"]),
        ),
        // A side read by the whole table is read out of an index that
        // groups the columns, whichever way that index holds them.
        (
            b"SELECT DISTINCT q FROM m",
            tree(&["`--SCAN m USING COVERING INDEX mq"]),
        ),
        (
            b"SELECT DISTINCT r FROM m",
            tree(&["`--SCAN m USING COVERING INDEX mr"]),
        ),
        // The rowid stands once in the table, so a list that names it
        // names no two rows alike.
        (
            b"SELECT DISTINCT p, rowid FROM m",
            tree(&["`--SCAN m USING COVERING INDEX mpq"]),
        ),
        // The key of a table that keeps its rows in the key's own tree is
        // unique and holds a value of every row.
        (b"SELECT DISTINCT a FROM u", tree(&["`--SCAN u"])),
        // A column the list compares under another collation than the
        // index holds it under stands in another order there, whether the
        // index groups the columns or makes the list name no two rows
        // alike, and a column a term holds at one value stands alike in
        // every row under the collation the term compares under alone.
        (
            b"SELECT DISTINCT q COLLATE binary FROM m",
            tree(&[
                "|--SCAN m USING COVERING INDEX mq",
                "`--USE TEMP B-TREE FOR DISTINCT",
            ]),
        ),
        (
            b"SELECT DISTINCT b COLLATE NOCASE FROM k",
            tree(&[
                "|--SCAN k USING COVERING INDEX kb",
                "`--USE TEMP B-TREE FOR DISTINCT",
            ]),
        ),
        (
            b"SELECT DISTINCT p, q COLLATE binary FROM m WHERE q='a'",
            tree(&[
                "|--SEARCH m USING INDEX mq (q=?)",
                "`--USE TEMP B-TREE FOR DISTINCT",
            ]),
        ),
        // No index of the table holds every column of the list.
        (
            b"SELECT DISTINCT p, q, r FROM m",
            tree(&["|--SCAN m", "`--USE TEMP B-TREE FOR DISTINCT"]),
        ),
        (
            b"SELECT DISTINCT b, a FROM k",
            tree(&["|--SCAN k", "`--USE TEMP B-TREE FOR DISTINCT"]),
        ),
        // The list names the one column of an index that is no unique one,
        // and leaves out the column of the unique index over the table.
        (
            b"SELECT DISTINCT a FROM k",
            tree(&["`--SCAN k USING COVERING INDEX ka"]),
        ),
        // A `*` and an expression each name no column of the side.
        (
            b"SELECT DISTINCT * FROM m",
            tree(&["|--SCAN m", "`--USE TEMP B-TREE FOR DISTINCT"]),
        ),
        (
            b"SELECT DISTINCT p+1 FROM m",
            tree(&[
                "|--SCAN m USING COVERING INDEX mpq",
                "`--USE TEMP B-TREE FOR DISTINCT",
            ]),
        ),
    ] {
        assert_eq!(plan(super::INDEXED, sql), lines, "{sql:?}");
    }
}

/// The rows a walk chosen by the list of a `DISTINCT` answers, which stand
/// in the order of the index it reads.
#[test]
fn what_a_walk_chosen_by_the_list_of_a_distinct_answers() {
    let database = Database::open(super::INDEXED).unwrap();
    // The index over `r` holds its entries from the largest value down and
    // the rows that hold no value there last.
    assert_eq!(
        database.query(b"SELECT DISTINCT r FROM m").unwrap().rows,
        alloc::vec![
            alloc::vec![Value::Int(50)],
            alloc::vec![Value::Int(30)],
            alloc::vec![Value::Int(20)],
            alloc::vec![Value::Int(10)],
            alloc::vec![Value::Null]
        ]
    );
    assert_eq!(
        database
            .query(b"SELECT DISTINCT q FROM m WHERE p=1")
            .unwrap()
            .rows,
        alloc::vec![
            alloc::vec![Value::Text(b"A".to_vec())],
            alloc::vec![Value::Text(b"b".to_vec())]
        ]
    );
}

/// What a `DISTINCT` over more than the one side, over the rows a
/// statement answered, and over a walk held to one rowid asks.
#[test]
fn what_a_distinct_over_no_walk_of_one_index_asks() {
    for (sql, lines) in [
        // A walk held to one rowid answers one row, whatever the list
        // names.
        (
            b"SELECT DISTINCT q FROM m WHERE rowid='x'".as_slice(),
            tree(&["`--SEARCH m USING INTEGER PRIMARY KEY (rowid=?)"]),
        ),
        // A partial index holds an entry for some rows of the table, so it
        // groups no column over them.
        (
            b"SELECT DISTINCT b FROM e",
            tree(&["|--SCAN e", "`--USE TEMP B-TREE FOR DISTINCT"]),
        ),
        (
            b"SELECT DISTINCT a FROM e",
            tree(&[
                "|--SCAN e USING COVERING INDEX ec",
                "`--USE TEMP B-TREE FOR DISTINCT",
            ]),
        ),
        // Two sides read one after another answer the rows of the second
        // per row of the first, which the C library reads as grouping the
        // columns of the first and this engine leaves to a tree of its
        // own. Item 279 of document 16 records it.
        (
            b"SELECT DISTINCT p FROM m, k",
            tree(&[
                "|--SCAN m USING COVERING INDEX mpq",
                "|--SCAN k USING COVERING INDEX kb",
                "`--USE TEMP B-TREE FOR DISTINCT",
            ]),
        ),
        // A side that reads what a statement answered holds no index.
        (
            b"SELECT DISTINCT p FROM (SELECT p FROM m LIMIT 2)",
            tree(&[
                "|--CO-ROUTINE (subquery-0)",
                "|  `--SCAN m USING COVERING INDEX mpq",
                "|--SCAN (subquery-0)",
                "`--USE TEMP B-TREE FOR DISTINCT",
            ]),
        ),
    ] {
        assert_eq!(plan(super::INDEXED, sql), lines, "{sql:?}");
    }
}

/// A unique index over an expression, and one that holds its column under
/// another collation, leave a `DISTINCT` its tree: no column of the first
/// carries a value of the row, and the entries of the second stand in
/// another order than the list compares under.
#[test]
fn what_a_unique_index_the_list_does_not_reach_leaves_a_distinct() {
    let mut writer = crate::change::Writer::new(1024, 0, crate::header::Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE g(a NOT NULL, b NOT NULL)")
        .unwrap();
    writer.run(b"CREATE UNIQUE INDEX gx ON g(abs(a))").unwrap();
    writer
        .run(b"CREATE UNIQUE INDEX gy ON g(b COLLATE NOCASE)")
        .unwrap();
    // A partial index holds an entry for some rows of the table, so two
    // rows may stand under one key of it.
    writer
        .run(b"CREATE UNIQUE INDEX gz ON g(a) WHERE a>0")
        .unwrap();
    writer.run(b"INSERT INTO g VALUES(1,'x')").unwrap();
    let image = writer.written();
    for sql in [
        b"SELECT DISTINCT a, b FROM g".as_slice(),
        // The list leaves out the column of the one unique index the
        // reading reaches.
        b"SELECT DISTINCT a FROM g",
    ] {
        assert_eq!(
            plan(&image, sql),
            tree(&["|--SCAN g", "`--USE TEMP B-TREE FOR DISTINCT"]),
            "{sql:?}"
        );
    }
}
/// A join whose `ON` names a column of a table that keeps its rows in the
/// key's own tree reads no index of that table, whose entries end with the
/// key and not with a rowid, and answers the rows of the join all the same.
///
/// Item 278 of document 16 records the walk of such an index as missing;
/// the C library reads `t2c` here.
#[test]
fn what_a_join_over_a_table_that_keeps_its_rows_in_the_key_answers() {
    let mut writer = crate::change::Writer::new(1024, 0, crate::header::Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(a INT, b INT)".as_slice(),
        b"INSERT INTO t1 VALUES(1,2)",
        b"INSERT INTO t1 VALUES(1,3)",
        b"INSERT INTO t1 VALUES(1,4)",
        b"CREATE TABLE t2(c INT, d INT PRIMARY KEY) WITHOUT ROWID",
        b"INSERT INTO t2 VALUES(3,33)",
        b"INSERT INTO t2 VALUES(4,44)",
        b"INSERT INTO t2 VALUES(5,55)",
        b"CREATE INDEX t2c ON t2(c)",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    assert_eq!(
        database
            .query(b"SELECT * FROM t1 LEFT OUTER JOIN t2 ON b=c ORDER BY +b")
            .unwrap()
            .rows,
        alloc::vec![
            alloc::vec![Value::Int(1), Value::Int(2), Value::Null, Value::Null],
            alloc::vec![Value::Int(1), Value::Int(3), Value::Int(3), Value::Int(33)],
            alloc::vec![Value::Int(1), Value::Int(4), Value::Int(4), Value::Int(44)]
        ]
    );
    assert_eq!(
        plan(
            &image,
            b"SELECT * FROM t1 LEFT OUTER JOIN t2 ON b=c ORDER BY +b"
        ),
        tree(&[
            "|--SCAN t1",
            "|--SCAN t2 LEFT-JOIN",
            "`--USE TEMP B-TREE FOR ORDER BY"
        ])
    );
}

/// The line of a side an outer join keeps the unmatched rows of ends with
/// the words for that join, and an `ON` that names the key of a table
/// keeping its rows in the key's own tree holds the walk of that key.
///
/// The C library walks the side of a `RIGHT` or a `FULL` join a second
/// time for the rows nothing matched, which it names `RIGHT-JOIN` over the
/// lines of that walk. Item 280 of document 16 records it.
#[test]
fn what_the_line_of_a_side_an_outer_join_keeps_ends_with() {
    let mut writer = crate::change::Writer::new(1024, 0, crate::header::Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(a INT, b INT)".as_slice(),
        b"INSERT INTO t1 VALUES(1,2)",
        b"CREATE TABLE t2(c INT, d INT PRIMARY KEY) WITHOUT ROWID",
        b"INSERT INTO t2 VALUES(3,33)",
        b"CREATE INDEX t2c ON t2(c)",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    for (sql, lines) in [
        (
            b"SELECT * FROM t1 LEFT JOIN t2 ON b=d".as_slice(),
            tree(&[
                "|--SCAN t1",
                "`--SEARCH t2 USING PRIMARY KEY (d=?) LEFT-JOIN",
            ]),
        ),
        (
            b"SELECT * FROM t1 JOIN t2 ON b=d",
            tree(&["|--SCAN t1", "`--SEARCH t2 USING PRIMARY KEY (d=?)"]),
        ),
        // A `FULL` join keeps the unmatched rows of both sides, and a
        // `RIGHT` join those of its own side alone. Both walk that side
        // out of its own tree, the second walk reading the rows in the
        // order the first left them in.
        (
            b"SELECT * FROM t1 FULL JOIN t2 ON b=d",
            tree(&["|--SCAN t1", "`--SCAN t2 LEFT-JOIN"]),
        ),
        (
            b"SELECT * FROM t1 RIGHT JOIN t2 ON b=d",
            tree(&["|--SCAN t1", "`--SCAN t2"]),
        ),
    ] {
        assert_eq!(plan(&image, sql), lines, "{sql:?}");
    }
}

/// The plan names the rows of a view as it names the rows of a statement
/// written inside a `FROM`: the statement of the view is written into the
/// one that reads it where nothing of what the two hold stops it, and named
/// `CO-ROUTINE` or `MATERIALIZE` otherwise.
///
/// `sqlite3SelectExpand` of `research/sqlite/src/select.c` puts the
/// statement of the view in the place of the name that stands for it.
#[test]
fn how_the_rows_of_a_view_are_read() {
    let mut writer = crate::change::Writer::new(1024, 0, crate::header::Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a,b)".as_slice(),
        b"INSERT INTO t VALUES(1,2)",
        b"INSERT INTO t VALUES(3,4)",
        b"CREATE VIEW v AS SELECT a, b FROM t WHERE a>0",
        b"CREATE VIEW w AS SELECT count(*) AS n FROM t",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    for (sql, lines) in [
        // The statement of the view reads one table and keeps every row,
        // so its line stands where the name stood.
        (b"SELECT * FROM v".as_slice(), tree(&["`--SCAN t"])),
        (b"SELECT * FROM v, t", tree(&["|--SCAN t", "`--SCAN t"])),
        // An aggregate statement is read a row at a time, which the plan
        // names `CO-ROUTINE` and hangs the lines of that statement under.
        (
            b"SELECT * FROM w",
            tree(&["|--CO-ROUTINE w", "|  `--SCAN t", "`--SCAN w"]),
        ),
        // A statement of its own in front of it makes the rows of the view
        // a table of their own.
        (
            b"SELECT * FROM (SELECT 1), w",
            tree(&[
                "|--CO-ROUTINE (subquery-0)",
                "|  `--SCAN CONSTANT ROW",
                "|--MATERIALIZE w",
                "|  `--SCAN t",
                "|--SCAN (subquery-0)",
                "`--SCAN w",
            ]),
        ),
    ] {
        assert_eq!(plan(&image, sql), lines, "{sql:?}");
    }
}

/// A view written into the statement that reads it is no statement of its
/// own, so a view after it is read a row at a time, which the plan names
/// `CO-ROUTINE`.
///
/// The C library reads the rows of a co-routine before the rows of the
/// sides the `FROM` writes in front of it, which item 283 of document 16
/// records; this engine reads the sides in the order the `FROM` writes
/// them, so the two lines of the walk stand the other way round.
#[test]
fn what_a_view_written_in_leaves_the_view_after_it() {
    let mut writer = crate::change::Writer::new(1024, 0, crate::header::Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a,b)".as_slice(),
        b"INSERT INTO t VALUES(1,2)",
        b"CREATE VIEW v AS SELECT a, b FROM t WHERE a>0",
        b"CREATE VIEW w AS SELECT count(*) AS n FROM t",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    assert_eq!(
        plan(&image, b"SELECT * FROM v, w"),
        tree(&["|--CO-ROUTINE w", "|  `--SCAN t", "|--SCAN t", "`--SCAN w"])
    );
    assert_eq!(
        plan(&image, b"SELECT * FROM w, v"),
        tree(&["|--CO-ROUTINE w", "|  `--SCAN t", "|--SCAN w", "`--SCAN t"])
    );
}

/// The plan names every statement written inside an expression after the
/// lines of the walk: `SCALAR SUBQUERY` for one read for a value and
/// `LIST SUBQUERY` for one an `IN` reads, with `CORRELATED` in front where
/// the statement reads a column of the row the walk stands on.
///
/// The C library reads an `EXISTS` as a walk of the statement above it,
/// which it names `SCAN t2 EXISTS`, and writes `CREATE BLOOM FILTER` under
/// the line of a list it reads once. Item 284 of document 16 records both.
#[test]
fn what_the_plan_names_a_statement_inside_an_expression_by() {
    let mut writer = crate::change::Writer::new(1024, 0, crate::header::Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(x,y)".as_slice(),
        b"CREATE TABLE t2(x,y)",
        b"CREATE TABLE sub(a,b)",
        b"INSERT INTO t1 VALUES(1,2)",
        b"INSERT INTO t2 VALUES(1,2)",
        b"INSERT INTO sub VALUES(1,2)",
    ] {
        writer.run(sql).unwrap();
    }
    let image = writer.written();
    for (sql, lines) in [
        (
            b"SELECT (SELECT a FROM sub) FROM t1".as_slice(),
            tree(&["|--SCAN t1", "`--SCALAR SUBQUERY 0", "   `--SCAN sub"]),
        ),
        // The lines of that statement stand under its own, the sorter of
        // its `ORDER BY` among them.
        (
            b"SELECT x FROM t1 WHERE y=(SELECT b FROM sub ORDER BY a)",
            tree(&[
                "|--SCAN t1",
                "`--SCALAR SUBQUERY 0",
                "   |--SCAN sub",
                "   `--USE TEMP B-TREE FOR ORDER BY",
            ]),
        ),
        (
            b"SELECT x FROM t1 WHERE y IN (SELECT y FROM t2)",
            tree(&["|--SCAN t1", "`--LIST SUBQUERY 0", "   `--SCAN t2"]),
        ),
        // A statement that reads a column of the row the walk stands on is
        // read once per row.
        (
            b"SELECT x FROM t1 WHERE y IN (SELECT y FROM t2 WHERE t2.x=t1.x)",
            tree(&[
                "|--SCAN t1",
                "`--CORRELATED LIST SUBQUERY 0",
                "   `--SCAN t2",
            ]),
        ),
        // A column the statement of its own answers is its own, whatever
        // the statement above it answers under that name.
        (
            b"SELECT x FROM t1 WHERE y IN (SELECT y FROM t2 WHERE t2.x=1)",
            tree(&["|--SCAN t1", "`--LIST SUBQUERY 0", "   `--SCAN t2"]),
        ),
        // A schema in front of the table of the side says which database of
        // the connection holds it.
        (
            b"SELECT x FROM t1 WHERE y IN (SELECT y FROM main.t2)",
            tree(&["|--SCAN t1", "`--LIST SUBQUERY 0", "   `--SCAN main.t2"]),
        ),
        // An alias stands in the place of both.
        (
            b"SELECT x FROM t1 WHERE y IN (SELECT y FROM main.t2 AS q)",
            tree(&["|--SCAN t1", "`--LIST SUBQUERY 0", "   `--SCAN q"]),
        ),
        // A side that reads a statement of its own answers a bare name and
        // the names the alias it carries stands in front of.
        (
            b"SELECT x FROM t1 WHERE y IN (SELECT y FROM (SELECT y FROM t2))",
            tree(&["|--SCAN t1", "`--LIST SUBQUERY 1", "   `--SCAN t2"]),
        ),
        (
            b"SELECT x FROM t1 WHERE y IN (SELECT q.y FROM (SELECT y FROM t2) AS q)",
            tree(&["|--SCAN t1", "`--LIST SUBQUERY 1", "   `--SCAN t2"]),
        ),
        (
            b"SELECT x FROM t1 WHERE y IN (SELECT t1.y FROM (SELECT y FROM t2) AS q)",
            tree(&[
                "|--SCAN t1",
                "`--CORRELATED LIST SUBQUERY 1",
                "   `--SCAN t2",
            ]),
        ),
        // An alias that is the name of a side of the statement above it
        // answers the name itself.
        (
            b"SELECT x FROM t1 WHERE y IN (SELECT t1.y FROM (SELECT y FROM t2) AS t1)",
            tree(&["|--SCAN t1", "`--LIST SUBQUERY 1", "   `--SCAN t2"]),
        ),
    ] {
        assert_eq!(plan(&image, sql), lines, "{sql:?}");
    }
}
