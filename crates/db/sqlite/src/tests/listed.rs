// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The pragmas that answer the schema: the columns of a table, the places
//! of an index, the indexes of a table and the collations a connection
//! holds.

use alloc::vec::Vec;

use crate::change::Writer;
use crate::header::Encoding;
use crate::value::{Collating, Value};

/// One collation the application defined, which
/// `PRAGMA collation_list` answers after the three of the library.
fn backwards(_name: &'static [u8], left: &[u8], right: &[u8]) -> core::cmp::Ordering {
    let one: Vec<u8> = left.iter().rev().copied().collect();
    let another: Vec<u8> = right.iter().rev().copied().collect();
    one.cmp(&another)
}

/// The one, as a connection holds it.
static COLLATING: &[Collating] = &[Collating {
    name: b"BACKWARDS",
    by: backwards,
}];

/// The rows one pragma answers, written out as one text.
fn shown(writer: &mut Writer, sql: &[u8]) -> alloc::string::String {
    let mut out = alloc::string::String::new();
    for row in writer.run(sql).expect("rows") {
        for value in row {
            out.push_str(&alloc::string::String::from_utf8_lossy(
                &value.text().unwrap_or_default(),
            ));
            out.push('|');
        }
    }
    out
}

/// `PRAGMA table_info` answers one row per column the table holds, and
/// `PRAGMA table_xinfo` the computed columns beside them.
#[test]
fn what_the_columns_of_a_table_answer() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a INTEGER PRIMARY KEY, b TEXT NOT NULL DEFAULT 'x', c)")
        .unwrap();
    assert_eq!(
        shown(&mut writer, b"PRAGMA table_info(t)"),
        "0|a|INTEGER|0||1|1|b|TEXT|1|'x'|0|2|c||0||0|"
    );
    // A computed column is out of the first and in the second, with two
    // for `VIRTUAL` and three for `STORED`.
    writer
        .run(b"CREATE TABLE g(a, b AS (a+1), c GENERATED ALWAYS AS (a*2) STORED)")
        .unwrap();
    assert_eq!(shown(&mut writer, b"PRAGMA table_info(g)"), "0|a||0||0|");
    assert_eq!(
        shown(&mut writer, b"PRAGMA table_xinfo(g)"),
        "0|a||0||0|0|1|b||0||0|2|2|c||0||0|3|"
    );
    // The place in the primary key counts from one, in the order the key
    // holds the columns.
    writer
        .run(b"CREATE TABLE k(a,b,PRIMARY KEY(b,a)) WITHOUT ROWID")
        .unwrap();
    assert_eq!(
        shown(&mut writer, b"PRAGMA table_info(k)"),
        "0|a||1||2|1|b||1||1|"
    );
    // A table the schema does not hold answers no row.
    assert!(shown(&mut writer, b"PRAGMA table_info(nosuch)").is_empty());
}

/// `PRAGMA index_info` answers one row per place of the index, and
/// `PRAGMA index_xinfo` the places an entry carries to name the row
/// beside them.
#[test]
fn what_the_places_of_an_index_answer() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a INTEGER PRIMARY KEY, b TEXT COLLATE NOCASE, c)")
        .unwrap();
    writer
        .run(b"CREATE INDEX i ON t(b DESC, c) WHERE c>0")
        .unwrap();
    assert_eq!(shown(&mut writer, b"PRAGMA index_info(i)"), "0|1|b|1|2|c|");
    assert_eq!(
        shown(&mut writer, b"PRAGMA index_xinfo(i)"),
        "0|1|b|1|NOCASE|1|1|2|c|0|BINARY|1|2|-1||0|BINARY|0|"
    );
    // A place over an expression holds no column of the table.
    writer.run(b"CREATE INDEX j ON t(b||c)").unwrap();
    assert_eq!(shown(&mut writer, b"PRAGMA index_info(j)"), "0|-2||");
    // An index of a table that keeps its rows in the key's own tree
    // carries the key, which names the row.
    writer
        .run(b"CREATE TABLE w(a,b,c,PRIMARY KEY(b,a)) WITHOUT ROWID")
        .unwrap();
    writer.run(b"CREATE INDEX wi ON w(c)").unwrap();
    assert_eq!(
        shown(&mut writer, b"PRAGMA index_xinfo(wi)"),
        "0|2|c|0|BINARY|1|1|1|b|0|BINARY|0|2|0|a|0|BINARY|0|"
    );
    // An index the schema does not hold answers no row.
    assert!(shown(&mut writer, b"PRAGMA index_info(nosuch)").is_empty());
}

/// `PRAGMA index_list` answers one row per index over the table, the one
/// made last first, and `PRAGMA collation_list` one row per collation.
#[test]
fn what_the_indexes_of_a_table_answer() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.collates(COLLATING);
    writer
        .run(b"CREATE TABLE t(a TEXT PRIMARY KEY, b UNIQUE, c)")
        .unwrap();
    writer.run(b"CREATE INDEX i ON t(c) WHERE c>0").unwrap();
    assert_eq!(
        shown(&mut writer, b"PRAGMA index_list(t)"),
        "0|i|0|c|1|1|sqlite_autoindex_t_2|1|u|0|2|sqlite_autoindex_t_1|1|pk|0|"
    );
    // The index a `PRIMARY KEY` made carries the rowid to name the row.
    assert_eq!(
        shown(&mut writer, b"PRAGMA index_xinfo(sqlite_autoindex_t_1)"),
        "0|0|a|0|BINARY|1|1|-1||0|BINARY|0|"
    );
    // A table the schema does not hold answers no row.
    assert!(shown(&mut writer, b"PRAGMA index_list(nosuch)").is_empty());
    // A place the index compares under a collation the application
    // defined carries the name that collation was defined under.
    writer
        .run(b"CREATE INDEX ib ON t(c COLLATE BACKWARDS)")
        .unwrap();
    assert_eq!(
        shown(&mut writer, b"PRAGMA index_xinfo(ib)"),
        "0|2|c|0|BACKWARDS|1|1|-1||0|BINARY|0|"
    );
    // The three collations of the library come first, and the ones the
    // application defined after them.
    assert_eq!(
        shown(&mut writer, b"PRAGMA collation_list"),
        "0|BINARY|1|NOCASE|2|RTRIM|3|BACKWARDS|"
    );
    assert_eq!(
        writer.run(b"PRAGMA table_info(t)").unwrap().first(),
        Some(&alloc::vec![
            Value::Int(0),
            Value::Text(b"a".to_vec()),
            Value::Text(b"TEXT".to_vec()),
            Value::Int(0),
            Value::Null,
            Value::Int(1),
        ])
    );
}
