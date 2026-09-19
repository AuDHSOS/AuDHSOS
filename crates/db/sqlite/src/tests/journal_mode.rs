// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The journal mode a connection is in, and how many columns a
//! statement an `IN` looks in answers.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// What one statement answers as one word.
fn word(writer: &mut Writer, sql: &[u8]) -> String {
    let rows = writer.run(sql).expect("an answer");
    let held = rows
        .first()
        .and_then(|row| row.first())
        .and_then(crate::value::Value::text)
        .unwrap_or_default();
    String::from_utf8_lossy(&held).into_owned()
}

/// The mode a connection is in is the one it was last set to, which it
/// answers whether the pragma names a schema or not, and a connection
/// with a transaction open is left in the mode it had.
#[test]
fn what_journal_mode_a_connection_is_in() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    assert_eq!(word(&mut writer, b"PRAGMA journal_mode"), "delete");
    assert_eq!(word(&mut writer, b"PRAGMA journal_mode=persist"), "persist");
    assert_eq!(word(&mut writer, b"PRAGMA journal_mode"), "persist");
    assert_eq!(word(&mut writer, b"PRAGMA main.journal_mode"), "persist");
    writer.run(b"CREATE TABLE t1(x)").unwrap();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1)").unwrap();
    // `sqlite3PragmaJournalMode` leaves the mode as it is inside a
    // transaction, so the pragma answers the mode it did not change.
    assert_eq!(
        word(&mut writer, b"PRAGMA journal_mode=truncate"),
        "persist"
    );
    assert_eq!(word(&mut writer, b"PRAGMA journal_mode=wal"), "persist");
    writer.run(b"COMMIT").unwrap();
    assert_eq!(
        word(&mut writer, b"PRAGMA journal_mode=truncate"),
        "truncate"
    );
    // A reader is told the mode of the connection that writes, and
    // answers `delete` where it is told none.
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    assert_eq!(
        database.query(b"PRAGMA journal_mode").unwrap().rows,
        alloc::vec![alloc::vec![Value::Text(b"delete".to_vec())]]
    );
    let told = Database::open(&image)
        .expect("a database")
        .journalling(writer.journalled());
    assert_eq!(
        told.query(b"PRAGMA journal_mode").unwrap().rows,
        alloc::vec![alloc::vec![Value::Text(b"truncate".to_vec())]]
    );
}

/// `PRAGMA journal_mode = X` with no schema in front of it sets the mode
/// of every database the connection holds, and one that names a schema
/// sets that database alone.
#[test]
fn what_journal_mode_the_databases_beside_the_first_are_in() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.opens(opening);
    writer.run(b"ATTACH 'one.db' AS one").unwrap();
    assert_eq!(word(&mut writer, b"PRAGMA one.journal_mode"), "delete");
    assert_eq!(word(&mut writer, b"PRAGMA journal_mode=persist"), "persist");
    assert_eq!(word(&mut writer, b"PRAGMA one.journal_mode"), "persist");
    // A pragma that names a schema leaves the other databases as they
    // were.
    assert_eq!(
        word(&mut writer, b"PRAGMA one.journal_mode=truncate"),
        "truncate"
    );
    assert_eq!(word(&mut writer, b"PRAGMA journal_mode"), "persist");
    assert_eq!(word(&mut writer, b"PRAGMA one.journal_mode"), "truncate");
}

/// The file the `ATTACH` of these tests names.
fn opening(file: &[u8]) -> Option<alloc::vec::Vec<u8>> {
    if file != b"one.db" {
        return None;
    }
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE u(b)").unwrap();
    Some(writer.written())
}

/// A statement an `IN` looks in answers one column for a bare value
/// and as many as a row of values holds, which is counted where the
/// statement is read and not where a row is.
#[test]
fn how_many_columns_a_statement_an_in_looks_in_answers() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t2(a,b,c)").unwrap();
    writer.run(b"CREATE TABLE t3(a,b,c)").unwrap();
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    for (sql, message) in [
        (
            b"SELECT * FROM t2 WHERE a IN (SELECT a, b FROM t3)".as_slice(),
            "sub-select returns 2 columns - expected 1",
        ),
        (
            b"SELECT * FROM t2 WHERE a IN (SELECT a, b FROM t3 UNION ALL SELECT a, b FROM t2)",
            "sub-select returns 2 columns - expected 1",
        ),
        (
            b"SELECT * FROM t2 WHERE (a,b) IN (SELECT a FROM t3)",
            "sub-select returns 1 columns - expected 2",
        ),
        (
            b"SELECT * FROM t2 WHERE a IN (VALUES(1,2))",
            "sub-select returns 2 columns - expected 1",
        ),
        // The walk carries the refusal of the first term it reads
        // past the terms after it.
        (
            b"SELECT * FROM t2 WHERE a IN (SELECT a, b FROM t3) AND b IN (SELECT a FROM t3)",
            "sub-select returns 2 columns - expected 1",
        ),
    ] {
        assert_eq!(
            database.query(sql).unwrap_err().message(),
            message,
            "{sql:?}"
        );
    }
    // A `*` is as wide as the tables it is over, which the count here
    // reads no table for, so the statement stands.
    database
        .query(b"SELECT * FROM t2 WHERE a IN (SELECT * FROM t3)")
        .expect("a statement the count here says nothing about");
    database
        .query(b"SELECT * FROM t2 WHERE a IN (SELECT a FROM t3)")
        .expect("an answer");
    database
        .query(b"SELECT * FROM t2 WHERE a IN (VALUES(1))")
        .expect("an answer");
}
