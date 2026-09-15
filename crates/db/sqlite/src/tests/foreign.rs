// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the foreign keys a table's rows are held to, against what
//! the C library answers for the same statements over the same rows.

use alloc::string::String;

use crate::change::Writer;
use crate::db::{Database, Error};
use crate::header::Encoding;
use crate::value::Value;

/// A connection that has run the statements, and what the last of them
/// answered.
fn ran(statements: &[&str]) -> Result<(Writer, alloc::vec::Vec<String>), Error> {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    let mut out = alloc::vec::Vec::new();
    for sql in statements {
        out.clear();
        for row in writer.run(sql.as_bytes())? {
            for value in &row {
                out.push(shown(value));
            }
        }
    }
    Ok((writer, out))
}

/// What one value looks like as a list element.
fn shown(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Int(number) => alloc::format!("{number}"),
        Value::Real(number) => String::from_utf8_lossy(&crate::fp::text(*number, 15)).into_owned(),
        Value::Text(bytes) | Value::Blob(bytes) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// What a statement answers over a connection that has run the ones
/// before it.
fn answered(writer: &Writer, sql: &str) -> alloc::vec::Vec<String> {
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    let mut out = alloc::vec::Vec::new();
    for row in &database.query(sql.as_bytes()).unwrap().rows {
        for value in row {
            out.push(shown(value));
        }
    }
    out
}

/// What a pragma answers, which the connection that writes answers out
/// of what it holds.
fn pragma(writer: &mut Writer, sql: &str) -> alloc::vec::Vec<String> {
    let mut out = alloc::vec::Vec::new();
    for row in writer.run(sql.as_bytes()).unwrap() {
        for value in &row {
            out.push(shown(value));
        }
    }
    out
}

/// The tables the tests of the parent side are written over.
const PARENTS: &[&str] = &[
    "PRAGMA foreign_keys=ON",
    "CREATE TABLE p(x INTEGER PRIMARY KEY, y)",
    "CREATE TABLE gone(a, b REFERENCES p ON DELETE CASCADE)",
    "CREATE TABLE emptied(a, b REFERENCES p ON DELETE SET NULL)",
    "CREATE TABLE fallen(a, b DEFAULT 7 REFERENCES p ON DELETE SET DEFAULT)",
    "CREATE TABLE held(a, b REFERENCES p)",
    "INSERT INTO p VALUES(1,'one'),(2,'two'),(3,'three'),(4,'four')",
    "INSERT INTO gone VALUES('a',1)",
    "INSERT INTO emptied VALUES('b',2)",
    "INSERT INTO fallen VALUES('c',3)",
    "INSERT INTO held VALUES('d',4)",
];

#[test]
fn a_row_whose_foreign_key_points_at_no_row_is_refused() {
    // `I.1` of `src/fkey.c`, which `PRAGMA foreign_keys` turns on and
    // which a connection told nothing leaves off.
    let (writer, _) = ran(&[
        "CREATE TABLE p(x PRIMARY KEY, y)",
        "CREATE TABLE c(a, b REFERENCES p(x))",
        "INSERT INTO p VALUES(1,'one')",
        "INSERT INTO c VALUES('a',9)",
    ])
    .unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM c"), ["1"]);
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE p(x PRIMARY KEY, y)",
        "CREATE TABLE c(a, b REFERENCES p(x))",
        "INSERT INTO p VALUES(1,'one')",
    ])
    .unwrap();
    assert_eq!(
        writer.run(b"INSERT INTO c VALUES('a',9)"),
        Err(Error::Foreign)
    );
    // A key with a null in it points at nothing, which `MATCH SIMPLE`
    // holds to nothing.
    writer.run(b"INSERT INTO c VALUES('a',NULL)").unwrap();
    writer.run(b"INSERT INTO c VALUES('b',1)").unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM c"), ["2"]);
    // An `UPDATE` is held to the same rule.
    assert_eq!(
        writer.run(b"UPDATE c SET b=9 WHERE a='b'"),
        Err(Error::Foreign)
    );
    writer.run(b"UPDATE c SET b=1 WHERE a='a'").unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM c WHERE b=1"), ["2"]);
}

#[test]
fn a_foreign_key_that_points_at_no_unique_columns_is_a_mismatch() {
    // `sqlite3FkLocateIndex`: the columns pointed at are the primary
    // key of that table or carry a unique index of their own.
    for sql in [
        // No unique columns at all.
        "INSERT INTO loose VALUES(1)",
        // A name the table pointed at does not carry.
        "INSERT INTO astray VALUES(1)",
        // More columns than the key it points at.
        "INSERT INTO wide VALUES(1,2)",
        // A table that is not there.
        "INSERT INTO nowhere VALUES(1)",
        // A table with no primary key at all.
        "INSERT INTO keyless VALUES(1)",
        // More columns pointed at than columns that point.
        "INSERT INTO uneven VALUES(1)",
        // As many columns as the primary key carries, under another
        // name, which no unique index covers.
        "INSERT INTO sideways VALUES(1)",
    ] {
        let (mut writer, _) = ran(&[
            "PRAGMA foreign_keys=ON",
            "CREATE TABLE plain(x, y)",
            "CREATE TABLE keyed(x PRIMARY KEY, y)",
            "CREATE TABLE loose(a REFERENCES plain(x))",
            "CREATE TABLE astray(a REFERENCES keyed(z))",
            "CREATE TABLE wide(a, b, FOREIGN KEY(a,b) REFERENCES keyed)",
            "CREATE TABLE nowhere(a REFERENCES missing(x))",
            "CREATE TABLE keyless(a REFERENCES plain)",
            "CREATE TABLE uneven(a, FOREIGN KEY(a) REFERENCES keyed(x, y))",
            "CREATE TABLE sideways(a REFERENCES keyed(y))",
        ])
        .unwrap();
        assert_eq!(
            writer.run(sql.as_bytes()),
            Err(Error::ForeignMismatch),
            "{sql}"
        );
    }
    // A `UNIQUE` over the columns pointed at is enough, and so is a
    // table whose primary key is the rowid.
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE u(x, y, UNIQUE(x,y))",
        "CREATE TABLE r(x INTEGER PRIMARY KEY)",
        "CREATE TABLE two(a, b, FOREIGN KEY(a,b) REFERENCES u(x,y))",
        "CREATE TABLE one(a REFERENCES r)",
        "INSERT INTO u VALUES(1,2)",
        "INSERT INTO r VALUES(5)",
        "INSERT INTO two VALUES(1,2)",
        "INSERT INTO one VALUES(5)",
    ])
    .unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM two"), ["1"]);
    assert_eq!(
        writer.run(b"INSERT INTO two VALUES(1,3)"),
        Err(Error::Foreign)
    );
    assert_eq!(
        writer.run(b"INSERT INTO one VALUES(6)"),
        Err(Error::Foreign)
    );
    // A key that names no columns points at the primary key in the
    // order the primary key was written, not the order of the table.
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE both(x, y, PRIMARY KEY(y,x))",
        "CREATE TABLE pair(a, b, FOREIGN KEY(a,b) REFERENCES both)",
        "INSERT INTO both VALUES(1,2)",
        "INSERT INTO pair VALUES(2,1)",
    ])
    .unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM pair"), ["1"]);
    assert_eq!(
        writer.run(b"INSERT INTO pair VALUES(1,2)"),
        Err(Error::Foreign)
    );
}

#[test]
fn what_happens_to_the_rows_that_point_at_a_row_that_goes() {
    // `D.2` of `src/fkey.c`, one action per table.
    let (mut writer, _) = ran(PARENTS).unwrap();
    writer.run(b"DELETE FROM p WHERE x=1").unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM gone"), ["0"]);
    writer.run(b"DELETE FROM p WHERE x=2").unwrap();
    assert_eq!(answered(&writer, "SELECT a, b FROM emptied"), ["b", ""]);
    writer.run(b"DELETE FROM p WHERE x=3").unwrap();
    assert_eq!(answered(&writer, "SELECT a, b FROM fallen"), ["c", "7"]);
    assert_eq!(writer.run(b"DELETE FROM p WHERE x=4"), Err(Error::Foreign));
    assert_eq!(answered(&writer, "SELECT count(*) FROM p"), ["1"]);
    // A row nothing points at goes, and so does a row whose key is
    // null wherever it is read.
    writer.run(b"INSERT INTO p VALUES(5,'five')").unwrap();
    writer.run(b"DELETE FROM p WHERE x=5").unwrap();
    writer.run(b"INSERT INTO held VALUES('e',NULL)").unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM held"), ["2"]);
}

#[test]
fn what_happens_to_the_rows_that_point_at_a_row_that_changes() {
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE p(x PRIMARY KEY, y)",
        "CREATE TABLE moved(a, b REFERENCES p ON UPDATE CASCADE)",
        "CREATE TABLE emptied(a, b REFERENCES p ON UPDATE SET NULL)",
        "CREATE TABLE held(a, b REFERENCES p)",
        "INSERT INTO p VALUES(1,'one'),(2,'two'),(3,'three')",
        "INSERT INTO moved VALUES('a',1)",
        "INSERT INTO emptied VALUES('b',2)",
        "INSERT INTO held VALUES('c',3)",
    ])
    .unwrap();
    writer.run(b"UPDATE p SET x=11 WHERE x=1").unwrap();
    assert_eq!(answered(&writer, "SELECT b FROM moved"), ["11"]);
    writer.run(b"UPDATE p SET x=12 WHERE x=2").unwrap();
    assert_eq!(answered(&writer, "SELECT b FROM emptied"), [""]);
    assert_eq!(
        writer.run(b"UPDATE p SET x=13 WHERE x=3"),
        Err(Error::Foreign)
    );
    // An `UPDATE` that leaves the columns pointed at alone leaves the
    // rows that point alone.
    writer.run(b"UPDATE p SET y='III' WHERE x=3").unwrap();
    assert_eq!(answered(&writer, "SELECT b FROM held"), ["3"]);
}

#[test]
fn a_row_the_key_of_which_is_null_is_pointed_at_by_nothing() {
    // The row of the table pointed at has nothing in the column that is
    // pointed at, so no row points at it and it goes alone.
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE u(x UNIQUE, y)",
        "CREATE TABLE c(a, b REFERENCES u(x) ON DELETE CASCADE)",
        "INSERT INTO u VALUES(NULL,'n'),(1,'one')",
        "INSERT INTO c VALUES('a',1)",
    ])
    .unwrap();
    writer.run(b"DELETE FROM u WHERE y='n'").unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM c"), ["1"]);
}

#[test]
fn a_row_that_points_and_keeps_its_rows_in_the_key_of_its_own_is_written_again() {
    // The column the key of the table that points is another name for
    // takes no place in the record, so the row is written again with
    // the key left out of it.
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE p(x PRIMARY KEY)",
        "CREATE TABLE k(id INTEGER PRIMARY KEY, b REFERENCES p(x) ON UPDATE CASCADE)",
        "INSERT INTO p VALUES(1)",
        "INSERT INTO k VALUES(7,1)",
    ])
    .unwrap();
    writer.run(b"UPDATE p SET x=2").unwrap();
    assert_eq!(answered(&writer, "SELECT id, b FROM k"), ["7", "2"]);
}

#[test]
fn a_cascade_reaches_the_rows_that_point_at_the_rows_it_takes() {
    let (mut writer, _) = ran(&[
        "PRAGMA foreign_keys=ON",
        "CREATE TABLE a(x PRIMARY KEY)",
        "CREATE TABLE b(x PRIMARY KEY, y REFERENCES a ON DELETE CASCADE)",
        "CREATE TABLE c(x, y REFERENCES b ON DELETE CASCADE)",
        "INSERT INTO a VALUES(1)",
        "INSERT INTO b VALUES(2,1)",
        "INSERT INTO c VALUES(3,2)",
    ])
    .unwrap();
    writer.run(b"DELETE FROM a").unwrap();
    assert_eq!(answered(&writer, "SELECT count(*) FROM b"), ["0"]);
    assert_eq!(answered(&writer, "SELECT count(*) FROM c"), ["0"]);
}

#[test]
fn the_foreign_keys_a_table_carries_are_answered_newest_first() {
    let (mut writer, _) = ran(&[
        "CREATE TABLE p(x PRIMARY KEY, y)",
        "CREATE TABLE two(u, v, PRIMARY KEY(u,v))",
        "CREATE TABLE c(a, b REFERENCES p(x) ON DELETE CASCADE, d REFERENCES p)",
        "CREATE TABLE wide(a, b, FOREIGN KEY(a,b) REFERENCES two)",
    ])
    .unwrap();
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_list(c)"),
        [
            "0",
            "0",
            "p",
            "d",
            "",
            "NO ACTION",
            "NO ACTION",
            "NONE",
            "1",
            "0",
            "p",
            "b",
            "x",
            "NO ACTION",
            "CASCADE",
            "NONE"
        ]
    );
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_list(wide)"),
        [
            "0",
            "0",
            "two",
            "a",
            "",
            "NO ACTION",
            "NO ACTION",
            "NONE",
            "0",
            "1",
            "two",
            "b",
            "",
            "NO ACTION",
            "NO ACTION",
            "NONE"
        ]
    );
    // The three actions the list writes out beside the two it writes
    // as `NO ACTION`.
    let (mut writer, _) = ran(&[
        "CREATE TABLE p(x PRIMARY KEY)",
        "CREATE TABLE ways(a REFERENCES p ON DELETE SET NULL ON UPDATE SET DEFAULT, \
         b REFERENCES p ON DELETE RESTRICT)",
    ])
    .unwrap();
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_list(ways)"),
        [
            "0",
            "0",
            "p",
            "b",
            "",
            "NO ACTION",
            "RESTRICT",
            "NONE",
            "1",
            "0",
            "p",
            "a",
            "",
            "SET DEFAULT",
            "SET NULL",
            "NONE"
        ]
    );
    let (mut writer, _) = ran(&[
        "CREATE TABLE p(x PRIMARY KEY, y)",
        "CREATE TABLE two(u, v, PRIMARY KEY(u,v))",
        "CREATE TABLE c(a, b REFERENCES p(x) ON DELETE CASCADE, d REFERENCES p)",
    ])
    .unwrap();
    // A table with no foreign key, and a name no table carries, answer
    // no row.
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_list(p)"),
        Vec::<String>::new()
    );
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_list(nowhere)"),
        Vec::<String>::new()
    );
}

#[test]
fn the_rows_that_point_at_no_row_are_answered_whatever_the_pragma_says() {
    let (mut writer, _) = ran(&[
        "CREATE TABLE p(x PRIMARY KEY, y)",
        "CREATE TABLE c(a, b REFERENCES p(x))",
        "CREATE TABLE d(a, b REFERENCES missing(x))",
        "INSERT INTO p VALUES(1,'one')",
        "INSERT INTO c VALUES('a',1),('b',9),('c',NULL)",
    ])
    .unwrap();
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_check"),
        ["c", "2", "p", "0"]
    );
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_check(c)"),
        ["c", "2", "p", "0"]
    );
    assert_eq!(
        pragma(&mut writer, "PRAGMA foreign_key_check(p)"),
        Vec::<String>::new()
    );
}

#[test]
fn the_text_a_refusal_is_written_as() {
    assert_eq!(Error::Foreign.message(), "FOREIGN KEY constraint failed");
    assert_eq!(Error::ForeignMismatch.message(), "foreign key mismatch");
    assert_eq!(Error::Unique.message(), "UNIQUE constraint failed");
    assert_eq!(
        Error::Nested.message(),
        "cannot start a transaction within a transaction"
    );
    assert_eq!(
        Error::NoTransaction.message(),
        "cannot commit - no transaction is active"
    );
    assert_eq!(
        Error::Recursion.message(),
        "recursive aggregate queries not supported"
    );
    // A refusal this crate has no text for is written as its name.
    assert_eq!(Error::NoTable.message(), "NoTable");
}
