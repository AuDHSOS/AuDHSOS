// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What a `CREATE INDEX` and a `DROP INDEX` are refused with, and what
//! an entry holds for a value a column converts.

use alloc::string::String;

use crate::change::Writer;
use crate::header::Encoding;
use crate::value::Value;

/// A connection over a table, an index, a view and a table with a
/// `UNIQUE` constraint.
fn writing() -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(a int, b int)".as_slice(),
        b"CREATE INDEX i1 ON t1(a)",
        b"CREATE VIEW v1 AS SELECT * FROM t1",
        b"CREATE TABLE t2(a UNIQUE)",
    ] {
        writer.run(sql).unwrap();
    }
    writer
}

/// What a statement is refused with.
fn refused(writer: &mut Writer, sql: &[u8]) -> String {
    writer.run(sql).unwrap_err().message()
}

/// A `CREATE INDEX` over a table SQLite keeps for itself, over a view,
/// over a table that is not there, and over a column the table does not
/// hold.
#[test]
fn what_a_create_index_is_refused_with() {
    let mut writer = writing();
    for (sql, message) in [
        (
            b"CREATE INDEX i2 ON sqlite_master(name)".as_slice(),
            "table sqlite_master may not be indexed",
        ),
        (b"CREATE INDEX i2 ON v1(a)", "views may not be indexed"),
        (
            b"CREATE INDEX i2 ON nosuch(a)",
            "no such table: main.nosuch",
        ),
        (b"CREATE INDEX i2 ON t1(nosuch)", "no such column: nosuch"),
        // `sqlite3CreateIndex` writes the name with its quotes taken
        // off, where `sqlite3StartTable` keeps them.
        (b"CREATE INDEX [i1] ON t1(b)", "index i1 already exists"),
        (b"CREATE TABLE [t1](c)", "table [t1] already exists"),
    ] {
        assert_eq!(refused(&mut writer, sql), message, "{sql:?}");
    }
}

/// A `DROP TABLE` over a name SQLite keeps for itself, and a key that
/// counts up where no key of the table holds it.
#[test]
fn what_a_name_sqlite_keeps_and_a_key_that_counts_up_are_refused_with() {
    let mut writer = writing();
    for (sql, message) in [
        (
            b"DROP TABLE sqlite_master".as_slice(),
            "table sqlite_master may not be dropped",
        ),
        (
            b"DROP VIEW sqlite_master",
            "table sqlite_master may not be dropped",
        ),
        (
            b"CREATE TABLE t9(a TEXT PRIMARY KEY AUTOINCREMENT)",
            "AUTOINCREMENT is only allowed on an INTEGER PRIMARY KEY",
        ),
        (
            b"CREATE TABLE t9(a INTEGER PRIMARY KEY AUTOINCREMENT, b) WITHOUT ROWID",
            "AUTOINCREMENT not allowed on WITHOUT ROWID tables",
        ),
        (b"CREATE TABLE t9(a, a)", "duplicate column name: a"),
        (
            b"CREATE TABLE t9(a PRIMARY KEY, b PRIMARY KEY)",
            "table \"t9\" has more than one primary key",
        ),
        (
            b"CREATE TABLE t9(a, b) WITHOUT ROWID",
            "PRIMARY KEY missing on table t9",
        ),
        (
            b"CREATE TABLE t9(a, PRIMARY KEY(nosuch))",
            "no such column: nosuch",
        ),
        (
            b"CREATE TABLE t9(a, UNIQUE(nosuch))",
            "no such column: nosuch",
        ),
        (
            b"CREATE TABLE t9(a, PRIMARY KEY(a+1))",
            "expressions prohibited in PRIMARY KEY and UNIQUE constraints",
        ),
    ] {
        assert_eq!(refused(&mut writer, sql), message, "{sql:?}");
    }
}

/// A `DROP INDEX` of an index that is not there and of one a `UNIQUE`
/// made.
#[test]
fn what_a_drop_index_is_refused_with() {
    let mut writer = writing();
    assert_eq!(
        refused(&mut writer, b"DROP INDEX nosuch"),
        "no such index: nosuch"
    );
    assert_eq!(
        refused(&mut writer, b"DROP INDEX sqlite_autoindex_t2_1"),
        "index associated with UNIQUE or PRIMARY KEY constraint cannot be dropped"
    );
    writer.run(b"DROP INDEX IF EXISTS nosuch").unwrap();
    writer.run(b"DROP INDEX i1").unwrap();
}

/// An entry holds the value the row holds and not the one the statement
/// wrote, so a column that converts what it is given leaves the entry
/// where the row is.
#[test]
fn what_an_entry_holds_for_a_value_the_column_converts() {
    let mut writer = writing();
    writer.run(b"INSERT INTO t1 VALUES('1.234e5',1)").unwrap();
    writer.run(b"INSERT INTO t1 VALUES('12.34e',4)").unwrap();
    let image = writer.written();
    let database = crate::db::Database::open(&image).expect("a database");
    let checked = database.query(b"PRAGMA integrity_check").expect("a check");
    assert_eq!(
        checked.rows,
        alloc::vec![alloc::vec![Value::Text(b"ok".to_vec())]]
    );
    let found = database
        .query(b"SELECT b FROM t1 WHERE a=123400")
        .expect("an answer");
    assert_eq!(found.rows, alloc::vec![alloc::vec![Value::Int(1)]]);
}

/// A constraint that says `ROLLBACK` undoes the transaction the
/// statement runs in and ends it, where `ABORT` leaves it open.
#[test]
fn what_a_constraint_that_says_rollback_undoes() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t7(a UNIQUE PRIMARY KEY)".as_slice(),
        b"CREATE TABLE t8(a UNIQUE PRIMARY KEY ON CONFLICT ROLLBACK)",
        b"INSERT INTO t7 VALUES(1)",
        b"INSERT INTO t8 VALUES(1)",
    ] {
        writer.run(sql).unwrap();
    }
    writer.run(b"BEGIN").unwrap();
    assert_eq!(
        refused(&mut writer, b"INSERT INTO t7 VALUES(1)"),
        "UNIQUE constraint failed: t7.a"
    );
    // The transaction the `ABORT` left open.
    assert_eq!(
        refused(&mut writer, b"BEGIN"),
        "cannot start a transaction within a transaction"
    );
    writer.run(b"INSERT INTO t7 VALUES(2)").unwrap();
    assert_eq!(
        refused(&mut writer, b"INSERT INTO t8 VALUES(1)"),
        "UNIQUE constraint failed: t8.a"
    );
    // The `ROLLBACK` clause ended the transaction and took the row
    // written inside it with it.
    writer.run(b"BEGIN").unwrap();
    writer.run(b"COMMIT").unwrap();
    let image = writer.written();
    let database = crate::db::Database::open(&image).expect("a database");
    let rows = database.query(b"SELECT a FROM t7").expect("an answer").rows;
    assert_eq!(rows, alloc::vec![alloc::vec![Value::Int(1)]]);
}

/// `INDEXED BY name` names an index the table must hold, and `NOT
/// INDEXED` names none, which a `SELECT`, an `UPDATE` and a `DELETE`
/// each carry and a trigger's body carries neither of.
#[test]
fn what_index_a_statement_names() {
    let mut writer = writing();
    writer.run(b"CREATE TABLE t3(c)").unwrap();
    writer.run(b"CREATE INDEX i3 ON t3(c)").unwrap();
    // An index the table holds and no index at all are both written.
    for sql in [
        b"UPDATE t1 INDEXED BY i1 SET b = 1 WHERE a = 1".as_slice(),
        b"UPDATE t1 NOT INDEXED SET b = 1 WHERE a = 1",
        b"DELETE FROM t1 INDEXED BY i1 WHERE a = 1",
        b"DELETE FROM t1 NOT INDEXED WHERE a = 1",
    ] {
        writer
            .run(sql)
            .unwrap_or_else(|error| panic!("{sql:?}: {error:?}"));
    }
    // A reader is held to the same names.
    let bytes = writer.written();
    let database = crate::db::Database::open(&bytes).unwrap();
    let read = |sql: &[u8]| database.query(sql).map_err(|error| error.message());
    assert!(read(b"SELECT * FROM t1 NOT INDEXED WHERE a = 1").is_ok());
    assert!(read(b"SELECT * FROM t1 INDEXED BY i1 WHERE a = 1").is_ok());
    assert_eq!(
        read(b"SELECT * FROM t1 INDEXED BY i3 WHERE a = 1").err(),
        Some(String::from("no such index: i3"))
    );
    assert_eq!(
        read(b"SELECT * FROM t1 INDEXED BY i9 WHERE a = 1").err(),
        Some(String::from("no such index: i9"))
    );
    // An index of another table is an index the table does not hold.
    for (sql, message) in [
        (
            b"UPDATE t1 INDEXED BY i3 SET b = 1 WHERE a = 1".as_slice(),
            "no such index: i3",
        ),
        (b"DELETE FROM t1 INDEXED BY i9 WHERE a = 1", "no such index: i9"),
        (
            b"CREATE TRIGGER r AFTER INSERT ON t3 BEGIN               UPDATE t1 INDEXED BY i1 SET b = 1; END",
            "the INDEXED BY clause is not allowed on UPDATE or DELETE statements within triggers",
        ),
        (
            b"CREATE TRIGGER r AFTER INSERT ON t3 BEGIN               DELETE FROM t1 NOT INDEXED; END",
            "the NOT INDEXED clause is not allowed on UPDATE or DELETE statements within triggers",
        ),
    ] {
        assert_eq!(refused(&mut writer, sql), message, "{sql:?}");
    }
}

/// `BEGIN`, `COMMIT` and `ROLLBACK` take a name after the word
/// `TRANSACTION`, which names nothing.
#[test]
fn a_transaction_takes_a_name_that_names_nothing() {
    let mut writer = writing();
    for sql in [
        b"BEGIN TRANSACTION 'one'".as_slice(),
        b"ROLLBACK TRANSACTION 'one'",
        b"BEGIN TRANSACTION two",
        b"COMMIT TRANSACTION two",
    ] {
        writer
            .run(sql)
            .unwrap_or_else(|error| panic!("{sql:?}: {error:?}"));
    }
    // A savepoint the name of a transaction stands in front of is still
    // read as one.
    writer.run(b"BEGIN").unwrap();
    writer.run(b"SAVEPOINT one").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(1, 1)").unwrap();
    writer.run(b"ROLLBACK TRANSACTION TO one").unwrap();
    writer.run(b"COMMIT").unwrap();
    let bytes = writer.written();
    let database = crate::db::Database::open(&bytes).unwrap();
    assert_eq!(database.rows_of(b"t1"), Ok(alloc::vec::Vec::new()));
}

/// The first value of every entry of the index `name`, in the order the
/// tree holds them.
fn entries<'a>(bytes: &'a [u8], name: &[u8]) -> alloc::vec::Vec<crate::record::Value<'a>> {
    let image = crate::image::Image::open(bytes).unwrap();
    let mut root = 0;
    for row in image.schema() {
        let record = row.unwrap().record().unwrap();
        if record.value(1).unwrap() == Some(crate::record::Value::Text(name))
            && let Some(crate::record::Value::Int(held)) = record.value(3).unwrap()
        {
            root = u32::try_from(held).unwrap();
        }
    }
    image
        .entries(root)
        .map(|entry| {
            let payload = entry.unwrap();
            crate::record::Record::parse(payload.local)
                .unwrap()
                .value(0)
                .unwrap()
                .unwrap()
        })
        .collect()
}

/// An index written `DESC` holds its entries from the largest value
/// down, which `sqlite3CreateIndex` honors on a file of schema format 4.
#[test]
fn an_index_written_desc_holds_its_entries_backwards() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a)".as_slice(),
        b"CREATE INDEX ta ON t(a)",
        b"CREATE INDEX tb ON t(a DESC)",
        b"INSERT INTO t VALUES(2),(1),(3)",
    ] {
        writer.run(sql).unwrap();
    }
    let bytes = writer.written();
    let held = |number: i64| crate::record::Value::Int(number);
    assert_eq!(entries(&bytes, b"ta"), [held(1), held(2), held(3)]);
    assert_eq!(entries(&bytes, b"tb"), [held(3), held(2), held(1)]);
}

/// A key of an index stops at the first place held the other way round
/// from the place before it, because the entries after that place run
/// in another order than the terms name.
#[test]
fn a_key_stops_at_a_place_held_the_other_way() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a,b)".as_slice(),
        b"CREATE INDEX tab ON t(a, b DESC)",
        b"INSERT INTO t VALUES(1,1),(1,2),(2,1)",
    ] {
        writer.run(sql).unwrap();
    }
    let bytes = writer.written();
    let database = crate::db::Database::open(&bytes).unwrap();
    let rows = database
        .query(b"SELECT rowid FROM t WHERE a=1 AND b=2")
        .unwrap();
    assert_eq!(rows.rows, alloc::vec![alloc::vec![Value::Int(2)]]);
}

/// An integrity check of a file whose schema format is below 4 reads
/// every place of an index forwards, because `sqlite3CreateIndex`
/// ignores `DESC` there.
#[test]
fn an_index_written_desc_on_an_old_format_holds_its_entries_forwards() {
    let database = crate::db::Database::open(crate::tests::FORMAT1).unwrap();
    let answer = database.query(b"PRAGMA integrity_check").unwrap();
    assert_eq!(answer.rows, [alloc::vec![Value::Text(b"ok".to_vec())]]);
}

/// A walk of a covering index answers the values the table holds: a
/// place that holds what an expression answers stands for no column,
/// a column of real affinity holds a whole number as a whole number,
/// and an entry longer than a page runs onto an overflow page.
#[test]
fn a_covering_index_answers_the_values_the_table_holds() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    let long = alloc::string::String::from_utf8(alloc::vec![b'x'; 4000]).unwrap();
    for sql in [
        b"CREATE TABLE t(a, b REAL, c)".as_slice(),
        b"CREATE INDEX tab ON t(a, b, abs(c))",
        b"INSERT INTO t VALUES(1,2,3),(2,4.5,6)",
        b"CREATE TABLE u(x)",
        b"CREATE INDEX ux ON u(x)",
        alloc::format!("INSERT INTO u VALUES('{long}')").as_bytes(),
    ] {
        writer.run(sql).unwrap();
    }
    let bytes = writer.written();
    let database = crate::db::Database::open(&bytes).unwrap();
    assert_eq!(
        database
            .query(b"SELECT a, b FROM t WHERE a=1")
            .unwrap()
            .rows,
        [alloc::vec![Value::Int(1), Value::Real(2.0)]]
    );
    assert_eq!(
        database
            .query(b"SELECT a, b FROM t ORDER BY a")
            .unwrap()
            .rows,
        [
            alloc::vec![Value::Int(1), Value::Real(2.0)],
            alloc::vec![Value::Int(2), Value::Real(4.5)]
        ]
    );
    let answer = database.query(b"SELECT x FROM u ORDER BY x").unwrap();
    assert_eq!(answer.rows, [alloc::vec![Value::Text(long.into_bytes())]]);
}

/// The column the rowid is another name for is answered out of the
/// rowid every entry ends with, so an index that holds it nowhere
/// covers a statement that reads it.
#[test]
fn a_covering_index_answers_the_column_the_rowid_names() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE k(i INTEGER PRIMARY KEY, a)".as_slice(),
        b"CREATE INDEX ka ON k(a)",
        b"INSERT INTO k VALUES(7,'z'),(8,'y')",
    ] {
        writer.run(sql).unwrap();
    }
    let bytes = writer.written();
    let database = crate::db::Database::open(&bytes).unwrap();
    assert_eq!(
        database
            .query(b"SELECT i, a FROM k WHERE a='z'")
            .unwrap()
            .rows,
        [alloc::vec![Value::Int(7), Value::Text(b"z".to_vec())]]
    );
}

/// The index whose key the terms name the most columns of answers the
/// fewest rows, and two indexes the terms name as many columns of are
/// taken in the order `pTab->pIndex` carries them, which is the index
/// made last first.
#[test]
fn the_index_the_terms_name_the_most_columns_of_is_taken() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a,b,c)".as_slice(),
        b"CREATE INDEX tab ON t(a,b)",
        b"CREATE INDEX ta ON t(a)",
        b"INSERT INTO t VALUES(1,2,3),(1,3,4),(2,2,5)",
    ] {
        writer.run(sql).unwrap();
    }
    let bytes = writer.written();
    let database = crate::db::Database::open(&bytes).unwrap();
    let plan = |sql: &[u8]| {
        database
            .query(sql)
            .unwrap()
            .rows
            .iter()
            .filter_map(|row| match row.get(3) {
                Some(Value::Text(text)) => Some(String::from_utf8_lossy(text).into_owned()),
                _ => None,
            })
            .collect::<alloc::vec::Vec<_>>()
            .join("|")
    };
    assert_eq!(
        plan(b"EXPLAIN QUERY PLAN SELECT c FROM t WHERE a=1 AND b=2"),
        "SEARCH t USING INDEX tab (a=? AND b=?)"
    );
    assert_eq!(
        plan(b"EXPLAIN QUERY PLAN SELECT c FROM t WHERE a=1"),
        "SEARCH t USING INDEX ta (a=?)"
    );
    assert_eq!(
        database
            .query(b"SELECT c FROM t WHERE a=1 AND b=2")
            .unwrap()
            .rows,
        [alloc::vec![Value::Int(3)]]
    );
}

/// A term of a `CREATE INDEX` and the `WHERE` of a partial index each
/// hold one value per row, so a window function there is a misuse.
#[test]
fn a_window_function_in_a_create_index_is_refused() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t6(a, b, c)").unwrap();
    for (sql, message) in [
        (
            b"CREATE INDEX t6i ON t6(a) WHERE sum(b) OVER ()".as_slice(),
            "misuse of window function sum()",
        ),
        (
            b"CREATE INDEX t6i ON t6(a) WHERE lead(b) OVER ()",
            "misuse of window function lead()",
        ),
        (
            b"CREATE INDEX t6i ON t6(sum(b) OVER ())",
            "misuse of window function sum()",
        ),
        (
            b"CREATE INDEX t6i ON t6(abs(lead(b) OVER ()))",
            "misuse of window function lead()",
        ),
    ] {
        assert_eq!(refused(&mut writer, sql), message);
    }
    // A term that calls no window function is one the index holds.
    writer.run(b"CREATE INDEX t6i ON t6(abs(a))").unwrap();
}
