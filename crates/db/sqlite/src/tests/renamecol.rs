// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ALTER TABLE ... RENAME COLUMN`, against the answers of the C
//! library's shell.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// The statements the schema of the file the writer holds is made of.
fn schema(writer: &Writer) -> Vec<Vec<u8>> {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    database
        .query(b"SELECT sql FROM sqlite_schema WHERE sql IS NOT NULL")
        .unwrap()
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(crate::value::Value::text))
        .collect()
}

#[test]
fn every_statement_of_the_schema_is_written_again_under_the_new_name() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a, b, c CHECK(c>a), d DEFAULT 7, e AS (a*2), PRIMARY KEY(a,b), UNIQUE(b))"
            .as_slice(),
        b"CREATE TABLE u(x REFERENCES t(a), y)",
        b"CREATE INDEX ti ON t(a, b) WHERE a>0",
        b"CREATE INDEX tx ON t(abs(a))",
        b"CREATE TRIGGER tr AFTER UPDATE OF a ON t WHEN new.a>0 \
          BEGIN UPDATE t SET b=new.a WHERE a=old.a; END",
        b"CREATE VIEW v AS SELECT a, b FROM t WHERE a>1",
    ] {
        writer.run(sql).unwrap();
    }
    writer.run(b"ALTER TABLE t RENAME COLUMN a TO zz").unwrap();
    let held = schema(&writer);
    let want: [&[u8]; 6] = [
        b"CREATE TABLE t(zz, b, c CHECK(c>zz), d DEFAULT 7, e AS (zz*2), \
          PRIMARY KEY(zz,b), UNIQUE(b))",
        b"CREATE TABLE u(x REFERENCES t(zz), y)",
        b"CREATE INDEX ti ON t(zz, b) WHERE zz>0",
        b"CREATE INDEX tx ON t(abs(zz))",
        b"CREATE TRIGGER tr AFTER UPDATE OF zz ON t WHEN new.zz>0 \
          BEGIN UPDATE t SET b=new.zz WHERE zz=old.zz; END",
        b"CREATE VIEW v AS SELECT zz, b FROM t WHERE zz>1",
    ];
    for (mine, wanted) in held.iter().zip(want) {
        assert_eq!(
            alloc::string::String::from_utf8_lossy(mine),
            alloc::string::String::from_utf8_lossy(wanted)
        );
    }
    assert_eq!(held.len(), want.len());
}

#[test]
fn the_rows_of_the_table_answer_under_the_new_name() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,2),(3,4)").unwrap();
    writer.run(b"CREATE INDEX ti ON t(a)").unwrap();
    writer.run(b"ALTER TABLE t RENAME a TO zz").unwrap();
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    assert_eq!(
        database
            .query(b"SELECT zz,b FROM t ORDER BY zz")
            .unwrap()
            .rows,
        [
            [Value::Int(1), Value::Int(2)],
            [Value::Int(3), Value::Int(4)]
        ]
    );
    assert_eq!(
        database
            .query(b"PRAGMA integrity_check")
            .unwrap()
            .rows
            .first()
            .and_then(|row| row.first().and_then(crate::value::Value::text)),
        Some(b"ok".to_vec())
    );
    // The old name names nothing.
    assert!(database.query(b"SELECT a FROM t").is_err());
}

#[test]
fn a_column_the_table_does_not_hold_and_a_table_sqlite_keeps_are_refused() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    assert_eq!(
        writer
            .run(b"ALTER TABLE t RENAME COLUMN zz TO yy")
            .unwrap_err()
            .message(),
        "no such column: \"zz\""
    );
    assert_eq!(
        writer
            .run(b"ALTER TABLE sqlite_master RENAME COLUMN name TO yy")
            .unwrap_err()
            .message(),
        "table sqlite_master may not be altered"
    );
    // A rename that leaves the table with two columns of one name is
    // refused, and the schema stands as it was.
    assert_eq!(
        writer
            .run(b"ALTER TABLE t RENAME COLUMN a TO b")
            .unwrap_err()
            .message(),
        "error in table t after rename: duplicate column name: b"
    );
    assert_eq!(schema(&writer), [b"CREATE TABLE t(a,b)".to_vec()]);
    // A name the statement wrote in quotes is written in quotes.
    writer
        .run(b"ALTER TABLE t RENAME COLUMN a TO \"y y\"")
        .unwrap();
    assert_eq!(schema(&writer), [b"CREATE TABLE t(\"y y\",b)".to_vec()]);
    // A view holds no columns of its own, so the three statements that
    // alter what a table holds name what they would have done to it.
    writer.run(b"CREATE VIEW v AS SELECT * FROM t").unwrap();
    for (sql, message) in [
        (
            b"ALTER TABLE v RENAME COLUMN b TO c".as_slice(),
            "cannot rename columns of view \"v\"",
        ),
        (
            b"ALTER TABLE v DROP COLUMN b",
            "cannot drop column from view \"v\"",
        ),
        (
            b"ALTER TABLE v DROP CONSTRAINT c",
            "cannot edit constraints of view \"v\"",
        ),
        (b"ALTER TABLE v RENAME TO w", "view v may not be altered"),
    ] {
        assert_eq!(writer.run(sql).unwrap_err().message(), message);
    }
}

#[test]
fn a_statement_that_names_another_table_is_left_alone() {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a,b)".as_slice(),
        b"CREATE TABLE u(a, d CHECK(a>0), e DEFAULT 1, f AS (a+1),           PRIMARY KEY(a), UNIQUE(d), FOREIGN KEY(d) REFERENCES u(a), CONSTRAINT ck CHECK(d>0))",
        b"CREATE INDEX ui ON u(a) WHERE a>0",
        b"CREATE TRIGGER ur AFTER UPDATE OF a ON u WHEN new.a>0           BEGIN INSERT INTO u(a) VALUES(1); UPDATE u SET a=2 WHERE a=3;           DELETE FROM u WHERE a=4; SELECT a FROM u; END",
        b"CREATE VIEW uv AS SELECT a FROM u",
    ] {
        writer.run(sql).unwrap();
    }
    let before = schema(&writer);
    writer.run(b"ALTER TABLE t RENAME COLUMN a TO zz").unwrap();
    let after = schema(&writer);
    assert_eq!(after.first(), Some(&b"CREATE TABLE t(zz,b)".to_vec()));
    assert_eq!(after.get(1..), before.get(1..));
}

/// The text a statement carries once `column` of `table` is written as
/// `to`, read out of the tree the parser built.
fn rewritten(sql: &[u8], table: &[u8], column: &[u8], to: &[u8]) -> Vec<u8> {
    let places = crate::rename::column_places(sql, table, column);
    crate::rename::written_as(sql, &places, to)
}

#[test]
fn every_place_a_statement_names_the_column() {
    // A name that begins with an underscore or past ASCII carries no
    // quotes.
    assert_eq!(
        alloc::string::String::from_utf8_lossy(&rewritten(
            b"CREATE TABLE t(_a, b)",
            b"t",
            b"_a",
            b"zz"
        )),
        "CREATE TABLE t(zz, b)"
    );
    assert_eq!(
        alloc::string::String::from_utf8_lossy(&rewritten(
            "CREATE TABLE t(é, b)".as_bytes(),
            b"t",
            "é".as_bytes(),
            b"zz"
        )),
        "CREATE TABLE t(zz, b)"
    );
    // A statement no schema holds names nothing.
    assert!(crate::rename::column_places(b"ALTER TABLE t RENAME a TO b", b"t", b"a").is_empty());
    assert!(crate::rename::column_places(b"CREATE TABLE t AS SELECT 1", b"t", b"a").is_empty());
    assert!(crate::rename::column_places(b"NOT A STATEMENT", b"t", b"a").is_empty());
    for (sql, want) in [
        // A column of the table, with a constraint the rename leaves
        // alone beside the ones it reads.
        (
            b"CREATE TABLE t(a NOT NULL COLLATE NOCASE, b)".as_slice(),
            b"CREATE TABLE t(zz NOT NULL COLLATE NOCASE, b)".as_slice(),
        ),
        // A `FOREIGN KEY` of another table that points at the column.
        (
            b"CREATE TABLE u(x, FOREIGN KEY(x) REFERENCES t(a))",
            b"CREATE TABLE u(x, FOREIGN KEY(x) REFERENCES t(zz))",
        ),
        // A `FOREIGN KEY` of the table over the column.
        (
            b"CREATE TABLE t(a, FOREIGN KEY(a) REFERENCES u(a))",
            b"CREATE TABLE t(zz, FOREIGN KEY(zz) REFERENCES u(a))",
        ),
        // A name written under another table names that table.
        (
            b"CREATE VIEW v AS SELECT t.a, u.a FROM t, u",
            b"CREATE VIEW v AS SELECT t.zz, u.a FROM t, u",
        ),
        // A `FROM` that names no table leaves a name under nothing
        // alone.
        (
            b"CREATE VIEW v AS SELECT a FROM (SELECT 1 AS a)",
            b"CREATE VIEW v AS SELECT a FROM (SELECT 1 AS a)",
        ),
        // `old` alone names the row the trigger stands on.
        (
            b"CREATE TRIGGER tr AFTER DELETE ON t BEGIN \
              INSERT INTO t(a) VALUES(old.a); DELETE FROM t; END",
            b"CREATE TRIGGER tr AFTER DELETE ON t BEGIN \
              INSERT INTO t(zz) VALUES(old.zz); DELETE FROM t; END",
        ),
        // The `ON CONFLICT` of an `INSERT` the trigger's body writes
        // names the columns of the table it writes.
        (
            b"CREATE TRIGGER tr AFTER DELETE ON t BEGIN \
              INSERT INTO t(a) VALUES(1) ON CONFLICT(a) WHERE a>1 \
              DO UPDATE SET a=a+1 WHERE a<9; END",
            b"CREATE TRIGGER tr AFTER DELETE ON t BEGIN \
              INSERT INTO t(zz) VALUES(1) ON CONFLICT(zz) WHERE zz>1 \
              DO UPDATE SET zz=zz+1 WHERE zz<9; END",
        ),
        // A place the statement wrote in quotes stays in quotes, and a
        // place it wrote bare is written bare.
        (
            b"CREATE TABLE t(\"a\" NOT NULL, b, CHECK(a>0))",
            b"CREATE TABLE t(\"zz\" NOT NULL, b, CHECK(zz>0))",
        ),
        // An `ON CONFLICT` that names no index, writes nothing and
        // holds the write to nothing names no column of its own, and
        // one of an `INSERT` over another table names none either.
        (
            b"CREATE TRIGGER tr AFTER DELETE ON t BEGIN \
              INSERT INTO t(a) VALUES(1) ON CONFLICT DO NOTHING; \
              INSERT INTO u(a) VALUES(1) ON CONFLICT(a) DO UPDATE SET a=2; END",
            b"CREATE TRIGGER tr AFTER DELETE ON t BEGIN \
              INSERT INTO t(zz) VALUES(1) ON CONFLICT DO NOTHING; \
              INSERT INTO u(a) VALUES(1) ON CONFLICT(a) DO UPDATE SET a=2; END",
        ),
        // A statement of a trigger's body over another table leaves its
        // own columns alone.
        (
            b"CREATE TRIGGER tr AFTER DELETE ON u BEGIN \
              INSERT INTO u(a) VALUES(1); UPDATE u SET a=2; DELETE FROM u WHERE a=3; END",
            b"CREATE TRIGGER tr AFTER DELETE ON u BEGIN \
              INSERT INTO u(a) VALUES(1); UPDATE u SET a=2; DELETE FROM u WHERE a=3; END",
        ),
    ] {
        assert_eq!(
            alloc::string::String::from_utf8_lossy(&rewritten(sql, b"t", b"a", b"zz")),
            alloc::string::String::from_utf8_lossy(want),
            "{}",
            alloc::string::String::from_utf8_lossy(sql)
        );
    }
}
