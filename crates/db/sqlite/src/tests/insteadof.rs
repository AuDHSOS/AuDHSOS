// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `INSERT`, `UPDATE` and `DELETE` over a view, against what the shell
//! answers for the same statements.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// The rows of `log`, as `x|y|w` lines.
fn logged(writer: &Writer) -> String {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let mut shown = String::new();
    for (_, values) in database.rows_of(b"log").unwrap() {
        for (at, value) in values.iter().enumerate() {
            if at > 0 {
                shown.push('|');
            }
            shown.push_str(&String::from_utf8_lossy(
                &value.text().unwrap_or(b"NULL".to_vec()),
            ));
        }
        shown.push('\n');
    }
    shown
}

/// A connection over a table, a view of it and the three `INSTEAD OF`
/// triggers, each writing the table and a log.
fn writer() -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a, b)".as_slice(),
        b"CREATE TABLE log(x, y, w)",
        b"CREATE VIEW v AS SELECT a, b FROM t",
        b"CREATE TRIGGER vi INSTEAD OF INSERT ON v BEGIN \
          INSERT INTO t VALUES(new.a, new.b); \
          INSERT INTO log VALUES(new.a, new.b, 'i'); END",
        b"CREATE TRIGGER vu INSTEAD OF UPDATE ON v BEGIN \
          UPDATE t SET b = new.b WHERE a = old.a; \
          INSERT INTO log VALUES(old.a, new.b, 'u'); END",
        b"CREATE TRIGGER vd INSTEAD OF DELETE ON v BEGIN \
          DELETE FROM t WHERE a = old.a; \
          INSERT INTO log VALUES(old.a, old.b, 'd'); END",
    ] {
        writer.run(sql).unwrap();
    }
    writer
}

/// The rows of `t`, as `a|b` lines.
fn held(writer: &Writer) -> String {
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    let mut shown = String::new();
    for (_, values) in database.rows_of(b"t").unwrap() {
        for (at, value) in values.iter().enumerate() {
            if at > 0 {
                shown.push('|');
            }
            shown.push_str(&String::from_utf8_lossy(
                &value.text().unwrap_or(b"NULL".to_vec()),
            ));
        }
        shown.push('\n');
    }
    shown
}

/// The three statements over a view run the triggers and write no row
/// of the view, which is `sqlite3Insert`, `sqlite3Update` and
/// `sqlite3DeleteFrom` over a view.
#[test]
fn a_statement_over_a_view_runs_the_instead_of_triggers() {
    let mut writer = writer();
    for sql in [
        b"INSERT INTO v VALUES(1, 2)".as_slice(),
        b"INSERT INTO v VALUES(3, 4)",
        b"UPDATE v SET b = 99 WHERE a = 1",
        b"DELETE FROM v WHERE a = 3",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(held(&writer), "1|99\n");
    assert_eq!(logged(&writer), "1|2|i\n3|4|i\n1|99|u\n3|4|d\n");
    // A statement with no `WHERE` runs the triggers over every row the
    // view answers.
    writer.run(b"UPDATE v SET b = 5").unwrap();
    writer.run(b"DELETE FROM v").unwrap();
    assert_eq!(held(&writer), "");
    assert_eq!(
        logged(&writer),
        "1|2|i\n3|4|i\n1|99|u\n3|4|d\n1|5|u\n1|5|d\n"
    );
}

/// A value an `INSERT` writes into a view stands as it was given: a
/// view column converts nothing, which the shell answers `text` and
/// `integer` for.
#[test]
fn a_view_column_converts_nothing_a_statement_writes_into_it() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a INTEGER, b TEXT)".as_slice(),
        b"CREATE TABLE log(x, y, w)",
        b"CREATE VIEW v AS SELECT a, b FROM t",
        b"CREATE TRIGGER vi INSTEAD OF INSERT ON v BEGIN \
          INSERT INTO log VALUES(typeof(new.a), typeof(new.b), new.a); END",
        b"INSERT INTO v VALUES('5', 7)",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(logged(&writer), "text|integer|5\n");
}

/// A column an `INSERT` over a view names no value for holds nothing,
/// `DEFAULT VALUES` writes one row of nothing, and a value written for
/// the key of a view goes nowhere.
#[test]
fn what_an_insert_over_a_view_leaves_a_column_holding() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a, b)".as_slice(),
        b"CREATE TABLE log(x, y, w)",
        b"CREATE VIEW v AS SELECT a, b FROM t",
        b"CREATE TRIGGER vi INSTEAD OF INSERT ON v BEGIN \
          INSERT INTO log VALUES(new.a, new.b, 'i'); END",
        b"INSERT INTO v(b) VALUES(9)",
        b"INSERT INTO v DEFAULT VALUES",
        b"INSERT INTO v(rowid) VALUES(1)",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(logged(&writer), "NULL|9|i\nNULL|NULL|i\nNULL|NULL|i\n");
}

/// What a statement over a view is refused with.
#[test]
fn what_a_statement_over_a_view_refuses() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t(a, b)".as_slice(),
        b"CREATE VIEW v AS SELECT a, b FROM t",
    ] {
        writer.run(sql).unwrap();
    }
    // A view with no `INSTEAD OF` trigger of that event.
    for sql in [
        b"INSERT INTO v VALUES(1, 2)".as_slice(),
        b"UPDATE v SET a = 1",
        b"DELETE FROM v",
    ] {
        assert_eq!(
            writer.run(sql).unwrap_err().message(),
            "cannot modify v because it is a view",
            "{sql:?}"
        );
    }
    writer
        .run(b"CREATE TRIGGER vi INSTEAD OF INSERT ON v BEGIN SELECT 1; END")
        .unwrap();
    writer
        .run(b"CREATE TRIGGER vu INSTEAD OF UPDATE ON v BEGIN SELECT 1; END")
        .unwrap();
    // The values of a row are counted against the columns of the view.
    assert_eq!(
        writer
            .run(b"INSERT INTO v VALUES(1)")
            .unwrap_err()
            .message(),
        "table v has 2 columns but 1 values were supplied"
    );
    assert_eq!(
        writer
            .run(b"INSERT INTO v(a) VALUES(1, 2)")
            .unwrap_err()
            .message(),
        "2 values for 1 columns"
    );
    // A column the view does not answer.
    assert_eq!(
        writer
            .run(b"INSERT INTO v(zz) VALUES(1)")
            .unwrap_err()
            .message(),
        "table v has no column named zz"
    );
    assert_eq!(
        writer.run(b"UPDATE v SET zz = 1").unwrap_err().message(),
        "no such column: zz"
    );
    // A view has no key, so a `SET` of the key writes nowhere.
    writer.run(b"UPDATE v SET rowid = 1").unwrap();
    // Only a view carries an `INSTEAD OF` trigger.
    assert_eq!(
        writer
            .run(b"CREATE TRIGGER ti INSTEAD OF INSERT ON t BEGIN SELECT 1; END")
            .unwrap_err()
            .message(),
        "cannot create INSTEAD OF trigger on table: t"
    );
}
