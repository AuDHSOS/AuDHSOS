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
