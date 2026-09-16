// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `INSERT ... ON CONFLICT`, against what the shell answers for the
//! same statements.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// A connection over a table of a key, a unique column and a value.
fn writer() -> Writer {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a INTEGER PRIMARY KEY, b UNIQUE, c)")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x',10)").unwrap();
    writer
}

/// The rows of the table, as `rowid|a|b|c` lines.
fn rows(writer: &Writer) -> alloc::string::String {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let mut shown = alloc::string::String::new();
    for (rowid, values) in database.rows_of(b"t").unwrap() {
        shown.push_str(&alloc::string::ToString::to_string(&rowid));
        for value in &values {
            shown.push('|');
            shown.push_str(&alloc::string::String::from_utf8_lossy(
                &value.text().unwrap_or(b"NULL".to_vec()),
            ));
        }
        shown.push('\n');
    }
    shown
}

#[test]
fn a_clause_that_does_nothing_passes_the_row_over() {
    let mut writer = writer();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT DO NOTHING")
            .unwrap(),
        Vec::<Vec<Value>>::new()
    );
    assert_eq!(rows(&writer), "1|1|x|10\n");
    // A row that shares no key is written.
    writer
        .run(b"INSERT INTO t VALUES(2,'y',20) ON CONFLICT DO NOTHING")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|10\n2|2|y|20\n");
}

#[test]
fn a_clause_that_writes_writes_the_row_the_conflict_found() {
    let mut writer = writer();
    // The key the table is written under is the one the clause names.
    writer
        .run(b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a) DO UPDATE SET c=excluded.c")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|20\n");
    // The unique column is the other key, and the row the conflict
    // found is the one the clause reads its columns from.
    writer
        .run(b"INSERT INTO t VALUES(2,'x',30) ON CONFLICT(b) DO UPDATE SET c=c+1")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|21\n");
    // A `WHERE` the row does not hold for passes it over.
    writer
        .run(b"INSERT INTO t VALUES(2,'x',30) ON CONFLICT(b) DO UPDATE SET c=99 WHERE c<0")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|21\n");
}

#[test]
fn the_clause_a_conflict_reaches_is_the_one_over_the_key_it_shares() {
    let mut writer = writer();
    writer.run(b"INSERT INTO t VALUES(2,'y',20)").unwrap();
    // The row shares the key of the table with one row and the unique
    // column with another, and each clause writes its own.
    writer
        .run(
            b"INSERT INTO t VALUES(1,'y',30) ON CONFLICT(a) DO UPDATE SET c=1 \
              ON CONFLICT(b) DO UPDATE SET c=2",
        )
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|1\n2|2|y|20\n");
    // A clause over another key leaves the conflict to the constraint.
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,'z',40) ON CONFLICT(b) DO NOTHING")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.a"
    );
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(9,'x',40) ON CONFLICT(a) DO NOTHING")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.b"
    );
}

#[test]
fn a_clause_that_names_the_columns_of_no_key_is_refused() {
    let mut writer = writer();
    for sql in [
        b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(c) DO NOTHING".as_slice(),
        b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a,b) DO NOTHING",
        b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a) DO UPDATE SET c=1 ON CONFLICT(c) DO NOTHING",
    ] {
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint",
            "{sql:?}"
        );
    }
    // The row is left as it was, because the statement is refused
    // before it writes.
    assert_eq!(rows(&writer), "1|1|x|10\n");
}

#[test]
fn a_column_the_row_of_the_conflict_does_not_hold_is_refused() {
    let mut writer = writer();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a) DO UPDATE SET c=excluded.zz")
            .unwrap_err()
            .message(),
        "no such column: excluded.zz"
    );
}

#[test]
fn a_clause_that_writes_holds_the_row_to_the_constraints_of_the_table() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a INTEGER PRIMARY KEY, b UNIQUE, c NOT NULL)")
        .unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x',10)").unwrap();
    writer.run(b"INSERT INTO t VALUES(2,'y',20)").unwrap();
    // The row the clause writes carries the unique column of another
    // row, which the constraint refuses.
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,'z',30) ON CONFLICT(a) DO UPDATE SET b='y'")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.b"
    );
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,'z',30) ON CONFLICT(a) DO UPDATE SET c=NULL")
            .unwrap_err()
            .message(),
        "NOT NULL constraint failed: t.c"
    );
}

#[test]
fn what_the_parser_reads_of_a_clause() {
    let sql = b"INSERT INTO t(a) VALUES(1) ON CONFLICT(a COLLATE NOCASE DESC) WHERE a>0 \
                DO UPDATE SET b=excluded.b WHERE b<5 ON CONFLICT DO NOTHING";
    let (arena, change) = crate::parse::change(sql).unwrap();
    let crate::ast::Change::Insert(statement) = change else {
        panic!("an insert was written");
    };
    let clauses = arena.upserts(statement.upserts);
    assert_eq!(clauses.len(), 2);
    assert_eq!(arena.names(clauses[0].targets).len(), 1);
    assert!(clauses[0].over.is_some());
    assert!(clauses[0].writes);
    assert!(clauses[0].filter.is_some());
    assert!(arena.names(clauses[1].targets).is_empty());
    assert!(!clauses[1].writes);
    for sql in [
        b"INSERT INTO t VALUES(1) ON CONFLICT".as_slice(),
        b"INSERT INTO t VALUES(1) ON CONFLICT DO",
        b"INSERT INTO t VALUES(1) ON CONFLICT(a) DO UPDATE",
        b"INSERT INTO t VALUES(1) ON CONFLICT(a) DO UPDATE SET",
        b"INSERT INTO t VALUES(1) ON CONFLICT() DO NOTHING",
    ] {
        assert!(crate::parse::change(sql).is_err(), "{sql:?}");
    }
}

#[test]
fn a_table_that_keeps_its_rows_in_the_key_s_own_tree_is_not_written_this_way() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE k(a TEXT PRIMARY KEY, b) WITHOUT ROWID")
        .unwrap();
    writer.run(b"INSERT INTO k VALUES('x',1)").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO k VALUES('x',2) ON CONFLICT(a) DO NOTHING")
            .unwrap_err()
            .message(),
        "Unsupported"
    );
}

#[test]
fn a_clause_writes_the_row_of_a_table_that_has_no_key_of_its_own() {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE u(b UNIQUE, c)").unwrap();
    writer.run(b"INSERT INTO u VALUES('x',1)").unwrap();
    // The row the conflict found answers under the name of the table
    // and under `rowid`, and the row that was not written answers under
    // `excluded`.
    writer
        .run(b"INSERT INTO u VALUES('x',2) ON CONFLICT(b) DO UPDATE SET c=hex(excluded.c)||u.c||rowid")
        .unwrap();
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let held = database.rows_of(b"u").unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!(
        held.first().and_then(|(_, values)| values.get(1)),
        Some(&Value::Text(b"3211".to_vec()))
    );
    // A name of another table is no column of either row.
    assert_eq!(
        writer
            .run(b"INSERT INTO u VALUES('x',2) ON CONFLICT(b) DO UPDATE SET c=q.c")
            .unwrap_err()
            .message(),
        "no such column: q.c"
    );
    // A column the table does not hold is no column to write either.
    assert_eq!(
        writer
            .run(b"INSERT INTO u VALUES('x',2) ON CONFLICT(b) DO UPDATE SET zz=1")
            .unwrap_err()
            .message(),
        "no such column: zz"
    );
}

#[test]
fn a_clause_that_writes_the_key_moves_the_row() {
    let mut writer = writer();
    writer
        .run(b"INSERT INTO t VALUES(1,'z',5) ON CONFLICT(a) DO UPDATE SET a=7")
        .unwrap();
    assert_eq!(rows(&writer), "7|7|x|10\n");
    // The key another row holds is refused.
    writer.run(b"INSERT INTO t VALUES(2,'y',20)").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(7,'q',30) ON CONFLICT(a) DO UPDATE SET a=2")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.a"
    );
}

#[test]
fn a_trigger_that_says_the_row_stays_leaves_the_clause_nothing_to_write() {
    let mut writer = writer();
    writer
        .run(b"CREATE TRIGGER tr BEFORE UPDATE ON t BEGIN SELECT RAISE(IGNORE); END")
        .unwrap();
    writer
        .run(b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a) DO UPDATE SET c=99")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|10\n");
    // A trigger that says nothing leaves the row written, and the
    // trigger after it runs over what was written.
    writer.run(b"DROP TRIGGER tr").unwrap();
    writer.run(b"CREATE TABLE log(what)").unwrap();
    writer
        .run(b"CREATE TRIGGER tr AFTER UPDATE ON t BEGIN INSERT INTO log VALUES(new.c); END")
        .unwrap();
    writer
        .run(b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a) DO UPDATE SET c=99")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|x|99\n");
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    assert_eq!(
        database
            .rows_of(b"log")
            .unwrap()
            .first()
            .and_then(|(_, values)| values.first()),
        Some(&Value::Int(99))
    );
}

#[test]
fn the_row_the_statement_would_have_written_carries_the_key_it_was_given() {
    let mut writer = writer();
    writer
        .run(b"INSERT INTO t VALUES(1,'y',20) ON CONFLICT(a ASC) DO UPDATE SET b='z', c=excluded.rowid WHERE c>0")
        .unwrap();
    assert_eq!(rows(&writer), "1|1|z|1\n");
    // A key that is no whole number is refused, which is what the key
    // of a table answers for a value of another type.
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,'q',30) ON CONFLICT(a) DO UPDATE SET a='abc'")
            .unwrap_err()
            .message(),
        "datatype mismatch"
    );
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES('abc',2,3)")
            .unwrap_err()
            .message(),
        "datatype mismatch"
    );
    assert_eq!(
        writer.run(b"UPDATE t SET a='abc'").unwrap_err().message(),
        "datatype mismatch"
    );
}
