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
