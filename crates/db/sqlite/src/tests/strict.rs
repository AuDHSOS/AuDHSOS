// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `CREATE TABLE ... STRICT`, against what the shell answers for the
//! same statements.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// What a statement answers, as `value|` per value with the type of
/// each in front of it where the statement asks for one.
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

/// A column of a `STRICT` table carries one of the six types, and the
/// statement names the column that carries another.
#[test]
fn a_column_of_a_strict_table_carries_one_of_the_six_types() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for (sql, message) in [
        (
            b"CREATE TABLE t1(a) STRICT".as_slice(),
            "missing datatype for t1.a",
        ),
        (
            b"CREATE TABLE t1(a BANJO PRIMARY KEY) WITHOUT ROWID, STRICT",
            "unknown datatype for t1.a: \"BANJO\"",
        ),
        (
            b"CREATE TABLE t1(a TEXT PRIMARY KEY, f TEXT(50)) STRICT",
            "unknown datatype for t1.f: \"TEXT(50)\"",
        ),
    ] {
        assert_eq!(writer.run(sql).unwrap_err().message(), message, "{sql:?}");
    }
    writer
        .run(b"CREATE TABLE t1(a INT, b INTEGER, c BLOB, d TEXT, e REAL, f ANY) STRICT")
        .unwrap();
}

/// A value written into a column of a `STRICT` table is converted under
/// the affinity of the column and then held to its type, which is
/// `OP_TypeCheck`.
#[test]
fn a_value_a_strict_column_may_not_hold_is_refused() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t1(a INT, b INTEGER, c BLOB, d TEXT, e REAL, f ANY) STRICT")
        .unwrap();
    for (sql, message) in [
        (
            b"INSERT INTO t1(a) VALUES('xyz')".as_slice(),
            "cannot store TEXT value in INT column t1.a",
        ),
        (
            b"INSERT INTO t1(b) VALUES('xyz')",
            "cannot store TEXT value in INTEGER column t1.b",
        ),
        (
            b"INSERT INTO t1(c) VALUES('456')",
            "cannot store TEXT value in BLOB column t1.c",
        ),
        (
            b"INSERT INTO t1(c) VALUES(2)",
            "cannot store INT value in BLOB column t1.c",
        ),
        (
            b"INSERT INTO t1(c) VALUES(2.5)",
            "cannot store REAL value in BLOB column t1.c",
        ),
        (
            b"INSERT INTO t1(d) VALUES(x'3142536475')",
            "cannot store BLOB value in TEXT column t1.d",
        ),
        (
            b"INSERT INTO t1(e) VALUES('xyz')",
            "cannot store TEXT value in REAL column t1.e",
        ),
        (
            b"INSERT INTO t1(a) VALUES(1.2)",
            "cannot store REAL value in INT column t1.a",
        ),
        (
            b"INSERT INTO t1(a) VALUES(x'313233')",
            "cannot store BLOB value in INT column t1.a",
        ),
    ] {
        assert_eq!(writer.run(sql).unwrap_err().message(), message, "{sql:?}");
    }
    // A column holds nothing whatever its type says.
    writer.run(b"INSERT INTO t1(a) VALUES(NULL)").unwrap();
    assert_eq!(shown(&writer, b"SELECT typeof(a) FROM t1"), "null|");
}

/// What a column of a `STRICT` table converts: a column of `INT` holds
/// the text of a whole number as that number, one of `REAL` holds a
/// whole number as a real, one of `TEXT` holds a number as its text,
/// and one of `ANY` holds whatever it is given.
#[test]
fn what_a_strict_column_converts_before_it_holds_a_value() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(a INT, b INTEGER, d TEXT, e REAL, f ANY) STRICT".as_slice(),
        b"INSERT INTO t1(a,b) VALUES(1,2),('3','4'),(5.0,6.0)",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(
        shown(&writer, b"SELECT typeof(a), a, typeof(b), b FROM t1"),
        "integer|1|integer|2|integer|3|integer|4|integer|5|integer|6|"
    );
    writer.run(b"DELETE FROM t1").unwrap();
    writer
        .run(b"INSERT INTO t1(e) VALUES(1),(2.5),('3'),('4.5')")
        .unwrap();
    assert_eq!(
        shown(&writer, b"SELECT typeof(e), e FROM t1"),
        "real|1.0|real|2.5|real|3.0|real|4.5|"
    );
    writer.run(b"DELETE FROM t1").unwrap();
    writer
        .run(b"INSERT INTO t1(d) VALUES('xyz'),(4),(5.5)")
        .unwrap();
    assert_eq!(
        shown(&writer, b"SELECT typeof(d), d FROM t1"),
        "text|xyz|text|4|text|5.5|"
    );
    writer.run(b"DELETE FROM t1").unwrap();
    writer
        .run(b"INSERT INTO t1(f) VALUES('x'),(1),(2.5),(x'ff')")
        .unwrap();
    assert_eq!(
        shown(&writer, b"SELECT typeof(f) FROM t1"),
        "text|integer|real|blob|"
    );
}

/// An `ALTER TABLE ... ADD COLUMN` that leaves the schema unreadable
/// names the alter that did it, which is `corruptSchema`.
#[test]
fn an_add_column_that_leaves_the_schema_unreadable_names_itself() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t4(c INT PRIMARY KEY) STRICT")
        .unwrap();
    assert_eq!(
        writer
            .run(b"ALTER TABLE t4 ADD COLUMN d VARCHAR")
            .unwrap_err()
            .message(),
        "error in table t4 after add column: unknown datatype for t4.d: \"VARCHAR\""
    );
    assert_eq!(
        writer
            .run(b"ALTER TABLE t4 ADD COLUMN d")
            .unwrap_err()
            .message(),
        "error in table t4 after add column: missing datatype for t4.d"
    );
    // The statement that was refused left the table as it found it.
    writer.run(b"ALTER TABLE t4 ADD COLUMN d TEXT").unwrap();
    assert_eq!(shown(&writer, b"SELECT count(*) FROM t4"), "0|");
}
