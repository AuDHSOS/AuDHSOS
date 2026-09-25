// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What an aggregate written where no group has been made is refused
//! with, in the words the C library refuses it with.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// A connection over a table of two columns and one row.
fn written() -> alloc::vec::Vec<u8> {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(a,b)").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1,2)").unwrap();
    writer.written()
}

/// What a statement is refused with.
fn refused(image: &[u8], sql: &[u8]) -> String {
    let database = Database::open(image).expect("a database");
    database.query(sql).unwrap_err().message()
}

/// A `*` for the arguments leaves a call with none, so every function
/// but `count` is refused for the number it was called with.
#[test]
fn what_a_star_for_the_arguments_is_refused_with() {
    let image = written();
    for (sql, message) in [
        (
            b"SELECT min(*) FROM t1".as_slice(),
            "wrong number of arguments to function min()",
        ),
        (
            b"SELECT SUM(*) FROM t1",
            "wrong number of arguments to function SUM()",
        ),
        (
            b"SELECT abs(*) FROM t1",
            "wrong number of arguments to function abs()",
        ),
        (b"SELECT nosuch(*) FROM t1", "no such function: nosuch"),
    ] {
        assert_eq!(refused(&image, sql), message, "{sql:?}");
    }
    let database = Database::open(&image).expect("a database");
    assert_eq!(
        database
            .query(b"SELECT count(*) FROM t1")
            .unwrap()
            .rows
            .len(),
        1
    );
}

/// An aggregate in a `WHERE`, in a `GROUP BY`, inside another
/// aggregate, and in the `ORDER BY` of a statement that groups nothing.
#[test]
fn what_an_aggregate_outside_a_group_is_refused_with() {
    let image = written();
    for (sql, message) in [
        (
            b"SELECT sum(min(a)) FROM t1".as_slice(),
            "misuse of aggregate function min()",
        ),
        (
            b"SELECT a FROM t1 WHERE min(a)>1",
            "misuse of aggregate function min()",
        ),
        (
            b"SELECT sum(a) FILTER (WHERE min(a)>0) FROM t1",
            "misuse of aggregate function min()",
        ),
        (
            b"SELECT a FROM t1 GROUP BY min(a)",
            "aggregate functions are not allowed in the GROUP BY clause",
        ),
        (
            b"SELECT a FROM t1 ORDER BY min(a)",
            "misuse of aggregate: min()",
        ),
    ] {
        assert_eq!(refused(&image, sql), message, "{sql:?}");
    }
}

/// An aggregate written where no statement groups anything, which is
/// every expression outside a `SELECT`.
#[test]
fn what_an_aggregate_outside_a_statement_is_refused_with() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(a,b)").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1,2)").unwrap();
    for sql in [
        b"INSERT INTO t1 VALUES(count(1),2)".as_slice(),
        b"UPDATE t1 SET a=count(a)",
        b"DELETE FROM t1 WHERE count(a)>0",
    ] {
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "misuse of aggregate function count()",
            "{sql:?}"
        );
    }
}

/// A name the statement answers an aggregate under, written inside
/// another aggregate.
#[test]
fn what_an_aliased_aggregate_inside_one_is_refused_with() {
    let image = written();
    for (sql, message) in [
        (
            b"SELECT min(a) AS m FROM t1 GROUP BY a HAVING max(m+5)<10".as_slice(),
            "misuse of aliased aggregate m",
        ),
        (
            b"SELECT coalesce(min(a)+5,11) AS m FROM t1 GROUP BY a HAVING max(m+5)<10",
            "misuse of aliased aggregate m",
        ),
        (
            b"SELECT count(a) AS cn FROM t1 GROUP BY a HAVING cn<max(cn)",
            "misuse of aliased aggregate cn",
        ),
        (
            b"SELECT 1+min(a) AS m FROM t1 GROUP BY a HAVING max(m+5)<10",
            "misuse of aliased aggregate m",
        ),
    ] {
        assert_eq!(refused(&image, sql), message, "{sql:?}");
    }
}

/// `DISTINCT` before the arguments of an aggregate that takes two, and
/// before those of a window function, which takes it nowhere; a scalar
/// reads it and drops it.
#[test]
fn what_a_distinct_before_the_arguments_is_refused_with() {
    let image = written();
    assert_eq!(
        refused(&image, b"SELECT group_concat(DISTINCT a,b) FROM t1"),
        "DISTINCT aggregates must have exactly one argument"
    );
    assert_eq!(
        refused(&image, b"SELECT count(DISTINCT a) OVER () FROM t1"),
        "DISTINCT is not supported for window functions"
    );
    let database = Database::open(&image).expect("a database");
    let answered = database.query(b"SELECT abs(DISTINCT -1) FROM t1").unwrap();
    assert_eq!(
        answered.rows,
        alloc::vec![alloc::vec![crate::value::Value::Int(1)]]
    );
}

/// An aggregate written where no group has been made is refused whether
/// a row reaches the statement that holds it or not, which is what
/// `sqlite3SelectPrep` reads before any statement runs.
#[test]
fn what_a_misplaced_aggregate_of_a_statement_inside_one_is_refused_with() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t0(a,b)").unwrap();
    let image = writer.written();
    let held = "(SELECT b FROM t0 ORDER BY sum(a))";
    for sql in [
        alloc::format!("SELECT {held} FROM t0"),
        alloc::format!("SELECT 1 FROM t0 WHERE EXISTS{held}"),
        alloc::format!("SELECT 1 FROM t0 WHERE a IN {held}"),
        alloc::format!("SELECT 1 FROM t0 GROUP BY {held}"),
        alloc::format!("SELECT 1 FROM t0 GROUP BY a HAVING {held}"),
        alloc::format!("SELECT 1 FROM t0 ORDER BY {held}"),
        alloc::format!("SELECT 1 FROM t0 LEFT JOIN t0 AS u ON {held}"),
        alloc::format!("SELECT 1 FROM {held}"),
        alloc::format!("SELECT 1 FROM t0 UNION SELECT {held} FROM t0"),
        alloc::format!("SELECT 1 FROM t0 WHERE EXISTS(SELECT 1 FROM t0 WHERE {held})"),
        // The walk stops at the first refusal, which the rest of the
        // expression the refused statement stands in does not undo.
        alloc::format!("SELECT 1 FROM t0 WHERE {held} AND 1"),
    ] {
        assert_eq!(
            refused(&image, sql.as_bytes()),
            "misuse of aggregate: sum()",
            "{sql}"
        );
    }
}

/// A name inside an aggregate of a statement written inside another one
/// stands for a column of that statement, which the walk over the
/// places the aggregates stand in reads no alias for.
#[test]
fn what_an_aggregate_of_a_statement_inside_one_answers() {
    let image = written();
    let database = Database::open(&image).expect("a database");
    assert_eq!(
        database
            .query(b"SELECT (SELECT sum(a) FROM t1) FROM t1")
            .unwrap()
            .rows,
        alloc::vec![alloc::vec![crate::value::Value::Int(1)]]
    );
}
