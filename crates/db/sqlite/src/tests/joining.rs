// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The words a join is written with, what an `ON` may name, and what
//! a trigger may not carry.

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

/// A trigger runs with no statement of its own to bind against, so a
/// variable anywhere in it is refused, and a write of its body takes
/// the name of a table alone.
#[test]
fn what_a_trigger_may_not_carry() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(a,b)").unwrap();
    writer.run(b"CREATE TABLE t2(c,d)").unwrap();
    for body in [
        "AFTER INSERT ON t1 WHEN new.a = ? BEGIN SELECT 1; END",
        "BEFORE DELETE ON t1 BEGIN SELECT ?; END",
        "BEFORE DELETE ON t1 BEGIN SELECT * FROM (SELECT * FROM (SELECT ?)); END",
        "BEFORE DELETE ON t1 BEGIN SELECT * FROM t2 GROUP BY ?; END",
        "BEFORE DELETE ON t1 BEGIN SELECT * FROM t2 LIMIT ?; END",
        "BEFORE DELETE ON t1 BEGIN SELECT * FROM t2 ORDER BY ?; END",
        "BEFORE UPDATE ON t1 BEGIN UPDATE t2 SET c = ?; END",
        "BEFORE UPDATE ON t1 BEGIN UPDATE t2 SET c = 1 WHERE d = ?; END",
        "BEFORE INSERT ON t1 BEGIN INSERT INTO t2 SELECT $1, 1 FROM t1; END",
    ] {
        let sql = alloc::format!("CREATE TRIGGER tr1 {body}");
        assert_eq!(
            writer.run(sql.as_bytes()).unwrap_err().message(),
            "trigger cannot use variables",
            "{body}"
        );
    }
    // A variable inside a text is the text and not a variable.
    writer
        .run(b"CREATE TRIGGER tr1 AFTER INSERT ON t1 BEGIN SELECT '?'; END")
        .expect("a trigger");
    for body in [
        "AFTER UPDATE ON t1 BEGIN INSERT INTO main.t2 VALUES(new.a, new.b); END",
        "AFTER UPDATE ON t1 BEGIN UPDATE main.t2 SET c = 1; END",
        "AFTER UPDATE ON t1 BEGIN DELETE FROM main.t2; END",
    ] {
        let sql = alloc::format!("CREATE TRIGGER tr2 {body}");
        assert_eq!(
            writer.run(sql.as_bytes()).unwrap_err().message(),
            concat!(
                "qualified table names are not allowed on ",
                "INSERT, UPDATE, and DELETE statements within triggers"
            ),
            "{body}"
        );
    }
    // A write that names the table alone stands.
    writer
        .run(b"CREATE TRIGGER tr2 AFTER UPDATE ON t1 BEGIN INSERT INTO t2 VALUES(1,2); END")
        .expect("a trigger");
}

/// `likelihood(X,Y)` takes a real written as one for `Y`, between
/// nought and one, and nothing else.
#[test]
fn what_the_second_argument_of_likelihood_may_be() {
    let image = three();
    let database = Database::open(&image).expect("a database");
    for sql in [
        "SELECT likelihood(123, 1.000001)",
        "SELECT likelihood(123, -0.000001)",
        "SELECT likelihood(123, 0.5+0.3)",
        "SELECT likelihood(123, 1)",
        "SELECT likelihood(123, '0.5')",
        "SELECT likelihood(123, NULL)",
    ] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            "second argument to likelihood() must be a constant between 0.0 and 1.0",
            "{sql}"
        );
    }
    // A call of another number of arguments is refused for that.
    for sql in ["SELECT likelihood(123)", "SELECT likelihood(1,2,3)"] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            "wrong number of arguments to function likelihood()",
            "{sql}"
        );
    }
    for sql in [
        "SELECT likelihood(123, 1.0)",
        "SELECT likelihood(456, 0.0)",
        "SELECT likelihood(NULL, 0.5)",
        "SELECT likelihood('test-string', 0.5)",
    ] {
        database.query(sql.as_bytes()).unwrap_or_else(|error| {
            panic!("{sql}: {}", error.message());
        });
    }
}

/// `#1` is the register of a routine the C library writes and no
/// statement carries one, so the token is read and then refused.
#[test]
fn what_a_register_of_the_routine_is_refused_with() {
    let image = three();
    let database = Database::open(&image).expect("a database");
    for (sql, message) in [
        ("SELECT #1", "near \"#1\": syntax error"),
        ("SELECT 1 WHERE #0=1", "near \"#0\": syntax error"),
        ("SELECT a FROM aa ORDER BY #2", "near \"#2\": syntax error"),
    ] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            message,
            "{sql}"
        );
    }
}

/// A column added to a table that points at a row of another falls
/// back to nothing, the rows the table already holds gaining no value
/// of their own.
#[test]
fn what_a_column_that_points_may_fall_back_to() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(a,b)").unwrap();
    writer.run(b"PRAGMA foreign_keys=on").unwrap();
    writer
        .run(b"ALTER TABLE t1 ADD COLUMN f REFERENCES t1")
        .expect("a column");
    writer
        .run(b"ALTER TABLE t1 ADD COLUMN h REFERENCES t1 DEFAULT NULL")
        .expect("a column");
    // `sqlite3ErrorIfNotEmpty` reads the table first, so a table that
    // holds no row takes the column whatever it falls back to.
    writer
        .run(b"ALTER TABLE t1 ADD COLUMN j REFERENCES t1 DEFAULT 4")
        .expect("a column");
    writer.run(b"CREATE TABLE t2(a)").unwrap();
    writer.run(b"INSERT INTO t2 VALUES(1)").unwrap();
    // A column that points at no row, and one that points and falls back
    // to nothing, are both taken by a table that holds a row.
    writer
        .run(b"ALTER TABLE t2 ADD COLUMN k DEFAULT 4")
        .expect("a column");
    writer
        .run(b"ALTER TABLE t2 ADD COLUMN l REFERENCES t1 DEFAULT NULL")
        .expect("a column");
    assert_eq!(
        writer
            .run(b"ALTER TABLE t2 ADD COLUMN g REFERENCES t1 DEFAULT 4")
            .unwrap_err()
            .message(),
        "Cannot add a REFERENCES column with non-NULL default value"
    );
    // A connection that holds its rows to no foreign key takes the
    // column, which `db->flags&SQLITE_ForeignKeys` reads.
    writer.run(b"PRAGMA foreign_keys=off").unwrap();
    writer
        .run(b"ALTER TABLE t2 ADD COLUMN g REFERENCES t1 DEFAULT 4")
        .expect("a column");
    // A column that points at no row takes the value it falls back to.
    writer
        .run(b"ALTER TABLE t1 ADD COLUMN i DEFAULT 4")
        .expect("a column");
}

/// A `WITH` term reads the terms beside it, whichever was written
/// first, and terms that read each other are refused naming the one
/// the statement reads.
#[test]
fn what_the_terms_of_a_with_may_read() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(x)").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1),(2)").unwrap();
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    // A term written after the one that reads it is answered first.
    assert_eq!(
        database
            .query(
                b"WITH tmp2(x) AS (SELECT * FROM tmp1), tmp1(a) AS (SELECT * FROM t1) \
                  SELECT * FROM tmp2"
            )
            .unwrap()
            .rows
            .len(),
        2
    );
    for (sql, message) in [
        (
            "WITH tmp2(x) AS (SELECT * FROM tmp1), tmp1(a) AS (SELECT * FROM tmp2) \
             SELECT * FROM tmp1",
            "circular reference: tmp1",
        ),
        (
            "WITH i(x) AS (SELECT * FROM j), j(x) AS (SELECT * FROM k), \
             k(x) AS (SELECT * FROM i) SELECT * FROM i",
            "circular reference: i",
        ),
        (
            "WITH i(x) AS (SELECT * FROM (SELECT * FROM j)), \
             j(x) AS (SELECT * FROM (SELECT * FROM i)) SELECT * FROM i",
            "circular reference: i",
        ),
        (
            "WITH tmp(a) AS (SELECT * FROM t1), tmp(a) AS (SELECT * FROM t1) \
             SELECT * FROM tmp",
            "duplicate WITH table name: tmp",
        ),
    ] {
        assert_eq!(
            database.query(sql.as_bytes()).unwrap_err().message(),
            message,
            "{sql}"
        );
    }
    // A circle the statement never reads is left unanswered, which is
    // what the C library does by answering a term where it is read.
    assert_eq!(
        database
            .query(
                b"WITH i(x) AS (SELECT * FROM j), j(x) AS (SELECT * FROM i) \
                  SELECT * FROM t1"
            )
            .unwrap()
            .rows
            .len(),
        2
    );
    assert_eq!(
        database
            .query(
                b"WITH a(x) AS (SELECT 1), b(x) AS (SELECT * FROM c), \
                  c(x) AS (SELECT * FROM b) SELECT * FROM a"
            )
            .unwrap()
            .rows
            .len(),
        1
    );
    // A term that reads a table the database does not hold is refused
    // for that table and not for a circle.
    assert_eq!(
        database
            .query(b"WITH i(x) AS (SELECT * FROM nowhere) SELECT * FROM i")
            .unwrap_err()
            .message(),
        "no such table: nowhere"
    );
}
