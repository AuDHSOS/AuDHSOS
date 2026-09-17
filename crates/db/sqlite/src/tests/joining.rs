// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The words a join is written with, and what an `ON` may name.

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// A database of three tables, one column each.
fn three() -> alloc::vec::Vec<u8> {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE aa(a)".as_slice(),
        b"CREATE TABLE bb(b)",
        b"CREATE TABLE cc(c)",
        b"INSERT INTO aa VALUES('one')",
        b"INSERT INTO bb VALUES('one')",
        b"INSERT INTO cc VALUES('one')",
    ] {
        writer.run(sql).unwrap();
    }
    writer.written()
}

/// A combination of words no join is written with is refused naming
/// the words, which `sqlite3JoinType` does: a word no join carries, and
/// `INNER` beside `OUTER`.
#[test]
fn what_words_a_join_is_written_with() {
    let image = three();
    let database = Database::open(&image).expect("a database");
    for (sql, message) in [
        (
            "SELECT * FROM aa INNER OUTER JOIN bb",
            "unknown join type: INNER OUTER",
        ),
        (
            "SELECT * FROM aa INNER OUTER CROSS JOIN bb",
            "unknown join type: INNER OUTER CROSS",
        ),
        (
            "SELECT * FROM aa OUTER NATURAL INNER JOIN bb",
            "unknown join type: OUTER NATURAL INNER",
        ),
        (
            "SELECT * FROM aa LEFT BOGUS JOIN bb",
            "unknown join type: LEFT BOGUS",
        ),
        (
            "SELECT * FROM aa INNER BOGUS CROSS JOIN bb",
            "unknown join type: INNER BOGUS CROSS",
        ),
    ] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            message,
            "{sql}"
        );
    }
    // A word that is neither a name nor one of SQL's own ends the
    // reading, and the statement is refused where `JOIN` should stand.
    assert_eq!(
        database
            .query(b"SELECT * FROM aa LEFT 5 JOIN bb")
            .unwrap_err()
            .message(),
        "near \"5\": syntax error"
    );
    // The combinations a join is written with stand.
    for sql in [
        "SELECT * FROM aa NATURAL LEFT OUTER JOIN bb",
        "SELECT * FROM aa CROSS JOIN bb",
        "SELECT * FROM aa LEFT JOIN bb ON a=b",
        "SELECT * FROM aa FULL OUTER JOIN bb ON a=b",
        "SELECT * FROM aa INNER JOIN bb ON a=b",
    ] {
        database.query(sql.as_bytes()).unwrap_or_else(|error| {
            panic!("{sql}: {}", error.message());
        });
    }
}

/// An `ON` of an outer join that names a table read after it is
/// refused, which the walk of `select.c` does; an inner join carries no
/// such mark, its `ON` being read as a `WHERE`.
#[test]
fn what_an_on_of_an_outer_join_may_name() {
    let image = three();
    let database = Database::open(&image).expect("a database");
    for sql in [
        "SELECT * FROM aa LEFT JOIN cc ON (a=b) JOIN bb ON (b=coalesce(c,1))",
        "SELECT * FROM aa RIGHT JOIN cc ON (a=b) JOIN bb ON (b=coalesce(c,1))",
        "SELECT * FROM aa LEFT JOIN cc ON (a=bb.b) JOIN bb ON (b=c)",
        // The name of a side read after this one, with a name of a side
        // read before it after that.
        "SELECT * FROM aa LEFT JOIN cc ON (b=a) JOIN bb ON (b=c)",
        // A name with the schema in front of it names no side a join
        // reads, so the walk passes over it and the name beside it is
        // the one the refusal names.
        "SELECT * FROM aa LEFT JOIN cc ON (main.aa.a=b) JOIN bb ON (b=c)",
    ] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            "ON clause references tables to its right",
            "{sql}"
        );
    }
    // An `ON` that names the sides read up to it stands.
    for sql in [
        "SELECT * FROM aa LEFT JOIN cc ON (a=c)",
        "SELECT * FROM aa LEFT JOIN cc ON (a=c) LEFT JOIN bb ON (b=c)",
    ] {
        database.query(sql.as_bytes()).unwrap_or_else(|error| {
            panic!("{sql}: {}", error.message());
        });
    }
}
