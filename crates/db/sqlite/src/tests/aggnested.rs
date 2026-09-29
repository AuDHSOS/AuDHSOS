// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The statement an aggregate written inside another one belongs to.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// A connection over two tables, three rows and two rows.
fn written() -> alloc::vec::Vec<u8> {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(a1 INTEGER)".as_slice(),
        b"INSERT INTO t1 VALUES(1),(2),(3)",
        b"CREATE TABLE t2(b1 INTEGER)",
        b"INSERT INTO t2 VALUES(4),(5)",
    ] {
        writer.run(sql).unwrap();
    }
    writer.written()
}

/// The rows one statement answers, one value per row.
fn answered(image: &[u8], sql: &[u8]) -> String {
    let database = Database::open(image).expect("a database");
    database
        .query(sql)
        .unwrap_or_else(|error| panic!("{sql:?}: {}", error.message()))
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|value| match value {
                    Value::Int(number) => alloc::format!("{number}"),
                    Value::Real(number) => {
                        String::from_utf8_lossy(&crate::fp::text(*number, 15)).into_owned()
                    }
                    held => panic!("a number: {held:?}"),
                })
                .collect::<alloc::vec::Vec<_>>()
                .join("|")
        })
        .collect::<alloc::vec::Vec<_>>()
        .join(" ")
}

/// An aggregate belongs to the statement whose sides it reads a column
/// of, and to the statement it stands in where it reads a column of
/// none.
#[test]
fn which_statement_an_aggregate_written_inside_another_one_belongs_to() {
    let image = written();
    for (sql, rows) in [
        // The column of the enclosing statement holds the aggregate to
        // it, so it answers one row over every row of it.
        (b"SELECT (SELECT sum(a1) FROM t2) FROM t1".as_slice(), "6"),
        (b"SELECT (SELECT sum(t1.a1) FROM t2) FROM t1", "6"),
        (b"SELECT (SELECT sum(a1)) FROM t1", "6"),
        (b"SELECT sum(a1), (SELECT sum(a1) FROM t2) FROM t1", "6|6"),
        // A column of its own statement holds it there, so it answers
        // once per row of the enclosing statement.
        (b"SELECT (SELECT sum(b1) FROM t2) FROM t1", "9 9 9"),
        (b"SELECT (SELECT sum(t2.b1) FROM t2) FROM t1", "9 9 9"),
        // The rowid of a side is a column of it.
        (b"SELECT (SELECT sum(rowid) FROM t2) FROM t1", "3 3 3"),
        // An aggregate that reads no column at all belongs to the
        // statement it stands in.
        (b"SELECT (SELECT sum(1) FROM t2) FROM t1", "2 2 2"),
        (b"SELECT (SELECT count(*) FROM t2) FROM t1", "2 2 2"),
        // A statement of a `FROM` is read as deep as one written as a
        // value, so an aggregate of it belongs to the statement whose
        // column it reads.
        (b"SELECT (SELECT x FROM (SELECT sum(a1) AS x)) FROM t1", "6"),
        (
            b"SELECT (SELECT sum(a1) FROM (SELECT t2.* FROM t2)) FROM t1",
            "6",
        ),
        (
            b"SELECT (SELECT sum(a1) FROM (t2 JOIN t2 AS t4)) FROM t1",
            "6",
        ),
        (b"SELECT (SELECT sum(a1) FROM (VALUES(1),(2))) FROM t1", "6"),
        // A compound answers under the names of the whole, which this
        // engine does not read, so every name is read as one of the
        // statement that stands on it.
        (
            b"SELECT (SELECT sum(b1) FROM (SELECT b1 FROM t2 UNION ALL SELECT 9)) FROM t1",
            "18 18 18",
        ),
        // The expression of an argument holds the names as well, and a
        // statement written inside an argument answers names of its own,
        // which are not read as the aggregate's.
        (b"SELECT (SELECT sum(a1+1) FROM t2) FROM t1", "9"),
        (b"SELECT (SELECT sum((SELECT a1)) FROM t2) FROM t1", "6"),
        (
            b"SELECT (SELECT sum((SELECT b1 FROM t2 LIMIT 1)) FROM t2) FROM t1",
            "8 8 8",
        ),
        // The terms an aggregate reads its rows in the order of, and the
        // `FILTER` it reads them through, hold names as its arguments do.
        (
            b"SELECT (SELECT sum(a1 ORDER BY a1 DESC) FROM t2) FROM t1",
            "6",
        ),
        (
            b"SELECT (SELECT sum(a1) FILTER(WHERE b1>4) FROM t2) FROM t1",
            "1 2 3",
        ),
    ] {
        assert_eq!(answered(&image, sql), rows, "{sql:?}");
    }
    // A side whose columns are not the schema's holds every name to the
    // statement it stands in, and the name is refused where it reads it.
    let database = Database::open(&image).expect("a database");
    assert_eq!(
        database
            .query(b"SELECT (SELECT sum(a1) FROM nope) FROM t1")
            .unwrap_err()
            .message(),
        "no such table: nope"
    );
    // A side of a side whose columns are not the schema's holds every
    // name to the statement it stands in as well.
    for sql in [
        b"SELECT (SELECT sum(a1) FROM (SELECT x FROM nope)) FROM t1".as_slice(),
        b"SELECT (SELECT sum(a1) FROM (SELECT * FROM nope)) FROM t1",
    ] {
        assert_eq!(
            database.query(sql).unwrap_err().message(),
            "no such table: nope",
            "{sql:?}"
        );
    }
    // What one aggregate of a statement written inside another one is
    // refused for is read before the columns after it and before the
    // sides of it.
    for sql in [
        b"SELECT (SELECT group_concat(DISTINCT a1,'x'), 1 FROM t2) FROM t1".as_slice(),
        b"SELECT (SELECT group_concat(DISTINCT a1,'x') FROM (SELECT 1)) FROM t1",
    ] {
        assert_eq!(
            database.query(sql).unwrap_err().message(),
            "DISTINCT aggregates must have exactly one argument",
            "{sql:?}"
        );
    }
}

/// A `WHERE` and the arguments of a window function answer no aggregate
/// of the statement they belong to.
#[test]
fn what_an_aggregate_a_statement_answers_no_group_for_is_refused_with() {
    let image = written();
    let database = Database::open(&image).expect("a database");
    for (sql, message) in [
        (
            b"SELECT count(*) FROM t1 WHERE (SELECT sum(a1) FROM t2)".as_slice(),
            "misuse of aggregate: sum()",
        ),
        (
            b"SELECT count(*) FROM t1 WHERE (SELECT sum(1) FILTER(WHERE a1) FROM t2)",
            "misuse of aggregate: sum()",
        ),
        (
            b"SELECT ntile((SELECT sum(a1))) OVER(ORDER BY a1) FROM t1",
            "misuse of aggregate: sum()",
        ),
        (
            b"SELECT count(*) FROM t1 WHERE sum(a1)",
            "misuse of aggregate: sum()",
        ),
        (
            b"SELECT a1 FROM t1 WHERE sum(a1)",
            "misuse of aggregate function sum()",
        ),
    ] {
        assert_eq!(
            database.query(sql).unwrap_err().message(),
            message,
            "{sql:?}"
        );
    }
    // An aggregate written in the arguments of a window function belongs
    // to the statement that wrote it, which answers it over its group.
    assert_eq!(
        answered(&image, b"SELECT ntile(sum(a1)) OVER(ORDER BY a1) FROM t1"),
        "1"
    );
    assert_eq!(
        answered(
            &image,
            b"SELECT ntile((SELECT count(*) FROM t2)) OVER(ORDER BY a1) FROM t1"
        ),
        "1 1 2"
    );
    // A table-valued function is a side this engine does not answer, and
    // the names of it are read as the statement's own until it is.
    assert_eq!(
        database
            .query(b"SELECT (SELECT sum(a1) FROM pragma_table_info('t1')) FROM t1")
            .unwrap_err()
            .message(),
        "Unsupported"
    );
}
