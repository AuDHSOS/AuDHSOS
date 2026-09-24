// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ALTER TABLE ... DROP COLUMN` over the rows of the table, against the
//! answers of the C library's shell.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// A writer over a file of 512-byte pages with the statements run.
fn written(sql: &[&[u8]]) -> Writer {
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for one in sql {
        writer.run(one).expect("a statement the writer takes");
    }
    writer
}

/// What one statement answers over the file the writer holds.
fn rows(writer: &Writer, sql: &[u8]) -> Vec<Vec<Value>> {
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    database.query(sql).expect("an answer").rows
}

/// One row of whole numbers.
fn numbers(held: &[i64]) -> Vec<Value> {
    held.iter().copied().map(Value::Int).collect()
}

/// A table that keeps its rows in the key's own tree holds a record of
/// another width once a column goes out, so every row is written again
/// under the key it already had.
#[test]
fn what_a_drop_column_over_a_table_that_keeps_its_rows_in_the_key_writes() {
    let mut writer = written(&[
        b"CREATE TABLE t1(a, b, c, PRIMARY KEY(a COLLATE nocase, a)) WITHOUT ROWID",
        b"INSERT INTO t1 VALUES(1, 2, 3)",
        b"INSERT INTO t1 VALUES(4, 5, 6)",
        b"ALTER TABLE t1 DROP COLUMN c",
    ]);
    assert_eq!(
        rows(&writer, b"SELECT sql FROM sqlite_schema"),
        alloc::vec![alloc::vec![Value::Text(
            b"CREATE TABLE t1(a, b, PRIMARY KEY(a COLLATE nocase, a)) WITHOUT ROWID".to_vec()
        )]]
    );
    assert_eq!(
        rows(&writer, b"SELECT * FROM t1"),
        alloc::vec![numbers(&[1, 2]), numbers(&[4, 5])]
    );
    assert_eq!(
        writer.run(b"PRAGMA integrity_check").unwrap(),
        alloc::vec![alloc::vec![Value::Text(b"ok".to_vec())]]
    );
}

/// A column the row computes for itself stands in no record, so the
/// record of a row written again holds the values before it and the
/// values after it and nothing of the column between them.
#[test]
fn what_a_drop_column_beside_a_column_the_row_computes_writes() {
    for kind in [b"VIRTUAL".as_slice(), b"STORED".as_slice()] {
        for rest in [b"".as_slice(), b"WITHOUT ROWID".as_slice()] {
            let made = [
                b"CREATE TABLE x1(a, b, c PRIMARY KEY, d AS (b+c) ".as_slice(),
                kind,
                b", e) ",
                rest,
            ]
            .concat();
            let mut writer = written(&[
                &made,
                b"INSERT INTO x1(a, b, c, e) VALUES(1, 2, 3, 4)",
                b"INSERT INTO x1(a, b, c, e) VALUES(5, 6, 7, 8)",
                b"ALTER TABLE x1 DROP COLUMN a",
            ]);
            let shown = alloc::string::String::from_utf8_lossy(&made);
            assert_eq!(
                rows(&writer, b"SELECT * FROM x1"),
                alloc::vec![numbers(&[2, 3, 5, 4]), numbers(&[6, 7, 13, 8])],
                "{shown}"
            );
            writer.run(b"ALTER TABLE x1 DROP COLUMN e").unwrap();
            assert_eq!(
                rows(&writer, b"SELECT * FROM x1"),
                alloc::vec![numbers(&[2, 3, 5]), numbers(&[6, 7, 13])],
                "{shown}"
            );
        }
    }
}

/// A column the row computes for itself that goes out leaves the values
/// of every other column where they stand.
#[test]
fn what_a_drop_of_a_column_the_row_computes_writes() {
    for kind in [b"VIRTUAL".as_slice(), b"STORED".as_slice()] {
        let made = [
            b"CREATE TABLE m(a, b PRIMARY KEY, c AS (a+b) ".as_slice(),
            kind,
            b", d) WITHOUT ROWID",
        ]
        .concat();
        let writer = written(&[
            &made,
            b"INSERT INTO m(a, b, d) VALUES(1, 2, 'hello')",
            b"INSERT INTO m(a, b, d) VALUES(3, 4, 'world')",
            b"ALTER TABLE m DROP COLUMN c",
        ]);
        let text = |held: &[u8]| Value::Text(held.to_vec());
        assert_eq!(
            rows(&writer, b"SELECT * FROM m"),
            alloc::vec![
                alloc::vec![Value::Int(1), Value::Int(2), text(b"hello")],
                alloc::vec![Value::Int(3), Value::Int(4), text(b"world")]
            ],
            "{}",
            alloc::string::String::from_utf8_lossy(&made)
        );
    }
}

/// The key of a row carries the rowid, so the record of a row written
/// again holds nothing under the column that names it.
#[test]
fn the_record_of_a_row_holds_nothing_under_the_column_that_names_the_rowid() {
    // `04 02` is a payload of four bytes under the rowid 2, and
    // `03 00 01 03` is the record: a header of three bytes, nothing
    // under `b`, and one byte holding 3 under `c`.
    let cell = [0x04, 0x02, 0x03, 0x00, 0x01, 0x03];
    for made in [
        b"CREATE TABLE t(a, b INTEGER PRIMARY KEY, c)".as_slice(),
        b"CREATE TABLE t(b INTEGER PRIMARY KEY, a, c)",
    ] {
        let writer = written(&[
            made,
            b"INSERT INTO t(a, b, c) VALUES(1, 2, 3)",
            b"ALTER TABLE t DROP COLUMN a",
        ]);
        let shown = alloc::string::String::from_utf8_lossy(made);
        let image = writer.written();
        assert!(
            image.windows(cell.len()).any(|held| held == cell),
            "{shown}"
        );
        assert_eq!(
            rows(&writer, b"SELECT * FROM t"),
            alloc::vec![numbers(&[2, 3])],
            "{shown}"
        );
    }
}
