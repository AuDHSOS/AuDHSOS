// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the constraints a row is held to, against what the C
//! library answers for the same statements.

use alloc::string::String;
use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;
use crate::value::Value;

/// A connection the statements are run over.
fn connection() -> Writer {
    Writer::new(4096, 0, Encoding::Utf8).unwrap()
}

/// What the statements answer, the last of them first refused.
fn refusal(writer: &mut Writer, statements: &[&str]) -> String {
    let mut last = String::new();
    for sql in statements {
        match writer.run(sql.as_bytes()) {
            Ok(_) => last = String::new(),
            Err(error) => last = error.message(),
        }
    }
    last
}

/// The rows of a table, each value as text.
fn rows(writer: &Writer, sql: &str) -> Vec<String> {
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    let mut out = Vec::new();
    for row in &database.query(sql.as_bytes()).unwrap().rows {
        for value in row {
            out.push(match value {
                Value::Null => String::from("-"),
                Value::Int(number) => alloc::format!("{number}"),
                Value::Real(number) => {
                    String::from_utf8_lossy(&crate::fp::text(*number, 15)).into_owned()
                }
                Value::Text(bytes) | Value::Blob(bytes) => {
                    String::from_utf8_lossy(bytes).into_owned()
                }
            });
        }
    }
    out
}

#[test]
fn a_column_that_refuses_nothing_refuses_a_row_that_holds_nothing() {
    let mut writer = connection();
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE t(a NOT NULL, b)",
                "INSERT INTO t VALUES(NULL,1)"
            ]
        ),
        "NOT NULL constraint failed: t.a"
    );
    assert_eq!(rows(&writer, "SELECT count(*) FROM t"), ["0"]);
    // The same where the row is written by an `UPDATE`.
    assert_eq!(
        refusal(
            &mut writer,
            &["INSERT INTO t VALUES(1,1)", "UPDATE t SET a=NULL"]
        ),
        "NOT NULL constraint failed: t.a"
    );
    assert_eq!(rows(&writer, "SELECT a FROM t"), ["1"]);
}

#[test]
fn what_a_row_that_holds_nothing_does_is_what_the_statement_says() {
    // `sqlite3GenerateConstraintChecks`: `IGNORE` passes the row over,
    // `REPLACE` writes what the column falls back to, and a column
    // that falls back to nothing refuses the row instead.
    let mut writer = connection();
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE t(a NOT NULL, b)",
                "INSERT OR IGNORE INTO t VALUES(NULL,1)",
            ]
        ),
        ""
    );
    assert_eq!(rows(&writer, "SELECT count(*) FROM t"), ["0"]);
    assert_eq!(
        refusal(&mut writer, &["INSERT OR REPLACE INTO t VALUES(NULL,1)"]),
        "NOT NULL constraint failed: t.a"
    );
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE u(a NOT NULL DEFAULT 5, b)",
                "INSERT OR REPLACE INTO u VALUES(NULL,1)",
            ]
        ),
        ""
    );
    assert_eq!(rows(&writer, "SELECT a, b FROM u"), ["5", "1"]);
    // A constraint of the column's own says what happens where the
    // statement says nothing.
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE v(a NOT NULL ON CONFLICT IGNORE, b)",
                "INSERT INTO v VALUES(NULL,1)",
            ]
        ),
        ""
    );
    assert_eq!(rows(&writer, "SELECT count(*) FROM v"), ["0"]);
    // `FAIL` stops the statement where it stands and keeps the rows it
    // wrote before that.
    let mut writer = connection();
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE w(a NOT NULL, b)",
                "INSERT OR FAIL INTO w VALUES(1,1),(NULL,2),(3,3)",
            ]
        ),
        "NOT NULL constraint failed: w.a"
    );
    assert_eq!(rows(&writer, "SELECT count(*) FROM w"), ["1"]);
}

#[test]
fn a_check_of_the_table_holds_every_row_it_is_written_with() {
    let mut writer = connection();
    assert_eq!(
        refusal(
            &mut writer,
            &["CREATE TABLE t(a CHECK(a>0))", "INSERT INTO t VALUES(0)"]
        ),
        "CHECK constraint failed: a>0"
    );
    // A `CHECK` that answers nothing holds, and the text of the
    // constraint is written as it stands in the statement.
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE u(a CHECK( a > 0 ))",
                "INSERT INTO u VALUES(NULL)",
            ]
        ),
        ""
    );
    assert_eq!(rows(&writer, "SELECT count(*) FROM u"), ["1"]);
    assert_eq!(
        refusal(&mut writer, &["UPDATE u SET a=0"]),
        "CHECK constraint failed: a > 0"
    );
    // A constraint that carries a name is written under that name, and
    // one of the table holds the columns against each other.
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE v(a, CONSTRAINT positive CHECK(a>0))",
                "INSERT INTO v VALUES(0)",
            ]
        ),
        "CHECK constraint failed: positive"
    );
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE w(a, b, CHECK(a<b))",
                "INSERT INTO w VALUES(2,1)"
            ]
        ),
        "CHECK constraint failed: a<b"
    );
    // `IGNORE` passes the row over and `FAIL` stops the statement.
    assert_eq!(
        refusal(&mut writer, &["INSERT OR IGNORE INTO w VALUES(2,1)"]),
        ""
    );
    assert_eq!(rows(&writer, "SELECT count(*) FROM w"), ["0"]);
    assert_eq!(
        refusal(
            &mut writer,
            &["INSERT OR FAIL INTO w VALUES(1,2),(2,1),(3,4)"]
        ),
        "CHECK constraint failed: a<b"
    );
    // An `UPDATE` reads the same four, and `IGNORE` leaves the row as
    // it was.
    assert_eq!(refusal(&mut writer, &["UPDATE OR IGNORE w SET a=5"]), "");
    assert_eq!(rows(&writer, "SELECT a, b FROM w"), ["1", "2"]);
    assert_eq!(
        refusal(&mut writer, &["UPDATE w SET a=5"]),
        "CHECK constraint failed: a<b"
    );
    assert_eq!(rows(&writer, "SELECT count(*) FROM w"), ["1"]);
}

#[test]
fn a_check_of_the_table_is_left_unread_where_the_pragma_says_so() {
    // `PRAGMA ignore_check_constraints` leaves every `CHECK` of the
    // table unread, which is what `sqlite3GenerateConstraintChecks`
    // does under `SQLITE_IgnoreChecks`.
    let mut writer = connection();
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE t(a CHECK(a>0))",
                "PRAGMA ignore_check_constraints=1",
                "INSERT INTO t VALUES(0)",
            ]
        ),
        ""
    );
    assert_eq!(rows(&writer, "SELECT a FROM t"), ["0"]);
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "PRAGMA ignore_check_constraints=0",
                "INSERT INTO t VALUES(0)"
            ]
        ),
        "CHECK constraint failed: a>0"
    );
}

#[test]
fn the_key_a_row_is_written_under_is_one_the_table_does_not_hold() {
    // `sqlite3GenerateConstraintChecks` holds the key of the row
    // against the keys the table holds, whether the key is the rowid
    // or an index of the table's own.
    let mut writer = connection();
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE t(a INTEGER PRIMARY KEY, b)",
                "INSERT INTO t VALUES(1,'x')",
                "INSERT INTO t VALUES(1,'y')",
            ]
        ),
        "UNIQUE constraint failed: t.a"
    );
    assert_eq!(rows(&writer, "SELECT a, b FROM t"), ["1", "x"]);
    assert_eq!(
        refusal(&mut writer, &["INSERT OR IGNORE INTO t VALUES(1,'y')"]),
        ""
    );
    assert_eq!(rows(&writer, "SELECT a, b FROM t"), ["1", "x"]);
    assert_eq!(
        refusal(&mut writer, &["INSERT OR REPLACE INTO t VALUES(1,'y')"]),
        ""
    );
    assert_eq!(rows(&writer, "SELECT a, b FROM t"), ["1", "y"]);
    assert_eq!(
        refusal(
            &mut writer,
            &["INSERT OR FAIL INTO t VALUES(2,'z'),(1,'w'),(3,'v')"]
        ),
        "UNIQUE constraint failed: t.a"
    );
    assert_eq!(rows(&writer, "SELECT count(*) FROM t"), ["2"]);
    // A row written again under the key it already holds is the row it
    // already was.
    assert_eq!(refusal(&mut writer, &["UPDATE t SET a=1 WHERE a=1"]), "");
    assert_eq!(rows(&writer, "SELECT a, b FROM t WHERE a=1"), ["1", "y"]);
    assert_eq!(
        refusal(&mut writer, &["UPDATE t SET a=1 WHERE a=2"]),
        "UNIQUE constraint failed: t.a"
    );
    assert_eq!(
        refusal(&mut writer, &["UPDATE OR IGNORE t SET a=1 WHERE a=2"]),
        ""
    );
    assert_eq!(rows(&writer, "SELECT count(*) FROM t"), ["2"]);
    assert_eq!(
        refusal(&mut writer, &["UPDATE OR FAIL t SET a=1 WHERE a=2"]),
        "UNIQUE constraint failed: t.a"
    );
    assert_eq!(
        refusal(&mut writer, &["UPDATE OR REPLACE t SET a=1 WHERE a=2"]),
        ""
    );
    assert_eq!(rows(&writer, "SELECT a, b FROM t"), ["1", "z"]);
}

#[test]
fn the_columns_a_key_is_over_are_the_ones_the_message_names() {
    let mut writer = connection();
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE t(a, b, PRIMARY KEY(a,b))",
                "INSERT INTO t VALUES(1,1)",
                "INSERT INTO t VALUES(1,1)",
            ]
        ),
        "UNIQUE constraint failed: t.a, t.b"
    );
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE u(a UNIQUE, b)",
                "INSERT INTO u VALUES(1,1)",
                "INSERT INTO u VALUES(1,2)",
            ]
        ),
        "UNIQUE constraint failed: u.a"
    );
}

/// The key a statement wrote is held against the rows of the table
/// whether or not a column is another name for it, and the message names
/// `rowid` where none is, which `sqlite3RowidConstraint` writes.
#[test]
fn a_key_a_statement_wrote_is_held_against_the_rows_the_table_holds() {
    let mut writer = connection();
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE t(a, b, c, PRIMARY KEY(b, c))",
                "INSERT INTO t(rowid, a, b, c) VALUES(7, 1, 1, 1)",
                "INSERT INTO t(rowid, a, b, c) VALUES(7, 2, 2, 2)",
            ]
        ),
        "UNIQUE constraint failed: t.rowid"
    );
    assert_eq!(rows(&writer, "SELECT rowid, a FROM t"), ["7", "1"]);
    // `REPLACE` writes over the row the key names.
    writer
        .run(b"REPLACE INTO t(rowid, a, b, c) VALUES(7, 2, 2, 2)")
        .unwrap();
    assert_eq!(rows(&writer, "SELECT rowid, a FROM t"), ["7", "2"]);
    // An `UPDATE` that writes the key is held against them as well.
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "INSERT INTO t(rowid, a, b, c) VALUES(8, 3, 3, 3)",
                "UPDATE t SET rowid = 7 WHERE rowid = 8",
            ]
        ),
        "UNIQUE constraint failed: t.rowid"
    );
    // The column the key is another name for names itself.
    assert_eq!(
        refusal(
            &mut writer,
            &[
                "CREATE TABLE u(a INTEGER PRIMARY KEY, b)",
                "INSERT INTO u VALUES(1, 1)",
                "INSERT INTO u VALUES(1, 2)",
            ]
        ),
        "UNIQUE constraint failed: u.a"
    );
}

#[test]
fn what_a_statement_of_a_trigger_does_where_the_statement_that_fired_it_said_anything() {
    let mut writer = connection();
    for sql in [
        "CREATE TABLE tbl(a PRIMARY KEY, b, c)",
        "CREATE TRIGGER ai_tbl AFTER INSERT ON tbl BEGIN \
         INSERT OR IGNORE INTO tbl VALUES(new.a, 0, 0); END",
        "INSERT INTO tbl VALUES(1,2,3)",
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    // A statement that said nothing leaves the `OR IGNORE` of the body
    // standing, so the row the body writes is passed over.
    assert_eq!(rows(&writer, "SELECT a, b, c FROM tbl"), ["1", "2", "3"]);
    // `OR ABORT` holds the body to itself and undoes the row the
    // statement wrote.
    assert_eq!(
        refusal(&mut writer, &["INSERT OR ABORT INTO tbl VALUES(2,2,3)"]),
        "UNIQUE constraint failed: tbl.a"
    );
    assert_eq!(rows(&writer, "SELECT a, b, c FROM tbl"), ["1", "2", "3"]);
    // `OR FAIL` keeps the row the statement wrote.
    assert_eq!(
        refusal(&mut writer, &["INSERT OR FAIL INTO tbl VALUES(2,2,3)"]),
        "UNIQUE constraint failed: tbl.a"
    );
    assert_eq!(
        rows(&writer, "SELECT a, b, c FROM tbl"),
        ["1", "2", "3", "2", "2", "3"]
    );
    // `OR REPLACE` lets the body write over the row the statement wrote.
    writer
        .run(b"INSERT OR REPLACE INTO tbl VALUES(2,2,3)")
        .unwrap();
    assert_eq!(
        rows(&writer, "SELECT a, b, c FROM tbl"),
        ["1", "2", "3", "2", "0", "0"]
    );
}

#[test]
fn what_a_statement_of_a_trigger_an_update_fired_does_where_the_update_said_anything() {
    let mut writer = connection();
    for sql in [
        "CREATE TABLE tbl(a PRIMARY KEY, b, c)",
        "INSERT INTO tbl VALUES(4,2,3)",
        "INSERT INTO tbl VALUES(6,3,4)",
        "CREATE TRIGGER au_tbl AFTER UPDATE ON tbl BEGIN \
         UPDATE OR IGNORE tbl SET a = new.a, c = 10; END",
        "UPDATE tbl SET a = 1 WHERE a = 4",
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    assert_eq!(
        rows(&writer, "SELECT a, b, c FROM tbl"),
        ["1", "2", "10", "6", "3", "4"]
    );
    assert_eq!(
        refusal(&mut writer, &["UPDATE OR ABORT tbl SET a = 4 WHERE a = 1"]),
        "UNIQUE constraint failed: tbl.a"
    );
    assert_eq!(
        rows(&writer, "SELECT a, b, c FROM tbl"),
        ["1", "2", "10", "6", "3", "4"]
    );
    assert_eq!(
        refusal(&mut writer, &["UPDATE OR FAIL tbl SET a = 4 WHERE a = 1"]),
        "UNIQUE constraint failed: tbl.a"
    );
    assert_eq!(
        rows(&writer, "SELECT a, b, c FROM tbl"),
        ["4", "2", "10", "6", "3", "4"]
    );
    writer
        .run(b"UPDATE OR REPLACE tbl SET a = 1 WHERE a = 4")
        .unwrap();
    assert_eq!(rows(&writer, "SELECT a, b, c FROM tbl"), ["1", "3", "10"]);
}

#[test]
fn what_a_statement_of_a_trigger_a_delete_fired_does() {
    let mut writer = connection();
    for sql in [
        "CREATE TABLE tbl(a PRIMARY KEY, b)",
        "CREATE TABLE log(a PRIMARY KEY)",
        "INSERT INTO log VALUES(1)",
        "CREATE TRIGGER ad AFTER DELETE ON tbl BEGIN \
         INSERT OR IGNORE INTO log VALUES(1); END",
        "INSERT INTO tbl VALUES(1,1)",
        // A `DELETE` says nothing, which `sqlite3DeleteFrom` passes as
        // `OE_Default`, so the `OR IGNORE` of the body stands.
        "DELETE FROM tbl",
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    assert_eq!(rows(&writer, "SELECT count(*) FROM log"), ["1"]);
}

#[test]
fn what_a_replace_whose_deletion_fired_a_trigger_is_held_to() {
    let mut writer = connection();
    writer.run(b"PRAGMA recursive_triggers=ON").unwrap();
    for sql in [
        "CREATE TABLE t0(c0,c1)",
        "CREATE UNIQUE INDEX i0 ON t0(c0)",
        "INSERT INTO t0 VALUES(123,1)",
        "CREATE TRIGGER tr0 AFTER DELETE ON t0 BEGIN INSERT INTO t0 VALUES(123,2); END",
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    // The deletion the `REPLACE` made fired a trigger that wrote the key
    // again, so the key is held against the table once more and refuses
    // the row whatever the statement said.
    assert_eq!(
        refusal(&mut writer, &["REPLACE INTO t0 VALUES(123,3)"]),
        "UNIQUE constraint failed: t0.c0"
    );
    assert_eq!(rows(&writer, "SELECT c0, c1 FROM t0"), ["123", "1"]);

    // The key of the table is held against it again as well, which the
    // trigger of the deletion took.
    for sql in [
        "CREATE TABLE t2(aa,bb)",
        "CREATE UNIQUE INDEX t2bb ON t2(bb)",
        "CREATE TRIGGER r BEFORE DELETE ON t2 BEGIN INSERT INTO t2(aa,bb) VALUES(99,1); END",
        "INSERT INTO t2(aa,bb) VALUES(10,20)",
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    assert_eq!(
        refusal(&mut writer, &["REPLACE INTO t2(aa,bb) VALUES(30,20)"]),
        "UNIQUE constraint failed: t2.rowid"
    );
    assert_eq!(rows(&writer, "SELECT aa, bb FROM t2"), ["10", "20"]);

    // The refusal names the column the key is another name for.
    for sql in [
        "CREATE TABLE t3(a INTEGER PRIMARY KEY, b)",
        "CREATE INDEX t3b ON t3(b)",
        "INSERT INTO t3 VALUES(1,'one')",
        "CREATE TRIGGER tr3 AFTER DELETE ON t3 BEGIN INSERT INTO t3 VALUES(1,'three'); END",
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    assert_eq!(
        refusal(&mut writer, &["REPLACE INTO t3 VALUES(1,'two')"]),
        "UNIQUE constraint failed: t3.a"
    );
    assert_eq!(rows(&writer, "SELECT a, b FROM t3"), ["1", "one"]);
}

#[test]
fn which_order_the_keys_of_a_table_are_held_against_a_row_in() {
    let mut writer = connection();
    writer.run(b"PRAGMA recursive_triggers=ON").unwrap();
    for sql in [
        "CREATE TABLE t1(a, b UNIQUE, c UNIQUE)",
        "INSERT INTO t1(a,b,c) VALUES(1,1,1),(2,2,2),(3,3,3),(4,4,4)",
        "CREATE TRIGGER r AFTER DELETE ON t1 WHEN OLD.c<>3 \
         BEGIN INSERT INTO t1(rowid,a,b,c) VALUES(100,100,100,3); END",
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    // The index made last is held against the row first, so the key `c`
    // is answered before the key `b` and the row the trigger of the `b`
    // deletion wrote holds `c` again.
    assert_eq!(
        refusal(
            &mut writer,
            &["REPLACE INTO t1(rowid,a,b,c) VALUES(200,1,2,3)"]
        ),
        "UNIQUE constraint failed: t1.c"
    );
    assert_eq!(rows(&writer, "SELECT a FROM t1"), ["1", "2", "3", "4"]);

    // An index whose own clause says `REPLACE` stands at the end of the
    // list, so the two indexes that refuse the row are answered first.
    for sql in [
        "CREATE TABLE u1(x PRIMARY KEY, UNIQUE(x,x), UNIQUE(x,x) ON CONFLICT REPLACE)",
        "INSERT INTO u1(x) VALUES(1)",
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    assert_eq!(
        refusal(&mut writer, &["INSERT INTO u1(x) VALUES(1)"]),
        "UNIQUE constraint failed: u1.x"
    );
    assert_eq!(rows(&writer, "SELECT x FROM u1"), ["1"]);
}

#[test]
fn what_a_name_after_default_stands_for() {
    let mut writer = connection();
    for sql in [
        // `sqlite3Dequote` leaves a name that opens with no quote alone,
        // and `sqlite3ExprIdToTrueFalse` reads `true` and `false` as
        // numbers.
        "CREATE TABLE t(a DEFAULT true, b DEFAULT false, c DEFAULT hi, \
         d DEFAULT \"quoted\", e DEFAULT [bracket], f DEFAULT 'text', \
         g DEFAULT `tick`)",
        "INSERT INTO t DEFAULT VALUES",
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    assert_eq!(
        rows(&writer, "SELECT a, b, c, d, e, f, g FROM t"),
        ["1", "0", "hi", "quoted", "bracket", "text", "tick"]
    );
    assert_eq!(
        rows(&writer, "SELECT typeof(a), typeof(b), typeof(c) FROM t"),
        ["integer", "integer", "text"]
    );
}
