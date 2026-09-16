// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a compound statement is refused with, and which column one
//! term of its `ORDER BY` counts to.

use alloc::string::String;
use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// A connection over two tables of one row each.
fn written() -> Vec<u8> {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(a,b)").unwrap();
    writer.run(b"CREATE TABLE t2(n)").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1,2)").unwrap();
    writer.run(b"INSERT INTO t2 VALUES(3)").unwrap();
    writer.written()
}

/// What a statement is refused with.
fn refused(image: &[u8], sql: &[u8]) -> String {
    let database = Database::open(image).expect("a database");
    database.query(sql).unwrap_err().message()
}

/// Two cores of a compound that answer different numbers of columns,
/// named by the word that joins them.
#[test]
fn what_two_cores_of_different_widths_are_refused_with() {
    let image = written();
    for (sql, message) in [
        (
            b"SELECT 1 UNION SELECT 2,3".as_slice(),
            "SELECTs to the left and right of UNION do not have the same number of result columns",
        ),
        (
            b"SELECT 1 UNION ALL SELECT 2,3",
            "SELECTs to the left and right of UNION ALL do not have the same number of result columns",
        ),
        (
            b"SELECT 1 EXCEPT SELECT 2,3",
            "SELECTs to the left and right of EXCEPT do not have the same number of result columns",
        ),
        (
            b"SELECT 1 INTERSECT SELECT 2,3",
            "SELECTs to the left and right of INTERSECT do not have the same number of result columns",
        ),
        (
            b"VALUES(1),(2,3)",
            "all VALUES must have the same number of terms",
        ),
        (
            b"VALUES(1) UNION VALUES(2,3)",
            "all VALUES must have the same number of terms",
        ),
    ] {
        assert_eq!(refused(&image, sql), message, "{sql:?}");
    }
}

/// An `ORDER BY` or a `LIMIT` written on a core other than the last.
#[test]
fn what_a_clause_before_the_word_is_refused_with() {
    let image = written();
    for (sql, message) in [
        (
            b"SELECT 1 ORDER BY 1 UNION ALL SELECT 2".as_slice(),
            "ORDER BY clause should come after UNION ALL not before",
        ),
        (
            b"SELECT 1 ORDER BY 1 UNION SELECT 2",
            "ORDER BY clause should come after UNION not before",
        ),
        (
            b"SELECT 1 LIMIT 1 UNION ALL SELECT 2",
            "LIMIT clause should come after UNION ALL not before",
        ),
        (
            b"SELECT 1 ORDER BY 1 EXCEPT SELECT 2",
            "ORDER BY clause should come after EXCEPT not before",
        ),
        (
            b"SELECT 1 ORDER BY 1 INTERSECT SELECT 2",
            "ORDER BY clause should come after INTERSECT not before",
        ),
    ] {
        assert_eq!(refused(&image, sql), message, "{sql:?}");
    }
}

/// A term of the `ORDER BY` of a compound counts to the column of the
/// first core that answers one of that name, and a term no core
/// answers is a refusal that says which term it is.
#[test]
fn which_column_a_term_of_the_order_by_counts_to() {
    let image = written();
    let database = Database::open(&image).expect("a database");
    // `n` is a column of the core on the right alone.
    let answered = database
        .query(b"SELECT a FROM t1 UNION ALL SELECT n FROM t2 ORDER BY n")
        .expect("an answer");
    assert_eq!(
        answered.rows,
        alloc::vec![alloc::vec![Value::Int(1)], alloc::vec![Value::Int(3)]]
    );
    assert_eq!(
        refused(
            &image,
            b"SELECT a FROM t1 UNION ALL SELECT n FROM t2 ORDER BY \"xyzzy\""
        ),
        "1st ORDER BY term does not match any column in the result set"
    );
    assert_eq!(
        refused(
            &image,
            b"SELECT a,b FROM t1 UNION ALL SELECT n,n FROM t2 ORDER BY a, \"xyzzy\""
        ),
        "2nd ORDER BY term does not match any column in the result set"
    );
}
