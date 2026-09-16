// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a window refuses, in the words the C library refuses it with.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// A connection over a table of two columns and one row.
fn written() -> alloc::vec::Vec<u8> {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(a,b)").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1,1)").unwrap();
    writer.written()
}

/// What a statement is refused with.
fn refused(image: &[u8], sql: &[u8]) -> String {
    let database = Database::open(image).expect("a database");
    database.query(sql).unwrap_err().message()
}

/// A window function written where no window was worked out, a scalar
/// written under an `OVER`, and a name called with a number of
/// arguments it does not take.
#[test]
fn what_a_name_under_an_over_is_refused_with() {
    let image = written();
    for (sql, message) in [
        (
            b"SELECT sum(a) FROM t1 WHERE sum(a) OVER () > 1".as_slice(),
            "misuse of window function sum()",
        ),
        (
            b"SELECT trim(a) OVER () FROM t1",
            "trim() may not be used as a window function",
        ),
        (
            b"SELECT row_number(a) OVER () FROM t1",
            "wrong number of arguments to function row_number()",
        ),
        (
            b"SELECT nth_value(a) FROM t1",
            "wrong number of arguments to function nth_value()",
        ),
        (
            b"SELECT nth_value(a,1) FROM t1",
            "misuse of window function nth_value()",
        ),
        (
            b"SELECT row_number() FILTER (WHERE b>0) OVER () FROM t1",
            "FILTER clause may only be used with aggregate window functions",
        ),
        (
            b"SELECT count(a,b) OVER () FROM t1",
            "wrong number of arguments to function count()",
        ),
        (
            b"SELECT nosuch(a) OVER () FROM t1",
            "no such function: nosuch",
        ),
    ] {
        assert_eq!(refused(&image, sql), message, "{sql:?}");
    }
}

/// A window that names one no `WINDOW` clause defines, and one that
/// writes again what the window it builds on wrote.
#[test]
fn what_a_window_that_names_another_is_refused_with() {
    let image = written();
    for (sql, message) in [
        (
            b"SELECT sum(a) OVER (nosuch) FROM t1".as_slice(),
            "no such window: nosuch",
        ),
        (
            b"SELECT sum(a) OVER (win PARTITION BY b) FROM t1 WINDOW win AS (ORDER BY b)",
            "cannot override PARTITION clause of window: win",
        ),
        (
            b"SELECT sum(a) OVER (win ORDER BY b) FROM t1 WINDOW win AS (ORDER BY b)",
            "cannot override ORDER BY clause of window: win",
        ),
        (
            b"SELECT sum(a) OVER (win ORDER BY b) FROM t1 \
              WINDOW win AS (ROWS BETWEEN 1 PRECEDING AND CURRENT ROW)",
            "cannot override frame specification of window: win",
        ),
    ] {
        assert_eq!(refused(&image, sql), message, "{sql:?}");
    }
}

/// A frame offset and a count that are not the numbers they have to
/// be: a frame counted in rows takes a whole number and one counted in
/// the values of its term takes any.
#[test]
fn what_a_frame_offset_and_a_count_are_refused_with() {
    let image = written();
    for (sql, message) in [
        (
            b"SELECT ntile(0) OVER () FROM t1".as_slice(),
            "argument of ntile must be a positive integer",
        ),
        (
            b"SELECT nth_value(a,0) OVER () FROM t1",
            "second argument to nth_value must be a positive integer",
        ),
        (
            b"SELECT sum(a) OVER (ROWS BETWEEN -1 PRECEDING AND CURRENT ROW) FROM t1",
            "frame starting offset must be a non-negative integer",
        ),
        (
            b"SELECT sum(a) OVER (ROWS BETWEEN CURRENT ROW AND -1 FOLLOWING) FROM t1",
            "frame ending offset must be a non-negative integer",
        ),
        (
            b"SELECT sum(a) OVER (ORDER BY b RANGE BETWEEN -1 PRECEDING AND CURRENT ROW) FROM t1",
            "frame starting offset must be a non-negative number",
        ),
        (
            b"SELECT sum(a) OVER (ORDER BY b RANGE BETWEEN CURRENT ROW AND -1 FOLLOWING) FROM t1",
            "frame ending offset must be a non-negative number",
        ),
    ] {
        assert_eq!(refused(&image, sql), message, "{sql:?}");
    }
}
