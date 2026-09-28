// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The names a statement that writes reads before it writes a row: its
//! own expressions and the triggers it may fire, against what the C
//! library's shell answers for the same statements.

use alloc::string::String;

use crate::change::Writer;
use crate::header::Encoding;
use crate::value::Value;

/// A connection over two tables, neither holding a row.
fn writing() -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [b"CREATE TABLE t1(a,b)".as_slice(), b"CREATE TABLE t2(c,d)"] {
        writer.run(sql).unwrap();
    }
    writer
}

/// What one statement of the connection is refused with.
fn refused(writer: &mut Writer, sql: &[u8]) -> String {
    writer.run(sql).expect_err("a refusal").message()
}

/// A `DELETE` and an `UPDATE` read the names of their expressions against
/// the table, so a name no column carries is refused although the table
/// holds no row to read it for.
#[test]
fn which_names_a_statement_over_no_row_reads() {
    let mut writer = writing();
    assert_eq!(
        refused(&mut writer, b"DELETE FROM t1 WHERE nosuch=5"),
        "no such column: nosuch"
    );
    assert_eq!(
        refused(&mut writer, b"UPDATE t1 SET a=1 WHERE nosuch=5"),
        "no such column: nosuch"
    );
    assert_eq!(
        refused(&mut writer, b"UPDATE t1 SET a=nosuch"),
        "no such column: nosuch"
    );
    // The key of the table stands under each of its three names, and a
    // name written under the table or under the name an `AS` gave it
    // reaches the same column.
    for sql in [
        b"DELETE FROM t1 WHERE rowid=5".as_slice(),
        b"DELETE FROM t1 AS held WHERE held.a=5",
        b"UPDATE t1 SET a=oid WHERE _rowid_>0",
        // An `UPDATE ... FROM` reads the columns of the tables its
        // `FROM` names as well, so its names are left to the walk.
        b"UPDATE t1 SET a=c FROM t2",
    ] {
        writer.run(sql).unwrap_or_else(|error| {
            panic!("{sql:?}: {}", error.message());
        });
    }
}

/// A statement reads the `WHEN` and the body of every trigger it may
/// fire, so a name neither the tables nor the two rows of the trigger
/// hold refuses the statement and not the row.
#[test]
fn which_names_the_triggers_a_statement_may_fire_read() {
    let mut writer = writing();
    writer
        .run(b"CREATE TRIGGER r BEFORE UPDATE ON t1 WHEN nosuch BEGIN SELECT 1; END")
        .unwrap();
    assert_eq!(
        refused(&mut writer, b"UPDATE t1 SET a=1"),
        "no such column: nosuch"
    );
    // A trigger over the other event is left alone, and so is a
    // statement that fires none.
    writer.run(b"DELETE FROM t1").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1,2)").unwrap();
    writer.run(b"DROP TRIGGER r").unwrap();
    // A step may read and write the key of the table under each of its
    // three names.
    writer
        .run(b"CREATE TRIGGER r AFTER INSERT ON t1 BEGIN UPDATE t2 SET rowid=8 WHERE oid=1; END")
        .unwrap();
    writer.run(b"INSERT INTO t1 VALUES(3,4)").unwrap();
    // A step that names a column no table of it holds refuses the
    // statement that fires it.
    writer.run(b"DROP TRIGGER r").unwrap();
    writer
        .run(b"CREATE TRIGGER r AFTER DELETE ON t1 BEGIN UPDATE t2 SET c=old.nosuch; END")
        .unwrap();
    assert_eq!(
        refused(&mut writer, b"DELETE FROM t1"),
        "no such column: old.nosuch"
    );
    // The bytes the schema is read out of are taken again where a
    // statement has changed the schema since the last one read them.
    writer.run(b"INSERT INTO t1 VALUES(7,8)").unwrap();
    writer.run(b"CREATE TABLE t4(k)").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(9,10)").unwrap();
    // A table that keeps its rows in the key's own tree has no key of
    // its own to name, so a step that writes one names no column.
    writer
        .run(b"CREATE TABLE t3(k PRIMARY KEY, v) WITHOUT ROWID")
        .unwrap();
    writer.run(b"DROP TRIGGER r").unwrap();
    writer
        .run(b"CREATE TRIGGER r AFTER INSERT ON t1 BEGIN UPDATE t3 SET rowid=1; END")
        .unwrap();
    assert_eq!(
        refused(&mut writer, b"INSERT INTO t1 VALUES(5,6)"),
        "no such column: rowid"
    );
}

/// The pass over a row of nulls resolves the names of a statement that
/// answered no row, and a refusal the value of that row raised stands for
/// no statement: a `RAISE` of a trigger's body no row reached raises
/// nothing, and a `zeroblob` longer than the largest value refuses
/// nothing.
#[test]
fn what_a_statement_that_answered_no_row_refuses() {
    let mut writer = writing();
    writer
        .run(
            b"CREATE TRIGGER r BEFORE INSERT ON t1 BEGIN \
              SELECT RAISE(IGNORE) WHERE EXISTS (SELECT a FROM t1 WHERE a=new.a); END",
        )
        .unwrap();
    // The `EXISTS` holds for no row of the empty table, so the body's
    // statement answers no row and the row is written.
    writer.run(b"INSERT INTO t1 VALUES(1,2)").unwrap();
    // The second row reaches the `EXISTS`, whose `RAISE` passes it over.
    writer.run(b"INSERT INTO t1 VALUES(1,3)").unwrap();
    let written = writer.written();
    let database = crate::db::Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"SELECT count(*) FROM t1").unwrap().rows,
        [[Value::Int(1)]]
    );
    assert!(
        database
            .query(b"SELECT zeroblob(2000000000) WHERE 0")
            .unwrap()
            .rows
            .is_empty()
    );
    // A name of such a statement is resolved all the same.
    assert_eq!(
        database
            .query(b"SELECT nosuch FROM t1 WHERE 0")
            .unwrap_err()
            .message(),
        "no such column: nosuch"
    );
}
