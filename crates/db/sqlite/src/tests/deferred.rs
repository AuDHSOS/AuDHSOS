// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Foreign keys held at the end of the transaction, against what the
//! shell answers for the same statements.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// What a statement answers, as `value|` per value.
fn shown(writer: &Writer, sql: &[u8]) -> String {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let answered = database.query(sql).unwrap();
    let mut out = String::new();
    for row in &answered.rows {
        for value in row {
            out.push_str(&String::from_utf8_lossy(
                &value.text().unwrap_or(b"NULL".to_vec()),
            ));
            out.push('|');
        }
    }
    out
}

/// A connection over a table that points at itself under a key held at
/// the end of the transaction.
fn writer() -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"PRAGMA foreign_keys=ON".as_slice(),
        b"CREATE TABLE node(nodeid PRIMARY KEY, \
          parent REFERENCES node DEFERRABLE INITIALLY DEFERRED)",
    ] {
        writer.run(sql).unwrap();
    }
    writer
}

/// A statement of its own commits at its end, so a key held there is
/// held by the statement; inside a transaction the `COMMIT` holds it
/// and the transaction stays open.
#[test]
fn a_key_held_at_the_end_of_the_transaction_is_held_by_the_commit() {
    let mut writer = writer();
    assert_eq!(
        writer
            .run(b"INSERT INTO node VALUES(1, 0)")
            .unwrap_err()
            .message(),
        "FOREIGN KEY constraint failed"
    );
    // The row the statement wrote is gone with it.
    assert_eq!(shown(&writer, b"SELECT count(*) FROM node"), "0|");
    writer.run(b"BEGIN").unwrap();
    writer.run(b"INSERT INTO node VALUES(1, 0)").unwrap();
    assert_eq!(
        writer.run(b"COMMIT").unwrap_err().message(),
        "FOREIGN KEY constraint failed"
    );
    // The transaction stays open, so the statement that mends the row
    // runs inside it.
    writer.run(b"UPDATE node SET parent = NULL").unwrap();
    writer.run(b"COMMIT").unwrap();
    assert_eq!(
        shown(&writer, b"SELECT nodeid, parent FROM node"),
        "1|NULL|"
    );
}

/// A row that appears is the parent the rows waiting for it were
/// counted against, and `PRAGMA defer_foreign_keys` holds every key at
/// the end of the transaction.
#[test]
fn a_row_that_appears_answers_the_rows_that_were_waiting_for_it() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"PRAGMA foreign_keys=ON".as_slice(),
        b"CREATE TABLE p(a PRIMARY KEY)",
        b"CREATE TABLE c(b REFERENCES p)",
        b"PRAGMA defer_foreign_keys=ON",
        b"BEGIN",
        b"INSERT INTO c VALUES(1)",
        b"INSERT INTO p VALUES(1)",
        b"COMMIT",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(shown(&writer, b"SELECT b FROM c"), "1|");
}

/// A row that goes stops pointing at nothing, and a `ROLLBACK TO` puts
/// the count back where the savepoint found it.
#[test]
fn a_row_that_goes_stops_being_counted_and_a_savepoint_puts_the_count_back() {
    let mut writer = writer();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"INSERT INTO node VALUES(1, 0)").unwrap();
    writer.run(b"DELETE FROM node WHERE nodeid=1").unwrap();
    writer.run(b"COMMIT").unwrap();
    assert_eq!(shown(&writer, b"SELECT count(*) FROM node"), "0|");
    writer.run(b"SAVEPOINT outer").unwrap();
    writer.run(b"INSERT INTO node VALUES(2, 9)").unwrap();
    // Releasing the savepoint that opened the transaction writes it, so
    // the key is held there and the savepoint stays open.
    assert_eq!(
        writer.run(b"RELEASE outer").unwrap_err().message(),
        "FOREIGN KEY constraint failed"
    );
    writer.run(b"ROLLBACK TO outer").unwrap();
    writer.run(b"RELEASE outer").unwrap();
    assert_eq!(shown(&writer, b"SELECT count(*) FROM node"), "0|");
}

/// `RESTRICT` is held where the row is written whatever the key says,
/// and `NO ACTION` on a key held at the end of the transaction counts
/// the rows that lose their parent.
#[test]
fn what_restrict_and_no_action_do_under_a_deferred_key() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"PRAGMA foreign_keys=ON".as_slice(),
        b"CREATE TABLE p(a PRIMARY KEY)",
        b"CREATE TABLE c(b REFERENCES p ON DELETE RESTRICT \
          DEFERRABLE INITIALLY DEFERRED)",
        b"CREATE TABLE d(e REFERENCES p DEFERRABLE INITIALLY DEFERRED)",
        b"INSERT INTO p VALUES(1)",
        b"INSERT INTO c VALUES(1)",
        b"INSERT INTO d VALUES(1)",
        b"BEGIN",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(
        writer
            .run(b"DELETE FROM p WHERE a=1")
            .unwrap_err()
            .message(),
        "FOREIGN KEY constraint failed"
    );
    writer.run(b"DELETE FROM c").unwrap();
    // `NO ACTION` counts the row of `d` that loses its parent rather
    // than refusing where the row goes.
    writer.run(b"DELETE FROM p WHERE a=1").unwrap();
    assert_eq!(
        writer.run(b"COMMIT").unwrap_err().message(),
        "FOREIGN KEY constraint failed"
    );
    writer.run(b"DELETE FROM d").unwrap();
    writer.run(b"COMMIT").unwrap();
    assert_eq!(shown(&writer, b"SELECT count(*) FROM p"), "0|");
}

/// A row that appears answers only the rows waiting under a key held
/// at the end of the transaction, and only where the columns it is
/// pointed at by hold something.
#[test]
fn what_a_row_that_appears_leaves_counted() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"PRAGMA foreign_keys=ON".as_slice(),
        b"CREATE TABLE p(a PRIMARY KEY)",
        b"CREATE TABLE c1(b REFERENCES p DEFERRABLE INITIALLY DEFERRED)",
        b"CREATE TABLE c2(b REFERENCES p)",
        b"BEGIN",
        b"INSERT INTO c1 VALUES(1)",
        // The row answers the row of `c1` that was waiting; the key of
        // `c2` is held where a row is written, so no row waits under it.
        b"INSERT INTO p VALUES(1)",
        b"COMMIT",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(shown(&writer, b"SELECT count(*) FROM c1"), "1|");
    // A row whose column the key points at holds nothing answers no row.
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"PRAGMA foreign_keys=ON".as_slice(),
        b"CREATE TABLE p(a PRIMARY KEY, u UNIQUE)",
        b"CREATE TABLE c(b REFERENCES p(u) DEFERRABLE INITIALLY DEFERRED)",
        b"BEGIN",
        b"INSERT INTO c VALUES(1)",
        b"INSERT INTO p VALUES(1, NULL)",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(
        writer.run(b"COMMIT").unwrap_err().message(),
        "FOREIGN KEY constraint failed"
    );
}

/// A row written while the connection held no key to its foreign keys
/// points at no row, and taking it away counts nothing.
#[test]
fn a_row_that_points_at_no_row_under_a_key_held_at_once_counts_nothing() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"PRAGMA foreign_keys=OFF".as_slice(),
        b"CREATE TABLE p(a PRIMARY KEY)",
        b"CREATE TABLE c(b REFERENCES p)",
        b"INSERT INTO c VALUES(1)",
        b"PRAGMA foreign_keys=ON",
        b"DELETE FROM c",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(shown(&writer, b"SELECT count(*) FROM c"), "0|");
}

/// `PRAGMA defer_foreign_keys` is off again once the transaction ends,
/// whether a `COMMIT` or a `ROLLBACK` ended it and whether the statement
/// that ran under it was one of its own.
#[test]
fn what_deferring_every_key_holds_once_the_transaction_ends() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"PRAGMA foreign_keys=ON".as_slice(),
        b"CREATE TABLE one(x PRIMARY KEY)",
        b"CREATE TABLE two(y REFERENCES one(x))",
        b"INSERT INTO one VALUES(1)",
    ] {
        writer.run(sql).expect("a statement the writer takes");
    }
    let deferring = |writer: &mut Writer| -> i64 {
        writer
            .run(b"PRAGMA defer_foreign_keys")
            .unwrap()
            .first()
            .and_then(|row| row.first())
            .map_or(-1, crate::value::Value::to_integer)
    };
    // A statement of its own commits at its end, so the key is held
    // there whatever the pragma says, and the rollback of the statement
    // leaves the pragma off.
    writer.run(b"PRAGMA defer_foreign_keys=ON").unwrap();
    assert_eq!(
        writer
            .run(b"INSERT INTO two VALUES(2)")
            .unwrap_err()
            .message(),
        "FOREIGN KEY constraint failed"
    );
    assert_eq!(deferring(&mut writer), 0);
    // A statement that writes nothing the pragma holds leaves it off as
    // well, because it commits at its end.
    writer.run(b"PRAGMA defer_foreign_keys=ON").unwrap();
    writer.run(b"INSERT INTO one VALUES(2)").unwrap();
    assert_eq!(deferring(&mut writer), 0);
    for ending in [b"COMMIT".as_slice(), b"ROLLBACK"] {
        writer.run(b"BEGIN").unwrap();
        writer.run(b"PRAGMA defer_foreign_keys=ON").unwrap();
        assert_eq!(deferring(&mut writer), 1);
        writer.run(b"DELETE FROM two").unwrap();
        writer.run(ending).unwrap();
        assert_eq!(
            deferring(&mut writer),
            0,
            "{}",
            alloc::string::String::from_utf8_lossy(ending)
        );
    }
}
