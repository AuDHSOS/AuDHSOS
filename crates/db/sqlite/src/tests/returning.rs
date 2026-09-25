// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `RETURNING` and `DEFAULT VALUES`, against what the shell answers for
//! the same statements.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// A connection over a table of a key, a column that falls back to 7,
/// and one that falls back to nothing.
fn writer() -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a INTEGER PRIMARY KEY, b DEFAULT 7, c)")
        .unwrap();
    writer
}

#[test]
fn a_statement_that_writes_no_values_writes_what_every_column_falls_back_to() {
    let mut writer = writer();
    assert_eq!(
        writer.run(b"INSERT INTO t DEFAULT VALUES").unwrap(),
        Vec::<Vec<Value>>::new()
    );
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    assert_eq!(
        database.rows_of(b"t").unwrap(),
        alloc::vec![(1, alloc::vec![Value::Int(1), Value::Int(7), Value::Null])]
    );
}

#[test]
fn every_statement_that_writes_answers_the_rows_its_returning_names() {
    let mut writer = writer();
    // `*` answers the columns of the table, and an expression beside it
    // answers what it computes over the row.
    assert_eq!(
        writer
            .run(b"INSERT INTO t(b) VALUES(1) RETURNING *, b+1")
            .unwrap(),
        alloc::vec![alloc::vec![
            Value::Int(1),
            Value::Int(1),
            Value::Null,
            Value::Int(2)
        ]]
    );
    writer.run(b"INSERT INTO t(b) VALUES(2)").unwrap();
    // An `UPDATE` answers one row per row it wrote, with the row as it
    // stands.
    assert_eq!(
        writer.run(b"UPDATE t SET c=3 RETURNING rowid, c").unwrap(),
        alloc::vec![
            alloc::vec![Value::Int(1), Value::Int(3)],
            alloc::vec![Value::Int(2), Value::Int(3)],
        ]
    );
    // A `DELETE` answers the row as it was.
    assert_eq!(
        writer
            .run(b"DELETE FROM t WHERE b=1 RETURNING b, c")
            .unwrap(),
        alloc::vec![alloc::vec![Value::Int(1), Value::Int(3)]]
    );
    // A statement that writes no row answers none.
    assert_eq!(
        writer.run(b"DELETE FROM t WHERE b=99 RETURNING b").unwrap(),
        Vec::<Vec<Value>>::new()
    );
}

#[test]
fn a_clause_that_writes_the_row_a_conflict_found_answers_it() {
    let mut writer = writer();
    writer.run(b"INSERT INTO t VALUES(1,2,3)").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,3,4) ON CONFLICT(a) DO UPDATE SET b=9 RETURNING *, rowid")
            .unwrap(),
        alloc::vec![alloc::vec![
            Value::Int(1),
            Value::Int(9),
            Value::Int(3),
            Value::Int(1)
        ]]
    );
    // A clause that does nothing writes no row, so it answers none.
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(1,3,4) ON CONFLICT(a) DO NOTHING RETURNING *")
            .unwrap(),
        Vec::<Vec<Value>>::new()
    );
}

#[test]
fn a_table_that_keeps_its_rows_in_the_key_s_own_tree_answers_them_too() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE k(a PRIMARY KEY, b) WITHOUT ROWID")
        .unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO k VALUES('x',1) RETURNING *")
            .unwrap(),
        alloc::vec![alloc::vec![Value::Text(b"x".to_vec()), Value::Int(1)]]
    );
    assert_eq!(
        writer.run(b"UPDATE k SET b=2 RETURNING b").unwrap(),
        alloc::vec![alloc::vec![Value::Int(2)]]
    );
    assert_eq!(
        writer.run(b"DELETE FROM k RETURNING b").unwrap(),
        alloc::vec![alloc::vec![Value::Int(2)]]
    );
}

#[test]
fn what_the_parser_reads_of_the_two() {
    let (arena, change) =
        crate::parse::change(b"INSERT INTO t DEFAULT VALUES RETURNING a, b AS x").unwrap();
    let crate::ast::Change::Insert(statement) = change else {
        panic!("an insert was written");
    };
    assert!(statement.defaults);
    assert_eq!(arena.results(statement.returning).len(), 2);
    let (arena, change) = crate::parse::change(b"DELETE FROM t RETURNING *").unwrap();
    let crate::ast::Change::Delete(statement) = change else {
        panic!("a delete was written");
    };
    assert_eq!(arena.results(statement.returning).len(), 1);
    for sql in [
        b"INSERT INTO t DEFAULT".as_slice(),
        b"INSERT INTO t DEFAULT VALUES RETURNING",
        b"UPDATE t SET a=1 RETURNING",
        b"DELETE FROM t RETURNING",
    ] {
        assert!(crate::parse::change(sql).is_err(), "{sql:?}");
    }
}

/// The names a `RETURNING` answers its columns under, which a caller
/// reads out of the statement without running it, and the `TABLE.*` the
/// clause takes no name in front of.
#[test]
fn what_names_a_returning_answers_its_columns_under() {
    let mut writer = writer();
    writer
        .run(b"CREATE TABLE u(x, y AS (x+1), z AS (x+2) STORED)")
        .unwrap();
    let named = |sql: &[u8]| -> alloc::vec::Vec<alloc::string::String> {
        writer
            .returning_names(sql)
            .iter()
            .map(|name| alloc::string::String::from_utf8_lossy(name).into_owned())
            .collect()
    };
    // A name an `AS` wrote, the name of a column, and the text of every
    // other expression.
    assert_eq!(
        named(b"INSERT INTO t(a) VALUES(1) RETURNING b AS [xyz]"),
        ["xyz"]
    );
    assert_eq!(named(b"INSERT INTO t(a) VALUES(1) RETURNING \"b\""), ["b"]);
    assert_eq!(
        named(b"UPDATE t SET b=1 RETURNING \"b\"+\"c\""),
        ["\"b\"+\"c\""]
    );
    // A `*` answers every column of the table that the row holds, so a
    // column the row computes where it is read is no column of it.
    assert_eq!(named(b"DELETE FROM t RETURNING *"), ["a", "b", "c"]);
    assert_eq!(named(b"DELETE FROM u RETURNING *"), ["x", "z"]);
    // A statement that holds no `RETURNING`, one the parser does not
    // take, and one over a table the schema does not hold each answer no
    // name.
    assert!(named(b"DELETE FROM t").is_empty());
    assert!(named(b"SELECT 1").is_empty());
    assert!(named(b"DELETE FROM nope RETURNING *").is_empty());
    // `sqlite3AddReturning` has one table to answer, so the clause takes
    // no name in front of a star.
    assert_eq!(
        writer
            .run(b"INSERT INTO t(a) VALUES(1) RETURNING t.*")
            .unwrap_err()
            .message(),
        "RETURNING may not use \"TABLE.*\" wildcards"
    );
}

/// A statement written inside a `RETURNING`, which is read against the
/// file as the statement that holds it found the file.
#[test]
fn what_a_statement_written_inside_a_returning_answers() {
    let mut writer = writer();
    writer.run(b"CREATE TABLE u(b,c)").unwrap();
    writer.run(b"INSERT INTO u VALUES(3,40)").unwrap();
    // The clause reads the table it answers under its own name, so a
    // statement inside it names the row the statement wrote.
    assert_eq!(
        writer
            .run(b"INSERT INTO t(a) VALUES(3) RETURNING a, (SELECT c FROM u WHERE t.a=u.b)")
            .unwrap(),
        alloc::vec![alloc::vec![Value::Int(3), Value::Int(40)]]
    );
    assert_eq!(
        writer
            .run(b"INSERT INTO t(a) VALUES(4) RETURNING EXISTS(SELECT 1 FROM u WHERE u.b=t.a)")
            .unwrap(),
        alloc::vec![alloc::vec![Value::Int(0)]]
    );
    // `new` and `old` are names of a trigger's row and no name of a
    // clause, whatever the statement wrote.
    for sql in [
        b"INSERT INTO t(a) VALUES(5) RETURNING (SELECT c FROM u WHERE new.a=u.b)".as_slice(),
        b"INSERT INTO t(a) VALUES(5) RETURNING (SELECT c FROM u WHERE old.a=u.b)",
    ] {
        let message = writer.run(sql).unwrap_err().message();
        assert!(message.starts_with("no such column: "), "{message}");
    }
}
