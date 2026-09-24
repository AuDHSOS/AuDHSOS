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
        "SELECT (1,2) BETWEEN 1 AND 2",
        "SELECT (1,2) BETWEEN (1,1) AND 3",
        "SELECT (1,2) BETWEEN (1,1,1) AND (2,2)",
        "SELECT c FROM t WHERE (a,b)<=1",
        "SELECT (1,2) IS 1",
        // A statement of more than one column stands as a row, so a
        // comparison against one value is a misuse of a row.
        "SELECT 1=(SELECT 1,2)",
        "SELECT (SELECT 1,2)<(SELECT 1,2,3)",
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
    // A member of an `IN` list that holds another number of values than
    // the row looked for names both counts, and one value is a term and
    // not terms.
    for (sql, message) in [
        (
            "SELECT (1,2) IN (1,2)",
            "IN(...) element has 1 term - expected 2",
        ),
        (
            "SELECT (1,2) IN ((1,2),(3,4,5))",
            "IN(...) element has 3 terms - expected 2",
        ),
        (
            "SELECT (1,2,3) IN ((1,2),(3,4))",
            "IN(...) element has 2 terms - expected 3",
        ),
    ] {
        assert_eq!(refusal(&writer, sql).message(), message, "{sql}");
    }
}

#[test]
fn a_statement_used_as_a_value_answers_one_column() {
    let writer = connection();
    for sql in ["SELECT (SELECT 1,2)", "SELECT (SELECT 1,2)+1"] {
        assert_eq!(
            refusal(&writer, sql).message(),
            "sub-select returns 2 columns - expected 1",
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
        crate::eval::evaluate(&arena, root, sql, None),
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
    writer
        .run(b"CREATE VIEW v4 AS SELECT (a,b) IN ((1,2,3)) FROM t")
        .unwrap();
    assert_eq!(
        refusal(&writer, "SELECT * FROM v4").message(),
        "IN(...) element has 3 terms - expected 2"
    );
}

/// A statement of more than one column stands as a row, which the C
/// library answers `1` for where the rows are alike.
#[test]
fn what_a_statement_of_more_than_one_column_stands_as() {
    let writer = connection();
    // A comparison of two statements, of a statement against a row, and
    // of a statement against a row the other way round.
    assert_eq!(answered(&writer, "SELECT (SELECT 1,2)=(SELECT 1,2)"), ["1"]);
    assert_eq!(answered(&writer, "SELECT (SELECT 1,2)=(1,2)"), ["1"]);
    assert_eq!(answered(&writer, "SELECT (1,2)=(SELECT 1,3)"), ["0"]);
    assert_eq!(answered(&writer, "SELECT (SELECT 1,2)<(SELECT 1,3)"), ["1"]);
    assert_eq!(
        answered(&writer, "SELECT (SELECT 1,NULL) IS (SELECT 1,NULL)"),
        ["1"]
    );
    // A statement that answers no row stands for a row of nulls, which
    // no comparison settles and `IS` answers for.
    assert_eq!(
        answered(&writer, "SELECT ((SELECT x,y FROM empty)=(1,2)) IS NULL"),
        ["1"]
    );
    // A statement of one column stands for one value, which carries the
    // collation of the sides it is compared under.
    assert_eq!(answered(&writer, "SELECT (SELECT 1)=1"), ["1"]);
    // A `COLLATE` on either side is the collation the comparison reads
    // under.
    assert_eq!(
        answered(&writer, "SELECT ('A' COLLATE nocase)=(SELECT 'a')"),
        ["1"]
    );
    assert_eq!(
        answered(&writer, "SELECT (SELECT 'a')=('A' COLLATE nocase)"),
        ["1"]
    );
    assert_eq!(answered(&writer, "SELECT (SELECT 'a')='A'"), ["0"]);
    // A `BETWEEN` reads a statement as a row as well.
    assert_eq!(
        answered(&writer, "SELECT (SELECT 2,2) BETWEEN (2,2) AND (3,3)"),
        ["1"]
    );
    assert_eq!(
        answered(&writer, "SELECT (SELECT 1) BETWEEN 0 AND 2"),
        ["1"]
    );
    // An `IN` over a statement reads the left side as a row of as many
    // values as the right side answers columns.
    assert_eq!(
        answered(&writer, "SELECT (SELECT 1,0) IN (SELECT x,y FROM u)"),
        ["1"]
    );
    assert_eq!(
        answered(&writer, "SELECT (SELECT 2,0) IN (SELECT x,y FROM u)"),
        ["0"]
    );
    // A `CASE` compares its operand against every `WHEN` as a row.
    assert_eq!(
        answered(&writer, "SELECT CASE (2,2) WHEN (1,1) THEN 2 ELSE 1 END"),
        ["1"]
    );
    assert_eq!(
        answered(
            &writer,
            "SELECT CASE (2,2) WHEN (1,1) THEN 2 WHEN (2,2) THEN 3 END"
        ),
        ["3"]
    );
    assert_eq!(
        answered(
            &writer,
            "SELECT CASE (SELECT 2,2) WHEN (2,2) THEN 2 ELSE 1 END"
        ),
        ["2"]
    );
    assert_eq!(answered(&writer, "SELECT CASE 2 WHEN 2 THEN 3 END"), ["3"]);
    assert_eq!(
        answered(&writer, "SELECT CASE WHEN 1 THEN 4 ELSE 5 END"),
        ["4"]
    );
    // A row against a statement of another width, and a row under an
    // operator that compares nothing, are both a misuse of a row.
    for sql in [
        "SELECT (SELECT 1,2) BETWEEN (1,1,1) AND (3,3,3)",
        "SELECT (SELECT 1) BETWEEN (1,2) AND (3,4)",
        "SELECT CASE (2,2) WHEN (1,1,1) THEN 2 ELSE 1 END",
        "SELECT CASE (2,2) WHEN 1 THEN 2 ELSE 1 END",
        "SELECT CASE 1 WHEN (1,1) THEN 2 ELSE 1 END",
        "SELECT CASE (SELECT 2,2) WHEN (1,1,1) THEN 2 ELSE 1 END",
    ] {
        assert_eq!(
            refusal(&writer, sql).message(),
            "row value misused",
            "{sql}"
        );
    }
    // `X IN (a, b)` reads a statement written for `X` as one value.
    assert_eq!(
        refusal(&writer, "SELECT (SELECT 1,2) IN (1,2)").message(),
        "sub-select returns 2 columns - expected 1"
    );
}

/// `SET (a, b) = value` writes one column of the row the value answers.
#[test]
fn what_a_set_of_more_than_one_column_writes() {
    let mut writer = connection();
    // A row of values, a statement, and a statement that answers no
    // row, which writes a null per column.
    writer.run(b"UPDATE t SET (a,b)=(10,20) WHERE c=3").unwrap();
    writer
        .run(b"UPDATE t SET (a,b)=(SELECT x,y FROM u WHERE x=9) WHERE c=2")
        .unwrap();
    writer
        .run(b"UPDATE t SET (a,b)=(SELECT x,y FROM empty) WHERE c=1")
        .unwrap();
    // One column in brackets, a clause of one column beside one of
    // more, and two clauses of more than one column.
    writer.run(b"UPDATE t SET (a)=(7), c=8 WHERE c=0").unwrap();
    assert_eq!(
        answered(&writer, "SELECT a,b,c FROM t ORDER BY c"),
        [
            "NULL", "NULL", "1", "9", "9", "2", "10", "20", "3", "7", "0", "8"
        ]
    );
    // A statement whose width cannot be counted before it runs is left
    // alone, and writes as many columns as the clause names.
    writer
        .run(b"UPDATE t SET (a,b)=(SELECT * FROM u WHERE x=9), (c)=(4) WHERE c=8")
        .unwrap();
    assert_eq!(
        answered(&writer, "SELECT a,b,c FROM t WHERE c=4"),
        ["9", "9", "4"]
    );
    // A clause that writes another number of columns than the value
    // holds names both counts.
    for sql in [
        b"UPDATE t SET (a,b)=(1,2,3)".as_slice(),
        b"UPDATE t SET (a,b)=(SELECT 1,2,3)",
    ] {
        assert_eq!(
            writer.run(sql).expect_err("a refusal").message(),
            "2 columns assigned 3 values",
            "{}",
            String::from_utf8_lossy(sql)
        );
    }
}

/// What a statement used as a value is refused with, which is the
/// refusal of the statement it holds: `sqlite3ExprCodeSubselect` builds
/// the rows of such a statement into the program of the statement that
/// holds it, so one `OP_Halt` ends both.
#[test]
fn what_a_statement_used_as_a_value_is_refused_with() {
    let writer = connection();
    // A refusal of the reader is carried out under the message and the
    // code that refusal names.
    for sql in [
        "SELECT (SELECT a FROM nope)",
        "SELECT EXISTS(SELECT a FROM nope)",
        "SELECT (1,2)=(SELECT a,b FROM nope)",
        "SELECT 1 IN (SELECT a FROM nope)",
        "SELECT 1 IN nope",
    ] {
        let held = refusal(&writer, sql);
        assert_eq!(held.message(), "no such table: nope", "{sql}");
        assert_eq!(held.code().number, 1, "{sql}");
        assert_eq!(held.code().extended_name, b"SQLITE_ERROR", "{sql}");
    }
    // A refusal the walk of an expression raised itself stands as it is.
    let held = refusal(&writer, "SELECT (SELECT nope)");
    assert_eq!(held.message(), "no such column: nope");
}
