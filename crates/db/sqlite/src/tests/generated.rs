// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Columns a `GENERATED ALWAYS AS` computes, against what the shell
//! answers for the same statements.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// What a statement answers, as `value|` per value.
fn shown(writer: &Writer, sql: &[u8]) -> String {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let answered = database.query(sql).unwrap();
    let mut out = String::new();
    for row in &answered.rows {
        for value in row {
            out.push_str(&String::from_utf8_lossy(
                &value.text().unwrap_or(b"NULL".to_vec()),
            ));
            out.push('|');
        }
    }
    out
}

/// A column computed once is written down and a column computed where
/// it is read takes no place in the record, which is
/// `sqlite3ComputeGeneratedColumns` and `sqlite3TableColumnToStorage`.
#[test]
fn a_column_computed_once_is_written_down_and_one_computed_on_reading_is_not() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(c INT, a INT AS (c*3) VIRTUAL, d INT, b INT AS (c*2) STORED)".as_slice(),
        b"INSERT INTO t(c,d) VALUES(1,9)",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(shown(&writer, b"SELECT c,a,d,b FROM t"), "1|3|9|2|");
    // The columns a statement writes are computed again.
    writer.run(b"UPDATE t SET c=5").unwrap();
    assert_eq!(shown(&writer, b"SELECT c,a,d,b FROM t"), "5|15|9|10|");
}

/// One computed column names another, in either direction, and a
/// `STRICT` table holds the value each computes to the type of its
/// column.
#[test]
fn a_computed_column_names_another_and_is_held_to_its_type() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t4(a INT AS (b*2) VIRTUAL, b INT AS (c*2) STORED, c INT PRIMARY KEY) STRICT"
            .as_slice(),
        b"INSERT INTO t4(c) VALUES(1)",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(shown(&writer, b"SELECT a,b,c FROM t4"), "4|2|1|");
    writer
        .run(b"CREATE TABLE t5(k INT PRIMARY KEY, c TEXT AS (k*2) STORED) STRICT")
        .unwrap();
    writer.run(b"INSERT INTO t5(k) VALUES(1)").unwrap();
    assert_eq!(shown(&writer, b"SELECT typeof(c), c FROM t5"), "text|2|");
    writer
        .run(b"CREATE TABLE t6(k INT PRIMARY KEY, c BLOB AS (k*2) STORED) STRICT")
        .unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t6(k) VALUES(1)")
            .unwrap_err()
            .message(),
        "cannot store INT value in BLOB column t6.c"
    );
}

/// A name the schema does not hold carries no computed column, so the
/// row stands as it was given.
#[test]
fn a_table_the_schema_does_not_hold_computes_nothing() {
    use crate::value::Value;
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let mut values = alloc::vec![Value::Int(1)];
    database.compute_row(b"nosuch", &mut values).unwrap();
    assert_eq!(values, alloc::vec![Value::Int(1)]);
}

/// A statement writes a value into every column but the computed ones,
/// and one that names a computed column is refused.
#[test]
fn what_columns_a_statement_writes_a_value_into() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a, b AS (a+1) VIRTUAL, c, d AS (a*2) STORED)")
        .unwrap();
    // `INSERT INTO t VALUES(...)` writes the two columns no expression
    // computes, and the count in the refusal is that of those two.
    writer.run(b"INSERT INTO t VALUES(1,9)").unwrap();
    assert_eq!(shown(&writer, b"SELECT * FROM t"), "1|2|9|2|");
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,2,3)")
            .expect_err("a refusal")
            .message(),
        "table t has 2 columns but 3 values were supplied"
    );
    // A computed column an `INSERT` or an `UPDATE` names is refused.
    assert_eq!(
        writer
            .run(b"INSERT INTO t(a,b) VALUES(1,2)")
            .expect_err("a refusal")
            .message(),
        "cannot INSERT into generated column \"b\""
    );
    assert_eq!(
        writer
            .run(b"UPDATE t SET d=4")
            .expect_err("a refusal")
            .message(),
        "cannot UPDATE generated column \"d\""
    );
    // A table that keeps its rows in the key's own tree refuses the
    // same, which the other walk of an `UPDATE` reads.
    writer
        .run(b"CREATE TABLE u(a PRIMARY KEY, b AS (a+1)) WITHOUT ROWID")
        .unwrap();
    writer.run(b"INSERT INTO u VALUES(1)").unwrap();
    assert_eq!(shown(&writer, b"SELECT * FROM u"), "1|2|");
    assert_eq!(
        writer
            .run(b"UPDATE u SET b=4")
            .expect_err("a refusal")
            .message(),
        "cannot UPDATE generated column \"b\""
    );
}
