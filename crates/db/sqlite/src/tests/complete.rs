// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Whether a text ends a statement.

use crate::token::complete;

/// A text whose last token that carries meaning is a semicolon ends a
/// statement, and one that holds no token at all ends none.
#[test]
fn what_ends_a_statement() {
    for text in [
        b"SELECT 1;".as_slice(),
        b"SELECT 1;  ",
        b"SELECT 1; -- a comment",
        b"SELECT 1; /* a comment */",
        b"-- a comment ;\n ;",
        b"/* a comment ; */\n;",
        b"SELECT 'a;b';",
        b"SELECT \"a;b\";",
        b"SELECT `a;b`;",
        b"SELECT [a;b];",
        b"SELECT 1+2;",
        b"SELECT 1/2;",
        b"SELECT 1-2;",
        b";",
    ] {
        assert!(complete(text), "{text:?}");
    }
    for text in [
        b"".as_slice(),
        b"   ",
        b"This is a test",
        b"-- a comment ;",
        b"/* a comment ; */",
        b"/* a comment ;",
        b"SELECT 'a;b",
        b"SELECT \"a;b",
        b"SELECT `a;b",
        b"SELECT [a;b",
        b"SELECT 1",
        b"SELECT 1; SELECT 2",
    ] {
        assert!(!complete(text), "{text:?}");
    }
}

/// A `CREATE TRIGGER` ends at the `END` of its body and not at the
/// semicolons inside it, whatever words stand between `CREATE` and
/// `TRIGGER`.
#[test]
fn what_ends_a_create_trigger() {
    for text in [
        b"CREATE TRIGGER t AFTER INSERT ON x BEGIN SELECT 1; END;".as_slice(),
        b"CREATE TEMP TRIGGER t AFTER INSERT ON x BEGIN SELECT 1; END;",
        b"CREATE TEMPORARY TRIGGER t AFTER INSERT ON x BEGIN SELECT 1; END;",
        b"EXPLAIN CREATE TRIGGER t AFTER INSERT ON x BEGIN SELECT 1; END;",
        b"CREATE TRIGGER t AFTER INSERT ON x BEGIN SELECT 1; SELECT 2; END;",
        b"EXPLAIN SELECT 1;",
        b"CREATE TABLE t(a);",
        b"EXPLAIN CREATE TABLE t(a);",
        b"EXPLAIN EXPLAIN SELECT 1;",
        b"CREATE TEMP TABLE t(a);",
        b"CREATE END;",
        b"EXPLAIN END;",
    ] {
        assert!(complete(text), "{text:?}");
    }
    for text in [
        b"CREATE TRIGGER t AFTER INSERT ON x BEGIN SELECT 1;".as_slice(),
        b"CREATE TRIGGER t AFTER INSERT ON x BEGIN SELECT 1; END",
        b"CREATE TRIGGER t AFTER INSERT ON x BEGIN SELECT 1; SELECT 2;",
        b"CREATE TRIGGER t AFTER INSERT ON x BEGIN SELECT 1; END; SELECT 3",
        b"CREATE TRIGGER",
        b"EXPLAIN CREATE",
        b"EXPLAIN",
        b"CREATE",
        b"CREATE TEMP",
    ] {
        assert!(!complete(text), "{text:?}");
    }
}

/// `NULLS FIRST` and `NULLS LAST` are written where a statement sorts
/// rows and nowhere else.
#[test]
fn where_the_place_of_the_nulls_may_be_written() {
    let mut writer = crate::change::Writer::new(1024, 0, crate::header::Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t1(a, b, c, d, UNIQUE (b))")
        .unwrap();
    for (sql, message) in [
        (
            b"CREATE INDEX i1 ON t1(a ASC NULLS LAST)".as_slice(),
            "unsupported use of NULLS LAST",
        ),
        (
            b"CREATE INDEX i1 ON t1(a, b DESC NULLS FIRST)",
            "unsupported use of NULLS FIRST",
        ),
        (
            b"CREATE TABLE t2(a, b, PRIMARY KEY(a DESC, b NULLS FIRST))",
            "unsupported use of NULLS FIRST",
        ),
        (
            b"CREATE TABLE t2(a, b, UNIQUE(a DESC NULLS LAST, b))",
            "unsupported use of NULLS LAST",
        ),
        (
            b"INSERT INTO t1 VALUES(1,2,3,4) ON CONFLICT (b DESC NULLS LAST) DO NOTHING",
            "unsupported use of NULLS LAST",
        ),
    ] {
        assert_eq!(writer.run(sql).unwrap_err().message(), message, "{sql:?}");
    }
    // A statement that sorts rows writes them where it pleases.
    writer
        .run(b"CREATE INDEX i1 ON t1(a, b)")
        .expect("an index");
    let image = writer.written();
    let database = crate::db::Database::open(&image).expect("a database");
    database
        .query(b"SELECT a FROM t1 ORDER BY a NULLS LAST")
        .expect("an answer");
}
