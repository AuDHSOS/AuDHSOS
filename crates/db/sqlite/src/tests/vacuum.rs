// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `VACUUM`, which makes the database again from nothing.

use crate::change::Writer;
use crate::db::{Database, Error};
use crate::header::Encoding;
use crate::value::Value;

/// A connection that ran the statements.
fn ran(statements: &[&str]) -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in statements {
        writer.run(sql.as_bytes()).expect(sql);
    }
    writer
}

/// What one statement answers over the file the writer holds.
fn answered(writer: &Writer, sql: &str) -> alloc::vec::Vec<alloc::vec::Vec<Value>> {
    let bytes = writer.written();
    Database::open(&bytes)
        .unwrap()
        .query(sql.as_bytes())
        .unwrap()
        .rows
}

/// `vacuum-1.*` of `test/vacuum.test`: the rows, the indexes and the
/// triggers stand after a vacuum, and the file holds no free page.
#[test]
fn the_rows_the_indexes_and_the_triggers_stand_after_a_vacuum() {
    let mut writer = ran(&[
        "CREATE TABLE t(a INTEGER PRIMARY KEY, b, c)",
        "CREATE INDEX i ON t(b, c)",
        "CREATE VIEW v AS SELECT b FROM t",
        "CREATE TABLE log(x)",
        "CREATE TRIGGER r AFTER DELETE ON t BEGIN INSERT INTO log VALUES(old.a); END",
        "INSERT INTO t VALUES(1,'one',1.5),(2,'two',x'00ff'),(3,NULL,NULL)",
        "DELETE FROM t WHERE a=2",
    ]);
    let before = answered(&writer, "SELECT a, b, c FROM t ORDER BY a");
    writer.run(b"VACUUM").unwrap();
    assert_eq!(
        answered(&writer, "SELECT a, b, c FROM t ORDER BY a"),
        before
    );
    // The trigger fired over the delete before the vacuum, and the row
    // it wrote stands.
    assert_eq!(
        answered(&writer, "SELECT x FROM log"),
        [alloc::vec![Value::Int(2)]]
    );
    // The index answers the row it holds, the view answers its rows, and
    // the trigger fires again.
    assert_eq!(
        answered(&writer, "SELECT a FROM t WHERE b='one'"),
        [alloc::vec![Value::Int(1)]]
    );
    assert_eq!(
        answered(&writer, "SELECT count(*) FROM v"),
        [alloc::vec![Value::Int(2)]]
    );
    writer.run(b"DELETE FROM t WHERE a=3").unwrap();
    assert_eq!(
        answered(&writer, "SELECT count(*) FROM log"),
        [alloc::vec![Value::Int(2)]]
    );
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    assert_eq!(
        database.query(b"PRAGMA freelist_count").unwrap().rows,
        [alloc::vec![Value::Int(0)]]
    );
    assert_eq!(
        crate::check::integrity(&database, false).unwrap(),
        [b"ok".to_vec()]
    );
}

/// `vacuum2-1.*`: a `PRAGMA page_size` written after the first table
/// says what the next vacuum writes the file under.
#[test]
fn a_page_size_written_after_the_first_table_is_what_the_vacuum_writes() {
    let mut writer = ran(&[
        "CREATE TABLE t(a)",
        "INSERT INTO t VALUES(1),(2),(3)",
        "PRAGMA page_size=4096",
    ]);
    // The pragma changes nothing until the vacuum runs.
    assert_eq!(
        answered(&writer, "PRAGMA page_size"),
        [alloc::vec![Value::Int(1024)]]
    );
    writer.run(b"VACUUM").unwrap();
    assert_eq!(
        answered(&writer, "PRAGMA page_size"),
        [alloc::vec![Value::Int(4096)]]
    );
    assert_eq!(
        answered(&writer, "SELECT count(*) FROM t"),
        [alloc::vec![Value::Int(3)]]
    );
}

/// A table without a rowid and one whose key counts up are written again
/// as they stood.
#[test]
fn a_table_without_a_rowid_and_one_that_counts_up_stand_after_a_vacuum() {
    let mut writer = ran(&[
        "CREATE TABLE k(a TEXT, b, PRIMARY KEY(a)) WITHOUT ROWID",
        "CREATE TABLE s(a INTEGER PRIMARY KEY AUTOINCREMENT, b)",
        "INSERT INTO k VALUES('x',1),('y',2)",
        "INSERT INTO s(b) VALUES(1),(2)",
        "DELETE FROM s WHERE a=2",
    ]);
    writer.run(b"VACUUM").unwrap();
    assert_eq!(
        answered(&writer, "SELECT a, b FROM k ORDER BY a"),
        [
            alloc::vec![Value::Text(b"x".to_vec()), Value::Int(1)],
            alloc::vec![Value::Text(b"y".to_vec()), Value::Int(2)],
        ]
    );
    // The counter the table keeps stands, so the next key is past the
    // one the delete took away.
    writer.run(b"INSERT INTO s(b) VALUES(3)").unwrap();
    assert_eq!(
        answered(&writer, "SELECT a FROM s ORDER BY a"),
        [alloc::vec![Value::Int(1)], alloc::vec![Value::Int(3)]]
    );
}

/// A file that vacuums itself keeps its maps, and an index a `UNIQUE`
/// made carries no statement of its own and is made again with its
/// table.
#[test]
fn a_file_that_vacuums_itself_and_an_index_a_unique_made_stand() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.vacuuming(true);
    writer.run(b"CREATE TABLE t(a UNIQUE, b)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x'),(2,'y')").unwrap();
    // A pragma the file holds, written after the first table, changes
    // nothing and is not the page size the vacuum reads.
    writer.run(b"PRAGMA auto_vacuum=0").unwrap();
    writer.run(b"VACUUM temp").unwrap();
    assert_eq!(
        answered(&writer, "PRAGMA auto_vacuum"),
        [alloc::vec![Value::Int(2)]]
    );
    assert_eq!(
        answered(&writer, "SELECT b FROM t WHERE a=2"),
        [alloc::vec![Value::Text(b"y".to_vec())]]
    );
    // The index the `UNIQUE` made holds the rows again, so a row that
    // shares a value with one the table holds is refused.
    assert!(writer.run(b"INSERT INTO t VALUES(1,'z')").is_err());
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    assert_eq!(
        crate::check::integrity(&database, false).unwrap(),
        [b"ok".to_vec()]
    );
}

/// A `VACUUM` inside a transaction, one that names a schema this
/// connection does not hold, and `VACUUM INTO` are refused.
#[test]
fn what_a_vacuum_is_refused_for() {
    let mut writer = ran(&["CREATE TABLE t(a)"]);
    assert_eq!(
        writer.run(b"VACUUM aux"),
        Err(Error::NoSchema(b"aux".to_vec()))
    );
    assert_eq!(writer.run(b"VACUUM INTO 'out.db'"), Err(Error::Unsupported));
    writer.run(b"BEGIN").unwrap();
    assert_eq!(writer.run(b"VACUUM"), Err(Error::VacuumInTransaction));
    writer.run(b"COMMIT").unwrap();
    writer.run(b"VACUUM main").unwrap();
    // A semicolon after the word names no schema, and a number is no
    // name at all.
    writer.run(b"VACUUM;").unwrap();
    assert!(writer.run(b"VACUUM 5").is_err());
    // A word that may not be a name is no schema either.
    assert!(writer.run(b"VACUUM SELECT").is_err());
    // The two refusals carry the text the C library writes.
    assert_eq!(
        Error::NoSchema(b"aux".to_vec()).message(),
        "unknown database aux"
    );
    assert_eq!(
        Error::VacuumInTransaction.message(),
        "cannot VACUUM from within a transaction"
    );
}

/// `PRAGMA default_synchronous`: a name no version of the library holds
/// answers no row, which is `sqlite3Pragma` for a name it does not know.
#[test]
fn a_pragma_the_library_does_not_know_answers_no_row() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    let none: alloc::vec::Vec<alloc::vec::Vec<Value>> = alloc::vec::Vec::new();
    assert_eq!(writer.run(b"PRAGMA default_synchronous").unwrap(), none);
    assert_eq!(writer.run(b"PRAGMA default_synchronous=2").unwrap(), none);
    assert_eq!(writer.run(b"PRAGMA optimize").unwrap(), none);
    // A name this crate holds nothing for at all is still refused.
    assert_eq!(writer.run(b"PRAGMA bogus_name"), Err(Error::Unsupported));
}
