// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A name more than one side of a `FROM` answers to.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// A connection over two tables that share a column name.
fn written() -> alloc::vec::Vec<u8> {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(a,b)").unwrap();
    writer.run(b"CREATE TABLE t2(a,c)").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1,2)").unwrap();
    writer.run(b"INSERT INTO t2 VALUES(1,3)").unwrap();
    writer.written()
}

/// What a statement is refused with.
fn refused(image: &[u8], sql: &[u8]) -> String {
    let database = Database::open(image).expect("a database");
    database.query(sql).unwrap_err().message()
}

/// A bare name two tables hold, a name written with an alias two sides
/// carry, and the three names the rowid answers to.
#[test]
fn what_a_name_two_sides_answer_is_refused_with() {
    let image = written();
    for (sql, message) in [
        (
            b"SELECT a FROM t1, t2".as_slice(),
            "ambiguous column name: a",
        ),
        (
            b"SELECT a FROM t1 AS x, t1 AS y",
            "ambiguous column name: a",
        ),
        (
            b"SELECT x.a FROM t1 AS x, t1 AS x",
            "ambiguous column name: x.a",
        ),
        (b"SELECT rowid FROM t1, t2", "ambiguous column name: rowid"),
        (
            b"SELECT b FROM t1, t2 ORDER BY a",
            "ambiguous column name: a",
        ),
    ] {
        assert_eq!(refused(&image, sql), message, "{sql:?}");
    }
}

/// A name one `USING` or one `NATURAL` matched is answered once, and a
/// name of the enclosing statement is read there rather than here.
#[test]
fn what_a_name_one_side_answers_reads() {
    let image = written();
    let database = Database::open(&image).expect("a database");
    for sql in [
        b"SELECT a FROM t1 JOIN t2 USING(a)".as_slice(),
        b"SELECT a FROM t1 NATURAL JOIN t2",
        b"SELECT b FROM t1 WHERE a IN (SELECT a FROM t2 WHERE t2.c=b+1)",
    ] {
        let answered = database.query(sql).unwrap_or_else(|error| {
            panic!("{sql:?}: {}", error.message());
        });
        assert_eq!(answered.rows.len(), 1, "{sql:?}");
    }
}
