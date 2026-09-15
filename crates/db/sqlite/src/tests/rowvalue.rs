// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of the rows of values a statement compares, against what the
//! C library answers for the same statements.

use alloc::string::String;
use alloc::vec::Vec;

use crate::change::Writer;
use crate::db::{Database, Error};
use crate::header::Encoding;
use crate::value::Value;

/// The tables the tests read.
const TABLES: &[&str] = &[
    "CREATE TABLE t(a,b,c)",
    "INSERT INTO t VALUES(0,0,0),(0,1,1),(1,0,2),(1,1,3)",
    "CREATE TABLE u(x,y)",
    "INSERT INTO u VALUES(1,0),(9,9)",
    "CREATE TABLE w(a TEXT, b TEXT)",
    "INSERT INTO w VALUES('1','2')",
    "CREATE TABLE empty(x,y)",
];

/// A connection holding the tables.
fn connection() -> Writer {
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    for sql in TABLES {
        writer.run(sql.as_bytes()).unwrap();
    }
    writer
}

/// What one value looks like as a list element.
fn shown(value: &Value) -> String {
    match value {
        Value::Null => String::from("NULL"),
        Value::Int(number) => alloc::format!("{number}"),
        Value::Real(number) => String::from_utf8_lossy(&crate::fp::text(*number, 15)).into_owned(),
        Value::Text(bytes) | Value::Blob(bytes) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// What a statement answers, every value of every row in order.
fn answered(writer: &Writer, sql: &str) -> Vec<String> {
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    let mut out = Vec::new();
    for row in &database.query(sql.as_bytes()).unwrap().rows {
        for value in row {
            out.push(shown(value));
        }
    }
    out
}

/// What a statement refuses with.
fn refusal(writer: &Writer, sql: &str) -> Error {
    let bytes = writer.written();
    let database = Database::open(&bytes).unwrap();
    database.query(sql.as_bytes()).unwrap_err()
}

#[test]
fn a_row_compared_against_a_row_answers_what_the_c_library_answers() {
    let writer = connection();
    for (sql, want) in [
        ("SELECT (1,2)=(1,2)", "1"),
        ("SELECT (1,2)=(1,3)", "0"),
        ("SELECT (1,NULL)=(1,2)", "NULL"),
        ("SELECT (1,NULL)=(2,2)", "0"),
        ("SELECT (1,2)<>(1,3)", "1"),
        ("SELECT (1,NULL)<>(1,2)", "NULL"),
        ("SELECT (1,2)<(1,3)", "1"),
        ("SELECT (1,2)<=(1,2)", "1"),
        ("SELECT (1,2)>(1,1)", "1"),
        ("SELECT (1,2)>=(1,3)", "0"),
        ("SELECT (1,NULL)<(1,2)", "NULL"),
        ("SELECT (NULL,1)<(2,2)", "NULL"),
        ("SELECT (1,NULL) IS (1,NULL)", "1"),
        ("SELECT (1,NULL) IS NOT (1,2)", "1"),
        ("SELECT ('a','b')<('a','c')", "1"),
        ("SELECT (1,2)=(SELECT 1,2)", "1"),
        ("SELECT (1,2)>(SELECT 1,1)", "1"),
    ] {
        assert_eq!(answered(&writer, sql), [want], "{sql}");
    }
}

#[test]
fn a_row_looked_for_among_rows_answers_what_the_c_library_answers() {
    let writer = connection();
    for (sql, want) in [
        ("SELECT (1,2) IN ((1,2),(3,4))", "1"),
        ("SELECT (1,2) IN ((3,4))", "0"),
        ("SELECT (1,2) IN ((1,NULL))", "NULL"),
        ("SELECT (1,2) NOT IN ((1,2))", "0"),
        ("SELECT (1,2) NOT IN ((1,NULL))", "NULL"),
        ("SELECT (1,2) IN (SELECT 1,2)", "1"),
        ("SELECT (1,2) IN (SELECT 3,NULL)", "0"),
        ("SELECT (1,2) BETWEEN (1,1) AND (2,2)", "1"),
        ("SELECT (1,2) NOT BETWEEN (1,1) AND (2,2)", "0"),
        ("SELECT (1,NULL) BETWEEN (1,1) AND (2,2)", "NULL"),
        ("SELECT (1,2) BETWEEN (3,1) AND (4,2)", "0"),
    ] {
        assert_eq!(answered(&writer, sql), [want], "{sql}");
    }
}

#[test]
fn a_row_of_columns_reads_the_rows_the_c_library_reads() {
    let writer = connection();
    for (sql, want) in [
        ("SELECT c FROM t WHERE (a,b) >= (1,0)", "2,3"),
        ("SELECT c FROM t WHERE (a,b) > (1,0)", "3"),
        ("SELECT c FROM t WHERE (a,b) IN (SELECT x,y FROM u)", "2"),
        ("SELECT c FROM t WHERE (a,b) IN u", "2"),
        (
            "SELECT c FROM t WHERE (a,b) NOT IN (SELECT x,y FROM u)",
            "0,1,3",
        ),
        (
            "SELECT count(*) FROM t WHERE (a,b) = (SELECT x,y FROM empty)",
            "0",
        ),
    ] {
        let rows = answered(&writer, sql);
        assert_eq!(rows.join(","), want, "{sql}");
    }
}

#[test]
fn a_statement_used_as_a_value_compares_under_the_affinity_of_its_column() {
    // `sqlite3ExprAffinity` of a `TK_SELECT` is the affinity of the
    // column the statement answers, so a text column compared against
    // a column of no type converts neither side.
    let writer = connection();
    assert_eq!(answered(&writer, "SELECT a=1 FROM w"), ["1"]);
    assert_eq!(
        answered(&writer, "SELECT a=(SELECT x FROM u) FROM w"),
        ["0"]
    );
    assert_eq!(answered(&writer, "SELECT a=x FROM w,u WHERE x=1"), ["0"]);
    assert_eq!(answered(&writer, "SELECT (SELECT b FROM w)"), ["2"]);
    assert_eq!(answered(&writer, "SELECT (SELECT x FROM empty)"), ["NULL"]);
}

#[test]
fn a_row_written_where_one_value_belongs_is_refused() {
    let writer = connection();
    for sql in [
        "SELECT (1,2)",
        "SELECT ((1,2))",
        "SELECT (1,2)+(1,2)",
        "SELECT (1,2)||(1,2)",
        "SELECT (1,2)=(1,2,3)",
        "SELECT (1,2)=1",
        "SELECT 1=(1,2)",
        "SELECT abs((1,2))",
        "SELECT (1,2) IN (1,2)",
        "SELECT (1,2) BETWEEN 1 AND 2",
        "SELECT (1,2) BETWEEN (1,1) AND 3",
        "SELECT (1,2) BETWEEN (1,1,1) AND (2,2)",
        "SELECT c FROM t WHERE (a,b)<=1",
        "SELECT (1,2) IS 1",
    ] {
        assert_eq!(
            refusal(&writer, sql),
            Error::Eval(crate::eval::Error::RowValue),
            "{sql}"
        );
    }
    assert_eq!(
        Error::Eval(crate::eval::Error::RowValue).message(),
        "row value misused"
    );
}

#[test]
fn a_statement_used_as_a_value_answers_one_column() {
    let writer = connection();
    for sql in ["SELECT (SELECT 1,2)", "SELECT 1=(SELECT 1,2)"] {
        assert_eq!(
            refusal(&writer, sql),
            Error::Eval(crate::eval::Error::Columns),
            "{sql}"
        );
    }
}

#[test]
fn a_row_compared_against_a_statement_needs_a_connection_to_answer_it() {
    // `crate::eval::evaluate` reads one row and knows no tables, so a
    // statement written under an expression refuses there.
    let sql = b"(1,2)=(SELECT 1,2)";
    let (arena, root) = crate::parse::expression(sql).unwrap();
    assert_eq!(
        crate::eval::evaluate(&arena, root, sql),
        Err(crate::eval::Error::Unsupported)
    );
}

#[test]
fn a_row_written_into_a_table_is_the_row_of_a_values() {
    // A `VALUES` writes one row node per row, which is the one place a
    // row stands outside a comparison.
    let mut writer = connection();
    writer.run(b"INSERT INTO empty VALUES(1,2),(3,4)").unwrap();
    assert_eq!(answered(&writer, "SELECT x FROM empty"), ["1", "3"]);
    assert_eq!(
        writer.run(b"INSERT INTO empty SELECT (1,2),3"),
        Err(Error::Eval(crate::eval::Error::RowValue))
    );
    assert_eq!(
        writer.run(b"DELETE FROM empty WHERE (x,y)=1"),
        Err(Error::Eval(crate::eval::Error::RowValue))
    );
    assert_eq!(
        writer.run(b"UPDATE empty SET x=1 WHERE (x,y)=(1,2)"),
        Ok(Vec::new())
    );
}

#[test]
fn a_view_written_over_a_row_is_refused_where_it_is_read() {
    // The body of a view is read when a statement names the view, so a
    // row misused inside one refuses there and not where the view was
    // written, which is what `sqlite3ViewGetColumnNames` does.
    let mut writer = connection();
    for (name, body) in [
        ("v1", "SELECT (a,b)=(1,2,3) FROM t"),
        ("v2", "SELECT (a,b)+(1,2) FROM t"),
        ("v3", "SELECT (a,b)=1 FROM t"),
        ("v4", "SELECT (a,b) IN ((1,2,3)) FROM t"),
        ("v5", "SELECT (a,b) BETWEEN (1,1,1) AND (2,2) FROM t"),
        ("v6", "SELECT 1=(a,b) FROM t"),
        ("v7", "SELECT 1 BETWEEN (a,b) AND 3 FROM t"),
        ("v8", "SELECT 1 BETWEEN 0 AND (a,b) FROM t"),
        ("v9", "SELECT (a,b) BETWEEN (1,1) AND (2,2,2) FROM t"),
    ] {
        let sql = alloc::format!("CREATE VIEW {name} AS {body}");
        writer.run(sql.as_bytes()).unwrap();
        assert_eq!(
            refusal(&writer, &alloc::format!("SELECT * FROM {name}")),
            Error::Eval(crate::eval::Error::RowValue),
            "{body}"
        );
    }
}
