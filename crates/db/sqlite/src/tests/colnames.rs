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

/// The type the schema declares for each column a statement answers,
/// which `sqlite3_column_decltype` answers and which a column that came
/// from an expression carries none of.
#[test]
fn what_type_the_schema_declares_for_each_column_answered() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t1(a INTEGER, b VARCHAR(10), c)")
        .unwrap();
    writer.run(b"CREATE TABLE t2(x BLOB, y)").unwrap();
    writer
        .run(b"CREATE TABLE t3(k TEXT PRIMARY KEY) WITHOUT ROWID")
        .unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1,'b',3)").unwrap();
    writer.run(b"INSERT INTO t2 VALUES(x'00',5)").unwrap();
    writer.run(b"INSERT INTO t3 VALUES('k')").unwrap();
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    let declared = |sql: &[u8]| -> Vec<String> {
        database
            .query(sql)
            .unwrap()
            .declared
            .iter()
            .map(|held| String::from_utf8_lossy(held).into_owned())
            .collect()
    };
    // A column of a table carries the type the schema declares, and one
    // a `*` stands for carries it as well.
    assert_eq!(
        declared(b"SELECT a, b, c FROM t1"),
        ["INTEGER", "VARCHAR(10)", ""]
    );
    assert_eq!(
        declared(b"SELECT * FROM t1"),
        ["INTEGER", "VARCHAR(10)", ""]
    );
    // A column written with the name of its table, and one written under
    // the name of another table of the statement.
    assert_eq!(
        declared(b"SELECT t2.x, t1.a FROM t1, t2"),
        ["BLOB", "INTEGER"]
    );
    // A bare `rowid` of a table that keeps its rows under a key carries
    // the type of a whole number.
    assert_eq!(declared(b"SELECT rowid FROM t1"), ["INTEGER"]);
    assert_eq!(declared(b"SELECT k FROM t3"), ["TEXT"]);
    // A `rowid` of a statement written inside the `FROM` is no column of
    // it, so the statement carries no type for it and is refused.
    assert!(database.query(b"SELECT rowid FROM (SELECT 1)").is_err());
    // A column that came from an expression carries none, and a column
    // a statement written inside the `FROM` answers carries the type of
    // the column it came from, which `columnTypeImpl` reads through it.
    assert_eq!(declared(b"SELECT a+1, 'x', count(*) FROM t1"), ["", "", ""]);
    assert_eq!(declared(b"SELECT a FROM (SELECT a FROM t1)"), ["INTEGER"]);
    // A statement over no table answers a type for no column of it.
    assert_eq!(declared(b"SELECT 1"), [""]);
    assert_eq!(declared(b"VALUES(1,2)"), ["", ""]);
}

/// Where each result column comes from: the database, the table under
/// the name the schema holds it by, and the name the column carries
/// there.
#[test]
fn where_a_result_column_comes_from() {
    let image = written();
    let database = Database::open(&image).expect("a database");
    let origins = |sql: &[u8]| {
        database
            .query(sql)
            .expect("an answer")
            .origins
            .iter()
            .map(|origin| {
                (
                    String::from_utf8_lossy(&origin.schema).into_owned(),
                    String::from_utf8_lossy(&origin.table).into_owned(),
                    String::from_utf8_lossy(&origin.column).into_owned(),
                )
            })
            .collect::<Vec<_>>()
    };
    let held = |column: &str| {
        (
            String::from("main"),
            String::from("t1"),
            String::from(column),
        )
    };
    let nowhere = (String::new(), String::new(), String::new());
    assert_eq!(origins(b"SELECT a, b FROM t1"), [held("a"), held("b")]);
    // An alias moves the name the statement answers under and not the
    // name the schema holds the table by.
    assert_eq!(origins(b"SELECT x.a AS c FROM t1 AS x"), [held("a")]);
    assert_eq!(origins(b"SELECT rowid FROM t1"), [held("rowid")]);
    // A statement written inside a `FROM` carries the origins of its
    // own result columns.
    assert_eq!(origins(b"SELECT a FROM (SELECT a FROM t1)"), [held("a")]);
    assert_eq!(
        origins(b"SELECT a+1, 2 FROM t1"),
        [nowhere.clone(), nowhere]
    );
}

/// A bare name no side of the `FROM` answers stands for the name the
/// statement answers a column under, which every clause but the answer
/// itself reads.
#[test]
fn which_clauses_read_a_name_the_statement_answers_under() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t2(a,b)").unwrap();
    writer.run(b"INSERT INTO t2 VALUES(1,2),(3,4)").unwrap();
    let image = writer.written();
    let database = Database::open(&image).expect("a database");
    let answered = |sql: &[u8]| -> Vec<String> {
        database
            .query(sql)
            .expect("an answer")
            .rows
            .iter()
            .flatten()
            .map(|value| String::from_utf8_lossy(&value.text().unwrap_or_default()).into_owned())
            .collect()
    };
    assert_eq!(answered(b"SELECT b+1 AS x FROM t2 WHERE x>3"), ["5"]);
    assert_eq!(answered(b"SELECT b+1 AS x FROM t2 GROUP BY x"), ["3", "5"]);
    // The names of the other columns of the answer stand for nothing.
    assert_eq!(
        answered(b"SELECT b AS y, a AS x FROM t2 WHERE x>1"),
        ["4", "3"]
    );
    assert_eq!(answered(b"SELECT count(*) AS n FROM t2 HAVING n>1"), ["2"]);
    assert_eq!(answered(b"SELECT b AS x FROM t2 ORDER BY -x"), ["4", "2"]);
    // A statement written inside a clause reads the name as well, which
    // it reaches through the row of the statement that holds it.
    assert_eq!(answered(b"SELECT b AS x FROM t2 WHERE (SELECT x)>3"), ["4"]);
    // A column of a side answers the name before the answer does.
    assert!(answered(b"SELECT b AS a FROM t2 WHERE a>3").is_empty());
    let refused = |sql: &[u8]| database.query(sql).unwrap_err().message();
    // The answer itself reads no such name, so a name that stands for
    // itself and a name of another column of the answer are both
    // refused.
    assert_eq!(refused(b"SELECT x AS x FROM t2"), "no such column: x");
    assert_eq!(
        refused(b"SELECT a AS x, x+1 AS y FROM t2"),
        "no such column: x"
    );
    // The name carries what the expression it stands for was refused
    // with, whether the name stands in the statement that answers under
    // it or in one written inside that one.
    assert_eq!(
        refused(b"SELECT nosuch AS x FROM t2 WHERE x>0"),
        "no such column: nosuch"
    );
    assert_eq!(
        refused(b"SELECT nosuch AS x FROM t2 WHERE (SELECT x)>0"),
        "no such column: nosuch"
    );
}
