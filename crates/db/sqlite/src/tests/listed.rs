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

/// `PRAGMA compile_options` answers one row per option the library was
/// built with.
#[test]
fn what_options_the_build_holds() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    assert_eq!(
        shown(&mut writer, b"PRAGMA compile_options"),
        "ENABLE_URI_00_ERROR|THREADSAFE=0|"
    );
}

/// The columns a pragma answers, which a caller reads the shape of the
/// statement from and a reader answers the rows of.
#[test]
fn what_columns_a_pragma_answers() {
    use crate::pragma::{Setting, of_name};
    let named = |sql: &str, valued: bool| {
        let name = sql.as_bytes();
        of_name(name)
            .map(|setting| setting.columns(name, valued))
            .unwrap_or_default()
            .iter()
            .map(|column| alloc::string::String::from_utf8_lossy(column).into_owned())
            .collect::<Vec<alloc::string::String>>()
            .join("|")
    };
    // A pragma of columns of its own names each of them, whether a value
    // follows the name or not.
    assert_eq!(
        named("table_info", false),
        "cid|name|type|notnull|dflt_value|pk"
    );
    assert_eq!(
        named("table_info", true),
        "cid|name|type|notnull|dflt_value|pk"
    );
    assert_eq!(
        named("table_xinfo", false),
        "cid|name|type|notnull|dflt_value|pk|hidden"
    );
    assert_eq!(named("index_info", false), "seqno|cid|name");
    assert_eq!(named("index_xinfo", false), "seqno|cid|name|desc|coll|key");
    assert_eq!(named("index_list", false), "seq|name|unique|origin|partial");
    assert_eq!(named("collation_list", false), "seq|name");
    assert_eq!(named("compile_options", false), "compile_options");
    assert_eq!(named("database_list", false), "seq|name|file");
    assert_eq!(
        named("foreign_key_list", false),
        "id|seq|table|from|to|on_update|on_delete|match"
    );
    assert_eq!(named("foreign_key_check", false), "table|rowid|parent|fkid");
    assert_eq!(named("wal_checkpoint", false), "busy|log|checkpointed");
    // A pragma of one value names the column after itself, and names none
    // where a value follows the name and `PragFlg_NoColumns1` stands
    // against it.
    assert_eq!(named("user_version", false), "user_version");
    assert_eq!(named("user_version", true), "");
    assert_eq!(named("page_size", true), "");
    assert_eq!(named("encoding", true), "");
    assert_eq!(named("auto_vacuum", true), "");
    assert_eq!(named("schema_version", true), "");
    assert_eq!(named("application_id", true), "");
    assert_eq!(named("schema_format", true), "");
    assert_eq!(named("count_changes", true), "");
    assert_eq!(named("default_cache_size", true), "");
    assert_eq!(named("foreign_keys", true), "");
    assert_eq!(named("cache_spill", true), "");
    assert_eq!(named("journal_mode", true), "journal_mode");
    assert_eq!(named("integrity_check", true), "integrity_check");
    assert_eq!(named("quick_check", false), "quick_check");
    assert_eq!(named("page_count", true), "page_count");
    assert_eq!(named("freelist_count", false), "freelist_count");
    assert_eq!(named("cell_size_check", false), "cell_size_check");
    // `PragFlg_NoColumns` names none either way, and a name no version of
    // the library still holds names none.
    assert_eq!(named("case_sensitive_like", false), "");
    assert_eq!(named("shrink_memory", false), "");
    assert_eq!(named("legacy_file_format", false), "");
    assert_eq!(named("default_synchronous", false), "");
    assert_eq!(named("optimize", false), "optimize");
    // A name that is no pragma names no column.
    assert_eq!(named("nonesuch", false), "");
    assert!(!Setting::Ignored.columns(b"optimize", true).is_empty());
}

/// A reader answers the pragmas of the schema out of the database it was
/// opened over, with the columns each names.
#[test]
fn what_a_reader_answers_for_a_pragma_of_the_schema() {
    let mut writer = Writer::new(1024, 0, Encoding::Utf8).unwrap();
    writer.collates(COLLATING);
    writer.run(b"CREATE TABLE t(a, b TEXT)").unwrap();
    writer.run(b"CREATE INDEX i ON t(b)").unwrap();
    let bytes = writer.written();
    let database = crate::db::Database::open_collating(&bytes, COLLATING).unwrap();
    let answered = |sql: &[u8]| {
        let answered = database.query(sql).expect("rows");
        let names = answered
            .names
            .iter()
            .map(|name| alloc::string::String::from_utf8_lossy(name).into_owned())
            .collect::<Vec<alloc::string::String>>()
            .join("|");
        let rows = answered
            .rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|value| {
                        alloc::string::String::from_utf8_lossy(&value.text().unwrap_or_default())
                            .into_owned()
                    })
                    .collect::<Vec<alloc::string::String>>()
                    .join(",")
            })
            .collect::<Vec<alloc::string::String>>()
            .join("/");
        alloc::format!("{names} {rows}")
    };
    assert_eq!(
        answered(b"PRAGMA table_info(t)"),
        "cid|name|type|notnull|dflt_value|pk 0,a,,0,,0/1,b,TEXT,0,,0"
    );
    assert_eq!(
        answered(b"PRAGMA table_xinfo(t)"),
        "cid|name|type|notnull|dflt_value|pk|hidden 0,a,,0,,0,0/1,b,TEXT,0,,0,0"
    );
    assert_eq!(answered(b"PRAGMA index_info(i)"), "seqno|cid|name 0,1,b");
    assert_eq!(
        answered(b"PRAGMA index_xinfo(i)"),
        "seqno|cid|name|desc|coll|key 0,1,b,0,BINARY,1/1,-1,,0,BINARY,0"
    );
    assert_eq!(
        answered(b"PRAGMA index_list(t)"),
        "seq|name|unique|origin|partial 0,i,0,c,0"
    );
    assert_eq!(
        answered(b"PRAGMA collation_list"),
        "seq|name 0,BINARY/1,NOCASE/2,RTRIM/3,BACKWARDS"
    );
    assert_eq!(
        answered(b"PRAGMA compile_options"),
        "compile_options ENABLE_URI_00_ERROR/THREADSAFE=0"
    );
    // A pragma of one value names its own column, and one the reader
    // holds no value for is refused.
    assert_eq!(answered(b"PRAGMA page_size"), "page_size 1024");
    assert!(database.query(b"PRAGMA cache_spill").is_err());
}
