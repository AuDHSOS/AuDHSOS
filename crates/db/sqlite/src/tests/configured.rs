// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The same statements under every page size and every encoding a file
//! may be written in.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// The three encodings a file may hold its text in.
const ENCODINGS: [Encoding; 3] = [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be];

/// The page sizes a file may be written under.
const PAGES: [u32; 4] = [512, 1024, 4096, 65536];

/// What one statement answers.
fn rows(database: &Database, sql: &[u8]) -> Vec<Vec<Value>> {
    database.query(sql).expect("an answer").rows
}

/// A `DROP` finds the row of `sqlite_schema` that names the object
/// under every encoding, because the schema holds its text in the
/// encoding of the file and the name a statement carries never is.
#[test]
fn what_a_drop_finds_under_every_encoding() {
    for encoding in ENCODINGS {
        let mut writer = Writer::new(1024, 0, encoding).unwrap();
        for sql in [
            b"CREATE TABLE t1(a,b)".as_slice(),
            b"CREATE TABLE t2(a,b)",
            b"CREATE INDEX i2 ON t2(a)",
            b"CREATE VIEW v2 AS SELECT a FROM t2",
            b"CREATE TRIGGER g2 AFTER INSERT ON t2 BEGIN INSERT INTO t1 VALUES(1,2); END",
            b"INSERT INTO t2 VALUES(1,2)",
        ] {
            writer.run(sql).unwrap_or_else(|error| {
                panic!(
                    "{encoding:?} {}: {}",
                    String::from_utf8_lossy(sql),
                    error.message()
                )
            });
        }
        // The trigger wrote the row of `t1`, which says the statements
        // above reached the schema they name.
        let image = writer.written();
        assert_eq!(
            rows(&Database::open(&image).unwrap(), b"SELECT count(*) FROM t1"),
            alloc::vec![alloc::vec![Value::Int(1)]],
            "{encoding:?}"
        );
        for sql in [
            b"DROP TRIGGER g2".as_slice(),
            b"DROP VIEW v2",
            b"DROP INDEX i2",
            b"DROP TABLE t2",
            b"DROP TABLE t1",
        ] {
            writer.run(sql).unwrap_or_else(|error| {
                panic!(
                    "{encoding:?} {}: {}",
                    String::from_utf8_lossy(sql),
                    error.message()
                )
            });
        }
        let image = writer.written();
        assert_eq!(
            rows(
                &Database::open(&image).unwrap(),
                b"SELECT count(*) FROM sqlite_master"
            ),
            alloc::vec![alloc::vec![Value::Int(0)]],
            "{encoding:?}"
        );
        // A name the schema does not hold is refused under every
        // encoding too.
        assert_eq!(
            writer.run(b"DROP TABLE t2").unwrap_err().message(),
            "no such table: t2",
            "{encoding:?}"
        );
    }
}

/// The same rows are written, read and ordered under every page size
/// and every encoding, and the text comes back as it was written.
///
/// `BINARY` compares the bytes the file holds, which `sqlite3MemCompare`
/// does without reading them into another encoding, so a character
/// above the basic plane sorts before every other under UTF-16 little
/// endian and after them under UTF-8 and UTF-16 big endian.
#[test]
fn what_a_statement_answers_under_every_page_size_and_encoding() {
    for page in PAGES {
        for encoding in ENCODINGS {
            let order = if encoding == Encoding::Utf16Le {
                [3, 2, 1]
            } else {
                [2, 1, 3]
            };
            let mut writer = Writer::new(page, 0, encoding).unwrap();
            writer
                .run(b"CREATE TABLE t(a TEXT, b INTEGER)")
                .unwrap_or_else(|error| panic!("{page} {encoding:?}: {}", error.message()));
            writer
                .run("INSERT INTO t VALUES('ä',1),('b',2),('\u{1f600}',3)".as_bytes())
                .unwrap();
            writer.run(b"CREATE INDEX i ON t(a)").unwrap();
            let image = writer.written();
            let database = Database::open(&image).expect("a database");
            assert_eq!(
                rows(&database, b"SELECT b FROM t ORDER BY a"),
                order
                    .iter()
                    .map(|held| alloc::vec![Value::Int(*held)])
                    .collect::<Vec<_>>(),
                "{page} {encoding:?}"
            );
            assert_eq!(
                rows(&database, b"SELECT length(a), b FROM t WHERE a = 'b'"),
                alloc::vec![alloc::vec![Value::Int(1), Value::Int(2)]],
                "{page} {encoding:?} ascii"
            );
            assert_eq!(
                rows(&database, b"SELECT a FROM t WHERE b = 3"),
                alloc::vec![alloc::vec![Value::Text("\u{1f600}".as_bytes().to_vec())]],
                "{page} {encoding:?}"
            );
            assert_eq!(
                rows(
                    &database,
                    "SELECT length(a), b FROM t WHERE a = '\u{e4}'".as_bytes()
                ),
                alloc::vec![alloc::vec![Value::Int(1), Value::Int(1)]],
                "{page} {encoding:?}"
            );
        }
    }
}

/// A `DO UPDATE` writes the row it found and the values it sets in the
/// encoding the file names, so the text it wrote reads back as it was
/// written, over a table with a rowid and over one without.
#[test]
fn what_an_upsert_writes_under_every_encoding() {
    for encoding in ENCODINGS {
        for keyed in ["", " WITHOUT ROWID"] {
            let mut writer = Writer::new(1024, 0, encoding).unwrap();
            let mut sql = b"CREATE TABLE t(a INT PRIMARY KEY, b, c UNIQUE)".to_vec();
            sql.extend_from_slice(keyed.as_bytes());
            writer.run(&sql).unwrap();
            writer.run(b"INSERT INTO t VALUES(1,'old',3)").unwrap();
            writer
                .run(b"INSERT INTO t VALUES(1,'ignored',3) ON CONFLICT(a) DO UPDATE SET b='new'")
                .unwrap();
            let image = writer.written();
            let database = Database::open(&image).expect("a database");
            assert_eq!(
                rows(&database, b"SELECT a,b,c FROM t"),
                alloc::vec![alloc::vec![
                    Value::Int(1),
                    Value::Text(b"new".to_vec()),
                    Value::Int(3)
                ]],
                "{encoding:?}{keyed}"
            );
            // The row the conflict found is written again whole, so a
            // column the clause does not set keeps its text.
            writer
                .run(b"INSERT INTO t VALUES(1,'ignored',3) ON CONFLICT(c) DO UPDATE SET a=2")
                .unwrap();
            let image = writer.written();
            let database = Database::open(&image).expect("a database");
            assert_eq!(
                rows(&database, b"SELECT a,b,c FROM t"),
                alloc::vec![alloc::vec![
                    Value::Int(2),
                    Value::Text(b"new".to_vec()),
                    Value::Int(3)
                ]],
                "{encoding:?}{keyed} kept"
            );
        }
    }
}

/// The affinity of a column is applied to the text a statement wrote
/// and not to the bytes the file holds, so `'03'` into an `INTEGER`
/// column is the number 3 under every encoding.
#[test]
fn what_an_affinity_makes_of_text_under_every_encoding() {
    for encoding in ENCODINGS {
        let mut writer = Writer::new(1024, 0, encoding).unwrap();
        writer
            .run(b"CREATE TABLE t(i INTEGER, r REAL, n NUMERIC, t TEXT)")
            .unwrap();
        writer
            .run(b"INSERT INTO t VALUES('03','03','03','03')")
            .unwrap();
        // A text the affinity cannot read as a number stays text.
        writer
            .run(b"INSERT INTO t VALUES('1x','1x','1x','1x')")
            .unwrap();
        let image = writer.written();
        let database = Database::open(&image).expect("a database");
        assert_eq!(
            rows(&database, b"SELECT i, r, n, t FROM t"),
            alloc::vec![
                alloc::vec![
                    Value::Int(3),
                    Value::Real(3.0),
                    Value::Int(3),
                    Value::Text(b"03".to_vec())
                ],
                alloc::vec![
                    Value::Text(b"1x".to_vec()),
                    Value::Text(b"1x".to_vec()),
                    Value::Text(b"1x".to_vec()),
                    Value::Text(b"1x".to_vec())
                ]
            ],
            "{encoding:?}"
        );
    }
}

/// A trigger reads `new` and `old` as the statement wrote them, so a
/// value it carries into another table reads back as it was written.
#[test]
fn what_a_trigger_carries_under_every_encoding() {
    for encoding in ENCODINGS {
        let mut writer = Writer::new(1024, 0, encoding).unwrap();
        writer.run(b"CREATE TABLE t6(a)").unwrap();
        writer.run(b"CREATE TABLE seen(old, new)").unwrap();
        writer
            .run(
                b"CREATE TRIGGER g AFTER UPDATE ON t6 BEGIN                   INSERT INTO seen VALUES(old.a, new.a); END",
            )
            .unwrap();
        writer.run(b"INSERT INTO t6 VALUES('one')").unwrap();
        writer.run(b"UPDATE t6 SET a='two'").unwrap();
        let image = writer.written();
        let database = Database::open(&image).expect("a database");
        assert_eq!(
            rows(&database, b"SELECT old, new FROM seen"),
            alloc::vec![alloc::vec![
                Value::Text(b"one".to_vec()),
                Value::Text(b"two".to_vec())
            ]],
            "{encoding:?}"
        );
    }
}

/// `PRAGMA integrity_check` holds the entries of an index against the
/// rows of the table under every encoding, so a file the engine wrote
/// answers `ok`.
#[test]
fn what_the_integrity_check_answers_under_every_encoding() {
    for encoding in ENCODINGS {
        let mut writer = Writer::new(1024, 0, encoding).unwrap();
        writer.run(b"CREATE TABLE t7(a TEXT, b)").unwrap();
        writer.run(b"CREATE INDEX i7 ON t7(a)").unwrap();
        for value in ["one", "two", "three"] {
            let mut sql = b"INSERT INTO t7 VALUES('".to_vec();
            sql.extend_from_slice(value.as_bytes());
            sql.extend_from_slice(b"', 1)");
            writer.run(&sql).unwrap();
        }
        let image = writer.written();
        let database = Database::open(&image).expect("a database");
        assert_eq!(
            rows(&database, b"PRAGMA integrity_check"),
            alloc::vec![alloc::vec![Value::Text(b"ok".to_vec())]],
            "{encoding:?}"
        );
    }
}

/// A table tree over 512-byte pages grows past the page where the
/// parent of its right-most leaf fills, which `balance_quick` leaves to
/// the balance proper.
///
/// `in2.test` writes this shape: whole numbers, then a text of no
/// bytes, then short texts, each row at the end of the tree.
#[test]
fn a_tree_over_short_pages_grows_past_a_full_parent() {
    let held = 2000;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE a(i INTEGER PRIMARY KEY, a)")
        .unwrap();
    for at in 0..held {
        let sql = alloc::format!("INSERT INTO a VALUES({at},{at})");
        writer.run(sql.as_bytes()).unwrap();
    }
    writer.run(b"INSERT INTO a VALUES(4000, '')").unwrap();
    for at in 0..held {
        let sql = alloc::format!("INSERT INTO a VALUES(NULL,'x{at:04}')");
        writer
            .run(sql.as_bytes())
            .unwrap_or_else(|error| panic!("row {at}: {}", error.message()));
    }
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    assert_eq!(
        rows(&database, b"SELECT count(*) FROM a"),
        alloc::vec![alloc::vec![Value::Int(held * 2 + 1)]]
    );
    assert_eq!(
        rows(&database, b"PRAGMA integrity_check"),
        alloc::vec![alloc::vec![Value::Text(b"ok".to_vec())]]
    );
}

/// The bytes a constraint reads the schema out of are taken again
/// where the schema cookie moves, so a table written again under the
/// same name is held to the constraints it carries now.
#[test]
fn what_a_constraint_reads_after_the_schema_changed() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a CHECK(a > 0), b AS (a * 2), c NOT NULL DEFAULT 7)")
        .unwrap();
    writer.run(b"INSERT INTO t(a) VALUES(1)").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t(a) VALUES(-1)")
            .unwrap_err()
            .message(),
        "CHECK constraint failed: a > 0"
    );
    // The same name, other constraints: the cookie moved, so the bytes
    // the check reads are taken again.
    writer.run(b"DROP TABLE t").unwrap();
    writer
        .run(b"CREATE TABLE t(a CHECK(a < 0), b AS (a * 3), c NOT NULL DEFAULT 9)")
        .unwrap();
    writer.run(b"INSERT INTO t(a) VALUES(-1)").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t(a) VALUES(1)")
            .unwrap_err()
            .message(),
        "CHECK constraint failed: a < 0"
    );
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    assert_eq!(
        rows(&database, b"SELECT a, b, c FROM t"),
        alloc::vec![alloc::vec![Value::Int(-1), Value::Int(-3), Value::Int(9)]]
    );
}
