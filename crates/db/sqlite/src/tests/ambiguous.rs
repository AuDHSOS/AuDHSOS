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

/// What one value looks like as a list element.
fn shown(value: &crate::value::Value) -> String {
    match value {
        crate::value::Value::Int(number) => alloc::format!("{number}"),
        crate::value::Value::Text(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        held => panic!("a whole number or text: {held:?}"),
    }
}

/// A connection over the tables the rowid of a bracketed `FROM` is read
/// against: two that keep a rowid, two that keep their rows in the key's
/// own tree, one of those carrying a column called `rowid`, and one that
/// keeps a rowid while carrying a column of each of the three names.
fn keyed() -> alloc::vec::Vec<u8> {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE r1(a)".as_slice(),
        b"CREATE TABLE r2(b)",
        b"CREATE TABLE r3(rowid, _rowid_, oid)",
        b"CREATE TABLE w1(a PRIMARY KEY, b) WITHOUT ROWID",
        b"CREATE TABLE w2(a PRIMARY KEY, rowid) WITHOUT ROWID",
        b"CREATE TABLE w3(a PRIMARY KEY, b) WITHOUT ROWID",
        b"INSERT INTO r1(rowid, a) VALUES(101, 'A')",
        b"INSERT INTO r2(rowid, b) VALUES(55, 'B')",
        b"INSERT INTO r3 VALUES(7, 8, 9)",
        b"INSERT INTO w1 VALUES('mya', 'myb')",
        b"INSERT INTO w2 VALUES('mypk', 'myrowid')",
        b"INSERT INTO w3 VALUES('MYA', 'MYB')",
    ] {
        writer.run(sql).unwrap();
    }
    writer.written()
}

/// The rowid of each of the tables inside brackets is reached by a bare
/// `rowid`, `oid` or `_rowid_`, a column of that name is answered before
/// any of them, and two of them are ambiguous.
#[test]
fn which_rowid_a_name_reaches_through_the_tables_inside_brackets() {
    let image = keyed();
    let database = Database::open(&image).expect("a database");
    let answered = |sql: &[u8]| {
        database
            .query(sql)
            .unwrap_or_else(|error| panic!("{sql:?}: {}", error.message()))
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(shown)
                    .collect::<alloc::vec::Vec<_>>()
                    .join("|")
            })
            .collect::<alloc::vec::Vec<_>>()
            .join(" ")
    };
    // One table inside brackets keeps a rowid, so the name reaches it.
    assert_eq!(answered(b"SELECT _rowid_ FROM w1 JOIN (w3 JOIN r1)"), "101");
    assert_eq!(answered(b"SELECT oid FROM w2 JOIN (w3 JOIN r1)"), "101");
    // A name in front of the column names one of the tables inside
    // brackets, and a database in front of that names the one the side
    // reads.
    assert_eq!(answered(b"SELECT r1.oid FROM w1 JOIN (w3 JOIN r1)"), "101");
    // The name the rowid of a table inside brackets is answered under
    // reaches it as the rowid and not as a column of the side.
    assert_eq!(
        answered(b"SELECT r1._rowid_ FROM w1 JOIN (w3 JOIN r1)"),
        "101"
    );
    assert_eq!(answered(b"SELECT main.r1.rowid FROM r1"), "101");
    // A column called `rowid` is answered before the rowid of another
    // side.
    assert_eq!(
        answered(b"SELECT rowid FROM r1 JOIN (r2 JOIN w2)"),
        "myrowid"
    );
    // A table that carries a column of each of the three names answers
    // no rowid through the brackets, so the column answers.
    assert_eq!(answered(b"SELECT rowid FROM w1 JOIN (w3 JOIN r3)"), "7");
    // A `*` leaves the rowid of each table inside brackets out.
    assert_eq!(
        answered(b"SELECT * FROM r1 JOIN (r2 JOIN w2)"),
        "A|B|mypk|myrowid"
    );
    for (sql, message) in [
        (
            b"SELECT rowid FROM w1, (r1, r2)".as_slice(),
            "ambiguous column name: rowid",
        ),
        (
            b"SELECT rowid FROM r1, (r2 JOIN w1)",
            "ambiguous column name: rowid",
        ),
        (
            b"SELECT temp.r1.rowid FROM r1",
            "no such column: temp.r1.rowid",
        ),
        (
            b"SELECT oid FROM w1 JOIN (w3 JOIN w2 ON 1)",
            "no such column: oid",
        ),
    ] {
        assert_eq!(refused(&image, sql), message, "{sql:?}");
    }
}
