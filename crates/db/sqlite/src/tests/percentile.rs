// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `median`, `percentile`, `percentile_cont` and `percentile_disc`,
//! against what the shell answers for the same statements.

use alloc::string::String;

use crate::change::Writer;
use crate::db::Database;
use crate::header::Encoding;

/// A connection over the nine values `percentile.test` reads.
fn writer() -> Writer {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(x)").unwrap();
    writer
        .run(b"INSERT INTO t1 VALUES(1),(4),(6),(7),(8),(9),(11),(11),(11)")
        .unwrap();
    writer
}

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

/// What each of the four answers for the same values, which is
/// `percentCompute`: `percentile_disc` answers a value the group holds
/// and the other three answer one between two of them.
#[test]
fn the_four_percentile_aggregates_answer_what_the_c_library_answers() {
    let writer = writer();
    for (sql, answer) in [
        (b"SELECT percentile(x,0) FROM t1".as_slice(), "1.0|"),
        (b"SELECT percentile(x,100) FROM t1", "11.0|"),
        (b"SELECT percentile(x,50) FROM t1", "8.0|"),
        (b"SELECT percentile(x,12.5) FROM t1", "4.0|"),
        (b"SELECT percentile(x,15) FROM t1", "4.4|"),
        (b"SELECT percentile(x,20) FROM t1", "5.2|"),
        (b"SELECT percentile_cont(x,0.15) FROM t1", "4.4|"),
        (b"SELECT percentile_disc(x,0.15) FROM t1", "4.0|"),
        (b"SELECT percentile_disc(x,0.2) FROM t1", "4.0|"),
        (b"SELECT median(x) FROM t1", "8.0|"),
        (b"SELECT median(DISTINCT x) FROM t1", "7.0|"),
    ] {
        assert_eq!(shown(&writer, sql), answer, "{sql:?}");
    }
}

/// `f(...) WITHIN GROUP (ORDER BY Y)` is `f(Y,...)`, which is
/// `sqlite3ExprAddFunctionOrderBy`.
#[test]
fn an_ordered_set_aggregate_reads_the_term_of_its_clause_as_its_first_value() {
    let writer = writer();
    for (sql, answer) in [
        (
            b"SELECT percentile(15) WITHIN GROUP (ORDER BY x) FROM t1".as_slice(),
            "4.4|",
        ),
        (
            b"SELECT percentile_cont(0.15) WITHIN GROUP (ORDER BY x DESC) FROM t1",
            "4.4|",
        ),
        (
            b"SELECT percentile_disc(0.15) WITHIN GROUP (ORDER BY x) FROM t1",
            "4.0|",
        ),
        (
            b"SELECT median() WITHIN GROUP (ORDER BY x ASC) FROM t1",
            "8.0|",
        ),
        (b"SELECT median() WITHIN GROUP (ORDER BY x) FROM t1", "8.0|"),
    ] {
        assert_eq!(shown(&writer, sql), answer, "{sql:?}");
    }
}

/// What a percentile aggregate refuses.
#[test]
fn what_a_percentile_aggregate_refuses() {
    let mut writer = writer();
    let database = |writer: &Writer| writer.written();
    for (sql, message) in [
        (
            b"SELECT percentile(x,null) FROM t1".as_slice(),
            "the fraction argument to percentile() is not between 0.0 and 100.0",
        ),
        (
            b"SELECT percentile(x,'fifty') FROM t1",
            "the fraction argument to percentile() is not between 0.0 and 100.0",
        ),
        (
            b"SELECT percentile(x,100.0000001) FROM t1",
            "the fraction argument to percentile() is not between 0.0 and 100.0",
        ),
        (
            b"SELECT percentile_cont(x,1.0000001) FROM t1",
            "the fraction argument to percentile_cont() is not between 0.0 and 1.0",
        ),
        (
            b"SELECT percentile(x, 15+0.1*rowid) FROM t1",
            "the fraction argument to percentile() is not the same for all input rows",
        ),
        (
            b"SELECT percentile(x) FROM t1",
            "wrong number of arguments to function percentile()",
        ),
        (
            b"SELECT median(x,0) FROM t1",
            "wrong number of arguments to function median()",
        ),
        (
            b"SELECT percentile(DISTINCT 50) WITHIN GROUP (ORDER BY x) FROM t1",
            "DISTINCT not allowed on ordered-set aggregate percentile()",
        ),
    ] {
        let image = database(&writer);
        let held = Database::open(&image).unwrap();
        assert_eq!(held.query(sql).unwrap_err().message(), message, "{sql:?}");
    }
    // A value that is neither nothing nor a number, and an infinity.
    writer.run(b"INSERT INTO t1 VALUES('50')").unwrap();
    let image = database(&writer);
    let held = Database::open(&image).unwrap();
    assert_eq!(
        held.query(b"SELECT percentile(x,50) FROM t1")
            .unwrap_err()
            .message(),
        "input to percentile() is not numeric"
    );
    writer.run(b"DELETE FROM t1 WHERE x='50'").unwrap();
    writer
        .run(b"INSERT INTO t1 VALUES(1.0e300*1.0e300)")
        .unwrap();
    let image = database(&writer);
    let held = Database::open(&image).unwrap();
    assert_eq!(
        held.query(b"SELECT median(x) FROM t1")
            .unwrap_err()
            .message(),
        "Inf input to median()"
    );
}

/// A group of no values answers nothing, and a group of one answers
/// that value whatever the fraction says.
#[test]
fn what_a_percentile_aggregate_answers_for_no_values_and_for_one() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t1(x)").unwrap();
    writer.run(b"INSERT INTO t1 VALUES(NULL),(NULL)").unwrap();
    assert_eq!(
        shown(&writer, b"SELECT ifnull(percentile(x,50),'none') FROM t1"),
        "none|"
    );
    writer.run(b"INSERT INTO t1 VALUES(12345)").unwrap();
    assert_eq!(
        shown(
            &writer,
            b"SELECT percentile(x,0), percentile(x,50), percentile(x,100) FROM t1"
        ),
        "12345.0|12345.0|12345.0|"
    );
}
