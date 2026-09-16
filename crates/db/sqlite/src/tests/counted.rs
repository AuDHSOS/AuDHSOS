// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `changes()`, `total_changes()` and `last_insert_rowid()`, against
//! what the shell answers for the same statements.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// The three counters of the connection, as `changes|total|rowid`.
fn counts(writer: &Writer) -> String {
    let image = writer.written();
    let database = Database::open(&image).unwrap().counting(writer.counts());
    let answered = database
        .query(b"SELECT changes(), total_changes(), last_insert_rowid()")
        .unwrap();
    let mut shown = String::new();
    for value in answered.rows.first().unwrap() {
        if !shown.is_empty() {
            shown.push('|');
        }
        shown.push_str(&String::from_utf8_lossy(
            &value.text().unwrap_or(b"NULL".to_vec()),
        ));
    }
    shown
}

/// A `CREATE` leaves the three as it found them, an `UPDATE` and a
/// `DELETE` leave the rowid, and a trigger's body counts toward the
/// total alone.
#[test]
fn the_three_counters_answer_what_the_connection_has_written() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    assert_eq!(counts(&writer), "0|0|0");
    writer.run(b"INSERT INTO t VALUES(1),(2),(3)").unwrap();
    assert_eq!(counts(&writer), "3|3|3");
    writer.run(b"CREATE TABLE u(b)").unwrap();
    assert_eq!(counts(&writer), "3|3|3");
    writer.run(b"UPDATE t SET a=9").unwrap();
    assert_eq!(counts(&writer), "3|6|3");
    writer.run(b"DELETE FROM t WHERE a=9").unwrap();
    assert_eq!(counts(&writer), "3|9|3");
    writer.run(b"INSERT INTO u VALUES(7)").unwrap();
    assert_eq!(counts(&writer), "1|10|1");
    writer.run(b"CREATE TABLE v(c)").unwrap();
    writer
        .run(b"CREATE TRIGGER tr AFTER INSERT ON u BEGIN INSERT INTO v VALUES(new.b); END")
        .unwrap();
    // The row the body writes counts toward the total, and the key it
    // wrote is one the statement that fired the trigger does not
    // answer.
    writer.run(b"INSERT INTO u VALUES(8)").unwrap();
    assert_eq!(counts(&writer), "1|12|2");
}

/// An `INSERT` that wrote no row and an `INSERT` into a table that
/// keeps its rows in the key's own tree each leave the rowid as they
/// found it.
#[test]
fn what_leaves_the_last_insert_rowid_as_it_stands() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a INTEGER PRIMARY KEY)".as_slice(),
        b"INSERT INTO t VALUES(5)",
        b"INSERT INTO t VALUES(6)",
        b"INSERT OR IGNORE INTO t VALUES(6)",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(counts(&writer), "0|2|6");
    writer
        .run(b"CREATE TABLE w(k INTEGER PRIMARY KEY) WITHOUT ROWID")
        .unwrap();
    writer.run(b"INSERT INTO w VALUES(123456)").unwrap();
    assert_eq!(counts(&writer), "1|3|6");
}

/// A statement of a trigger's body sets `changes()` and
/// `last_insert_rowid()` as it runs, and the statement that fired the
/// trigger sets both again when it ends.
#[test]
fn a_triggers_body_sets_the_counters_as_it_runs() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t0(x)".as_slice(),
        b"INSERT INTO t0 VALUES(1),(1),(1)",
        b"CREATE TABLE t1(k INTEGER PRIMARY KEY)",
        b"CREATE TABLE n1(k INTEGER PRIMARY KEY, n, r)",
        b"CREATE TRIGGER r1 AFTER INSERT ON t1 BEGIN \
          INSERT INTO n1 VALUES(NULL, changes(), last_insert_rowid()); \
          UPDATE t0 SET x=x*10; \
          INSERT INTO n1 VALUES(NULL, changes(), last_insert_rowid()); END",
        b"INSERT INTO t1 VALUES(77)",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(counts(&writer), "1|9|77");
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let answered = database.query(b"SELECT n, r FROM n1").unwrap();
    let mut shown = String::new();
    for row in &answered.rows {
        for value in row {
            shown.push_str(&String::from_utf8_lossy(
                &value.text().unwrap_or(b"NULL".to_vec()),
            ));
            shown.push('|');
        }
    }
    assert_eq!(shown, "3|77|3|1|");
}

/// A statement that is refused leaves `changes()` at the rows that
/// stand: nought where the file went back to where the statement found
/// it, and what the statement wrote where it stopped where it stood.
#[test]
fn a_refused_statement_counts_the_rows_that_stand() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a UNIQUE, b)".as_slice(),
        b"INSERT INTO t VALUES(1,1),(2,2),(3,3)",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(counts(&writer), "3|3|3");
    // `OE_Abort` puts the file back, so no row of the statement stands.
    assert_eq!(
        writer
            .run(b"INSERT INTO t VALUES(4,4),(1,1)")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.a"
    );
    // The key of a row the statement wrote stands although the row
    // does not, which is what `sqlite3_last_insert_rowid` answers.
    assert_eq!(counts(&writer), "0|3|4");
    // `OE_Fail` keeps the rows the statement wrote before it stopped.
    assert_eq!(
        writer
            .run(b"INSERT OR FAIL INTO t VALUES(5,5),(6,6),(1,1),(7,7)")
            .unwrap_err()
            .message(),
        "UNIQUE constraint failed: t.a"
    );
    assert_eq!(counts(&writer), "2|5|5");
}

/// Two connections over one file each carry their own counters, which
/// [`Writer::counts_as`] is what a caller that holds one writer per
/// file sets before a statement.
#[test]
fn a_connection_carries_its_own_counters_over_a_shared_file() {
    use crate::func::Counted;
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1),(2)").unwrap();
    assert_eq!(counts(&writer), "2|2|2");
    // The second connection has written nothing of its own.
    writer.counts_as(Counted::default());
    assert_eq!(counts(&writer), "0|0|0");
    writer.run(b"INSERT INTO t VALUES(3)").unwrap();
    assert_eq!(counts(&writer), "1|1|3");
}
