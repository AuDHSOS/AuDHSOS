// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The names a statement answers its columns under.

use alloc::string::String;
use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::{Database, Naming};
use crate::header::Encoding;

/// A connection over one table of two columns and one row.
fn written() -> Vec<u8> {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(a,b)").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1,2)").unwrap();
    writer.written()
}

/// The names a statement answers under, with the two pragmas at
/// `naming`.
fn names(image: &[u8], naming: Naming, sql: &[u8]) -> Vec<String> {
    let database = Database::open(image).expect("a database").naming(naming);
    database
        .query(sql)
        .expect("an answer")
        .names
        .iter()
        .map(|name| String::from_utf8_lossy(name).into_owned())
        .collect()
}

/// A connection told nothing names a column by the column and every
/// other expression by the text it was written as.
#[test]
fn what_a_connection_told_nothing_names_a_column() {
    let image = written();
    let naming = Naming::default();
    assert_eq!(names(&image, naming, b"SELECT a FROM t1"), ["a"]);
    assert_eq!(names(&image, naming, b"SELECT a AS z FROM t1"), ["z"]);
    assert_eq!(names(&image, naming, b"SELECT 1+2 FROM t1"), ["1+2"]);
    assert_eq!(names(&image, naming, b"SELECT t1 . a FROM t1"), ["a"]);
    assert_eq!(names(&image, naming, b"SELECT * FROM t1 AS x"), ["a", "b"]);
}

/// With both pragmas off a column answers under the text it was written
/// as, quotes, spaces and all.
#[test]
fn what_a_connection_told_neither_pragma_names_a_column() {
    let image = written();
    let naming = Naming {
        short: false,
        full: false,
    };
    assert_eq!(names(&image, naming, b"SELECT t1 . a FROM t1"), ["t1 . a"]);
    assert_eq!(names(&image, naming, b"SELECT a AS z FROM t1"), ["z"]);
    assert_eq!(names(&image, naming, b"SELECT 1+2 FROM t1"), ["1+2"]);
}

/// `full_column_names` names a column by its table, whichever name the
/// statement calls that table by, and a `*` by the name the statement
/// calls the side, but only where `short_column_names` is off.
#[test]
fn what_full_column_names_names_a_column() {
    let image = written();
    let long = Naming {
        short: false,
        full: true,
    };
    assert_eq!(names(&image, long, b"SELECT x.a FROM t1 AS x"), ["t1.a"]);
    assert_eq!(names(&image, long, b"SELECT a FROM t1"), ["t1.a"]);
    assert_eq!(names(&image, long, b"SELECT 1+2 FROM t1"), ["1+2"]);
    assert_eq!(
        names(&image, long, b"SELECT * FROM t1 AS x"),
        ["x.a", "x.b"]
    );
    assert_eq!(
        names(&image, long, b"SELECT x.* FROM t1 AS x"),
        ["x.a", "x.b"]
    );
    // A statement inside the `FROM` that carries no alias is named
    // after its own place.
    assert_eq!(
        names(&image, long, b"SELECT a FROM (SELECT 1 AS a)")[0]
            .split('.')
            .next_back()
            .unwrap_or_default(),
        "a"
    );
    assert!(names(&image, long, b"SELECT a FROM (SELECT 1 AS a)")[0].starts_with("(subquery-"));
    assert!(names(&image, long, b"SELECT * FROM (SELECT 1 AS a)")[0].starts_with("(subquery-"));
    let both = Naming {
        short: true,
        full: true,
    };
    assert_eq!(names(&image, both, b"SELECT x.a FROM t1 AS x"), ["t1.a"]);
    assert_eq!(names(&image, both, b"SELECT * FROM t1 AS x"), ["a", "b"]);
}

/// A statement that stands as a table has the names of its columns made
/// unique, and a name that already ends in a colon and digits loses
/// them before it takes its own.
#[test]
fn what_a_statement_that_stands_as_a_table_names_its_columns() {
    let image = written();
    let naming = Naming::default();
    assert_eq!(
        names(&image, naming, b"SELECT * FROM (SELECT a, a, a FROM t1)"),
        ["a", "a:1", "a:2"]
    );
    assert_eq!(
        names(
            &image,
            naming,
            b"SELECT * FROM (SELECT 1 AS \"a:1\", 2 AS a, 3 AS a)"
        ),
        ["a:1", "a", "a:2"]
    );
    // The tables inside brackets answer under their own names, so two
    // sides that hold one name answer it twice.
    assert_eq!(
        names(&image, naming, b"SELECT * FROM (t1 AS x, t1 AS y)"),
        ["a", "b", "a", "b"]
    );
}

/// A name no side holds, which the shape works out before the walk
/// refuses it.
#[test]
fn what_a_name_no_side_holds_is_refused_with() {
    let image = written();
    let long = Naming {
        short: false,
        full: true,
    };
    let database = Database::open(&image).expect("a database").naming(long);
    for (sql, message) in [
        (
            b"SELECT nosuch FROM t1".as_slice(),
            "no such column: nosuch",
        ),
        (b"SELECT q.a FROM t1", "no such column: q.a"),
    ] {
        assert_eq!(
            database.query(sql).unwrap_err().message(),
            message,
            "{sql:?}"
        );
    }
}

/// A `*` written where the statement reads no table.
#[test]
fn what_a_star_over_no_table_is_refused_with() {
    let image = written();
    let database = Database::open(&image).expect("a database");
    assert_eq!(
        database
            .query(b"SELECT 1 FROM (SELECT *)")
            .unwrap_err()
            .message(),
        "no tables specified"
    );
}

/// The two pragmas belong to the connection, which answers them back
/// and carries them for a caller that holds one writer per file.
#[test]
fn what_the_two_pragmas_answer_back() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    assert_eq!(writer.naming(), Naming::default());
    writer.run(b"PRAGMA short_column_names=OFF").unwrap();
    writer.run(b"PRAGMA full_column_names=ON").unwrap();
    assert_eq!(
        writer.naming(),
        Naming {
            short: false,
            full: true
        }
    );
    let kept = writer.kept();
    // A connection opened again was told no pragma.
    writer.kept_as(crate::pragma::Kept::default());
    assert_eq!(writer.naming(), Naming::default());
    writer.kept_as(kept);
    assert_eq!(
        writer.naming(),
        Naming {
            short: false,
            full: true
        }
    );
}
