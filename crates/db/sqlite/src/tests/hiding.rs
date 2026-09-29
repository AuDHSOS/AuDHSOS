// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Columns a name beginning with `__hidden__` hides, which
//! `SQLITE_ENABLE_HIDDEN_COLUMNS` of the C library turns on and this
//! engine holds always.

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

/// A `*` leaves a hidden column out and a statement that names no column
/// writes no value into one, where a name reaches it.
#[test]
fn which_columns_a_star_answers_where_one_of_them_is_hidden() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(__hidden__a, b)".as_slice(),
        b"INSERT INTO t1 VALUES('1')",
        b"INSERT INTO t1(__hidden__a, b) VALUES('x', 'y')",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(shown(&writer, b"SELECT * FROM t1"), "1|y|");
    assert_eq!(
        shown(&writer, b"SELECT __hidden__a, * FROM t1"),
        "NULL|1|x|y|"
    );
    assert_eq!(shown(&writer, b"SELECT t1.* FROM t1"), "1|y|");
    // A number of a `GROUP BY` counts the columns the `*` answers, of
    // which the hidden one is none.
    assert_eq!(shown(&writer, b"SELECT * FROM t1 GROUP BY 1"), "1|y|");
    assert_eq!(shown(&writer, b"SELECT t1.* FROM t1 GROUP BY 1"), "1|y|");
    let image = writer.written();
    let database = Database::open(&image).unwrap();
    assert_eq!(
        database
            .query(b"SELECT * FROM t1 GROUP BY 2")
            .unwrap_err()
            .message(),
        "1st GROUP BY term out of range - should be between 1 and 1"
    );
}

/// `PRAGMA table_info` leaves a hidden column out and `PRAGMA
/// table_xinfo` writes it with a one, where a computed column carries a
/// two or a three.
#[test]
fn what_the_pragmas_of_a_table_answer_for_a_hidden_column() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a, __hidden__b, c AS (a+1) VIRTUAL)")
        .unwrap();
    assert_eq!(shown(&writer, b"PRAGMA table_info(t)"), "0|a||0|NULL|0|");
    assert_eq!(
        shown(&writer, b"PRAGMA table_xinfo(t)"),
        "0|a||0|NULL|0|0|1|__hidden__b||0|NULL|0|1|2|c||0|NULL|0|2|"
    );
}

/// The column of a view a name beginning with `__hidden__` hides, which
/// the column list of the view or an alias of its statement names.
#[test]
fn which_columns_a_star_over_a_view_answers() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE x1(a, b, c)".as_slice(),
        b"INSERT INTO x1 VALUES(1, 2, 3)",
        b"CREATE VIEW v1(a, b, __hidden__c) AS SELECT a, b, c FROM x1",
        b"CREATE VIEW v2 AS SELECT a, b, c AS __hidden__c FROM x1",
    ] {
        writer.run(sql).unwrap();
    }
    for name in [b"v1".as_slice(), b"v2"] {
        let mut sql = b"SELECT * FROM ".to_vec();
        sql.extend_from_slice(name);
        assert_eq!(shown(&writer, &sql), "1|2|");
        let mut named = b"SELECT a, b, __hidden__c FROM ".to_vec();
        named.extend_from_slice(name);
        assert_eq!(shown(&writer, &named), "1|2|3|");
    }
}

/// A statement that writes the rows another answers fills the columns
/// the `*` of that statement answers, so a hidden column of the table
/// written into holds what it falls back to.
#[test]
fn which_columns_a_statement_that_reads_a_statement_writes() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t4(a, __hidden__b, c)".as_slice(),
        b"INSERT INTO t4 SELECT 1, 2",
        b"CREATE TABLE t5(__hidden__a, b, c)",
        b"CREATE TABLE t6(__hidden__a, b, c)",
        b"INSERT INTO t6(__hidden__a, b, c) VALUES(1, 2, 3)",
        b"INSERT INTO t5 SELECT * FROM t6",
    ] {
        writer.run(sql).unwrap();
    }
    assert_eq!(
        shown(&writer, b"SELECT a, __hidden__b, c FROM t4"),
        "1|NULL|2|"
    );
    assert_eq!(shown(&writer, b"SELECT * FROM t5"), "2|3|");
    assert_eq!(shown(&writer, b"SELECT __hidden__a FROM t5"), "NULL|");
}
