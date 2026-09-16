// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a page that has no room for a cell does, against the walk of
//! `PRAGMA integrity_check`.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// What `PRAGMA integrity_check` answers for the file the writer holds.
fn checked(writer: &Writer) -> Vec<Vec<u8>> {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    database
        .query(b"PRAGMA integrity_check")
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(crate::value::Value::text))
        .collect()
}

/// The file of `btree01.test`: thirty rows that fill a page of 64 KiB,
/// written again shorter, and one of them written again longer than a
/// page holds room for.
///
/// A row in the middle of a leaf that the parent's right pointer does
/// not name is where the quick balance answered wrongly: it wrote the
/// page it made into the parent's right pointer, which named a page of
/// the tree twice and left another with nothing naming it.
#[test]
fn a_row_written_longer_than_the_page_holds_leaves_the_tree_whole() {
    let mut writer = Writer::new(65536, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t1(a INTEGER PRIMARY KEY, b BLOB)")
        .unwrap();
    for size in [3000_usize, 2000] {
        for at in [1_usize, 10, 15, 30] {
            for sql in [
                b"DELETE FROM t1".to_vec(),
                b"WITH RECURSIVE c(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM c WHERE i<30) \
                  INSERT INTO t1(a,b) SELECT i, zeroblob(6500) FROM c"
                    .to_vec(),
                alloc::format!("UPDATE t1 SET b=zeroblob({size})").into_bytes(),
                alloc::format!("UPDATE t1 SET b=zeroblob(64000) WHERE a={at}").into_bytes(),
            ] {
                writer.run(&sql).unwrap();
            }
            assert_eq!(checked(&writer), alloc::vec![b"ok".to_vec()], "{size}/{at}");
            let image = writer.written();
            let database = Database::open(&image).unwrap();
            assert_eq!(
                database
                    .query(b"SELECT count(*), sum(length(b)) FROM t1")
                    .unwrap()
                    .rows
                    .first(),
                Some(&alloc::vec![
                    Value::Int(30),
                    Value::Int(
                        i64::try_from(size.saturating_mul(29).saturating_add(64000)).unwrap()
                    )
                ]),
                "{size}/{at}"
            );
        }
    }
}

/// The quick balance itself: a row written at the end of the right-most
/// leaf, which is the one place it holds.
#[test]
fn a_row_written_past_the_last_of_them_goes_on_a_page_of_its_own() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a INTEGER PRIMARY KEY, b)")
        .unwrap();
    for i in 1..=60 {
        writer
            .run(alloc::format!("INSERT INTO t VALUES({i}, zeroblob(200))").as_bytes())
            .unwrap();
    }
    assert_eq!(checked(&writer), alloc::vec![b"ok".to_vec()]);
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    assert_eq!(
        database
            .query(b"SELECT count(*) FROM t")
            .unwrap()
            .rows
            .first()
            .and_then(|row| row.first()),
        Some(&Value::Int(60))
    );
}

/// What an `INSERT` and an `UPDATE` refuse a column or a row of the
/// wrong width with, which is `sqlite3Insert` and `sqlite3Update`
/// naming the table, the column and the counts.
#[test]
fn the_columns_a_statement_names_are_counted_against_the_values() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b,c)").unwrap();
    for (sql, message) in [
        (
            b"INSERT INTO t VALUES(1,2)".as_slice(),
            "table t has 3 columns but 2 values were supplied",
        ),
        (b"INSERT INTO t(a,b) VALUES(1)", "1 values for 2 columns"),
        (
            b"INSERT INTO t(a,zz) VALUES(1,2)",
            "table t has no column named zz",
        ),
        (
            b"INSERT INTO t SELECT 1,2",
            "table t has 3 columns but 2 values were supplied",
        ),
        (b"INSERT INTO t(a,b) SELECT 1,2,3", "3 values for 2 columns"),
        (b"UPDATE t SET zz=1", "no such column: zz"),
    ] {
        assert_eq!(writer.run(sql).unwrap_err().message(), message, "{sql:?}");
    }
}

/// What a `CREATE` of a name the schema already holds is refused with,
/// which is `sqlite3StartTable`, `sqlite3CreateIndex` and
/// `sqlite3FinishTrigger` naming what holds it.
#[test]
fn a_name_the_schema_already_holds_names_what_holds_it() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a)".as_slice(),
        b"CREATE INDEX i ON t(a)",
        b"CREATE VIEW v AS SELECT 1",
        b"CREATE TRIGGER tr AFTER INSERT ON t BEGIN SELECT 1; END",
    ] {
        writer.run(sql).unwrap();
    }
    for (sql, message) in [
        (b"CREATE TABLE t(b)".as_slice(), "table t already exists"),
        (b"CREATE TABLE v(b)", "view v already exists"),
        (b"CREATE VIEW v AS SELECT 2", "view v already exists"),
        (b"CREATE VIEW t AS SELECT 2", "table t already exists"),
        (b"CREATE INDEX i ON t(a)", "index i already exists"),
        (
            b"CREATE INDEX t ON t(a)",
            "there is already a table named t",
        ),
        (
            b"CREATE INDEX v ON t(a)",
            "there is already a table named v",
        ),
        (b"CREATE TABLE i(b)", "there is already an index named i"),
        (
            b"CREATE TRIGGER tr AFTER INSERT ON t BEGIN SELECT 2; END",
            "trigger tr already exists",
        ),
        (
            b"CREATE TABLE sqlite_x(a)",
            "object name reserved for internal use: sqlite_x",
        ),
        (
            b"CREATE INDEX sqlite_y ON t(a)",
            "object name reserved for internal use: sqlite_y",
        ),
        (
            b"CREATE TRIGGER sqlite_tr AFTER INSERT ON t BEGIN SELECT 1; END",
            "object name reserved for internal use: sqlite_tr",
        ),
    ] {
        assert_eq!(writer.run(sql).unwrap_err().message(), message, "{sql:?}");
    }
    // `IF NOT EXISTS` writes nothing and refuses nothing.
    for sql in [
        b"CREATE TABLE IF NOT EXISTS t(b)".as_slice(),
        b"CREATE INDEX IF NOT EXISTS i ON t(a)",
        b"CREATE VIEW IF NOT EXISTS v AS SELECT 2",
        b"CREATE TRIGGER IF NOT EXISTS tr AFTER INSERT ON t BEGIN SELECT 2; END",
    ] {
        assert_eq!(
            writer.run(sql).unwrap(),
            Vec::<Vec<Value>>::new(),
            "{sql:?}"
        );
    }
    // The two tables this crate writes for itself carry names SQLite
    // keeps, which it writes them under all the same.
    let mut counting = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    counting
        .run(b"CREATE TABLE c(a INTEGER PRIMARY KEY AUTOINCREMENT, b)")
        .unwrap();
    counting.run(b"INSERT INTO c(b) VALUES(1)").unwrap();
    counting.run(b"ANALYZE").unwrap();
    let image = counting.written();
    let database = Database::open(&image).unwrap();
    assert!(database.table(b"sqlite_sequence").is_some());
    assert!(database.table(b"sqlite_stat1").is_some());
    // `PRAGMA writable_schema=ON` lets a connection write a name SQLite
    // keeps, which is the first branch of `sqlite3CheckObjectName`.
    let mut open = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    open.run(b"PRAGMA writable_schema=ON").unwrap();
    assert_eq!(
        open.run(b"PRAGMA writable_schema").unwrap(),
        [[Value::Int(1)]]
    );
    open.run(b"CREATE TABLE sqlite_stat1(tbl,idx,stat)")
        .unwrap();
    open.run(b"PRAGMA writable_schema=RESET").unwrap();
    assert_eq!(
        open.run(b"PRAGMA writable_schema").unwrap(),
        [[Value::Int(0)]]
    );
    assert_eq!(
        open.run(b"CREATE TABLE sqlite_z(a)").unwrap_err().message(),
        "object name reserved for internal use: sqlite_z"
    );
}
