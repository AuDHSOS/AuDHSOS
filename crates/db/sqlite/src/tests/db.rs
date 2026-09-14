// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::db`, against the answers SQLite gives for the same
//! statements over the same files.
//!
//! `fixtures/query.corpus` is a fixture and a statement per line;
//! `fixtures/query.golden` is the columns SQLite named and the rows it
//! answered, each value quoted, written by `tools/sqlite-oracle.c`. The
//! comparison is the name of every column and the value of every field.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use crate::db::Database;
use crate::func::{Function, call};
use crate::value::{Collation, Value};

/// The cases: which file, and what to ask it.
fn corpus() -> Vec<(&'static str, &'static str)> {
    include_str!("fixtures/query.corpus")
        .lines()
        .map(|line| line.split_once('|').unwrap_or((line, "")))
        .collect()
}

/// What SQLite answered.
fn golden() -> Vec<&'static str> {
    include_str!("fixtures/query.golden").lines().collect()
}

/// The bytes of a fixture, by name.
fn fixture(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "small.db" => super::SMALL,
        "page512.db" => super::PAGE512,
        "utf16.db" => super::UTF16,
        "overflow.db" => super::OVERFLOW,
        "indexed.db" => super::INDEXED,
        "keys.db" => super::KEYS,
        "generated.db" => super::GENERATED,
        "wide16.db" => super::WIDE16,
        "joins.db" => super::JOINS,
        "format1.db" => super::FORMAT1,
        "format3.db" => super::FORMAT3,
        "format4.db" => super::FORMAT4,
        "defaults.db" => super::DEFAULTS,
        "affinity.db" => super::AFFINITY,
        _ => return super::matrix::fixture(name),
    })
}

/// One value as `quote` writes it.
fn quoted(value: &Value) -> String {
    let answer = call(
        Function::Quote,
        core::slice::from_ref(value),
        Collation::Binary,
        crate::header::Encoding::Utf8,
        None,
    )
    .expect("a function that always answers");
    String::from_utf8_lossy(&answer.text().unwrap_or_default()).into_owned()
}

/// What this engine answers, as the oracle would have written it.
fn answer(bytes: &[u8], sql: &str) -> Option<String> {
    let database = Database::open(bytes).ok()?;
    let answered = database.query(sql.as_bytes()).ok()?;
    let mut out = String::from("N");
    for name in &answered.names {
        out.push('|');
        out.push_str(&String::from_utf8_lossy(name));
    }
    for row in &answered.rows {
        out.push_str("\tR");
        for value in row {
            out.push('|');
            out.push_str(&quoted(value));
        }
    }
    Some(out)
}

#[test]
fn every_statement_answers_what_the_c_library_answers() {
    let cases = corpus();
    let answers = golden();
    assert_eq!(cases.len(), answers.len());
    for ((name, sql), theirs) in cases.iter().zip(answers) {
        let bytes = fixture(name).unwrap_or_else(|| panic!("no fixture {name}"));
        let mine = answer(bytes, sql);
        // A refusal before the first row is a line of its own; one
        // after some rows is a field at the end of the line. Either way
        // this engine answers the whole statement or none of it.
        if theirs.starts_with('!') || theirs.contains("\t!") {
            assert!(mine.is_none(), "{sql} over {name} is refused by SQLite");
            continue;
        }
        let mine = mine.unwrap_or_else(|| panic!("{sql} over {name} is refused"));
        assert_eq!(mine, theirs, "{sql} over {name}");
    }
}

#[test]
fn a_file_that_is_not_a_database_is_refused_as_one() {
    use crate::db::Error;
    use crate::error;
    assert_eq!(
        Database::open(b"not a database at all").unwrap_err(),
        Error::Image(error::Error::Truncated)
    );
    let mut bytes = super::SMALL.to_vec();
    bytes[0] = b'X';
    assert_eq!(
        Database::open(&bytes).unwrap_err(),
        Error::Image(error::Error::Magic)
    );
}

#[test]
fn a_statement_that_is_not_one_is_refused_as_one() {
    use crate::db::Error;
    let database = Database::open(super::SMALL).expect("a database");
    assert!(matches!(
        database.query(b"SELECT FROM").unwrap_err(),
        Error::Parse(_)
    ));
    assert_eq!(database.query(b"SELECT * FROM nosuch"), Err(Error::NoTable));
    assert_eq!(
        database.query(b"SELECT a FROM t ORDER BY 9"),
        Err(Error::OrderRange)
    );
}

#[test]
fn a_schema_this_crate_cannot_read_is_refused() {
    use crate::db::Error;
    use crate::schema;
    // The schema of a file is text, and a file is not obliged to hold
    // text this crate can make a table of. One byte makes the small
    // fixture's two columns one name.
    let mut bytes = super::SMALL.to_vec();
    let at = bytes
        .windows(4)
        .position(|window| window == b"b TE")
        .expect("the column in the schema");
    bytes[at] = b'a';
    assert_eq!(
        Database::open(&bytes).unwrap_err(),
        Error::Schema(schema::Error::DuplicateColumn)
    );
}

#[test]
fn the_tables_of_a_file_are_the_ones_its_schema_names() {
    let database = Database::open(super::INDEXED).expect("a database");
    let names: Vec<String> = database
        .tables()
        .map(|table| String::from_utf8_lossy(&table.name).into_owned())
        .collect();
    // The indexes of the fixture are not tables, and are skipped.
    assert_eq!(names, ["k", "m", "e", "o", "u", "f"]);
    let database = Database::open(super::KEYS).expect("a database");
    let names: Vec<String> = database
        .tables()
        .map(|table| String::from_utf8_lossy(&table.name).into_owned())
        .collect();
    assert_eq!(names, ["r", "w", "d", "u", "p", "q"]);
}

/// A database whose schema table holds exactly `records`, built by hand
/// so that a row the shell would never write can be put in front of the
/// reader.
fn schema_file(records: &[Vec<u8>]) -> Vec<u8> {
    const PAGE: usize = 4096;
    let mut bytes = super::SMALL[..100].to_vec();
    bytes.resize(PAGE, 0);
    bytes[28..32].copy_from_slice(&1u32.to_be_bytes());
    let mut cells = Vec::new();
    let mut offset = PAGE;
    for (at, record) in records.iter().enumerate() {
        let (len, len_bytes) = super::varint(record.len() as u64);
        let (rowid, rowid_bytes) = super::varint(at as u64 + 1);
        let mut cell = Vec::new();
        cell.extend_from_slice(&len[..len_bytes]);
        cell.extend_from_slice(&rowid[..rowid_bytes]);
        cell.extend_from_slice(record);
        offset -= cell.len();
        bytes[offset..offset + cell.len()].copy_from_slice(&cell);
        cells.push(offset);
    }
    bytes[100] = 13;
    bytes[103..105].copy_from_slice(&(cells.len() as u16).to_be_bytes());
    bytes[105..107].copy_from_slice(&(offset as u16).to_be_bytes());
    for (at, start) in cells.iter().enumerate() {
        let slot = 108 + at * 2;
        bytes[slot..slot + 2].copy_from_slice(&(*start as u16).to_be_bytes());
    }
    bytes
}

/// One record of five values, each either text or a small integer.
fn record(values: [Result<&str, i64>; 5]) -> Vec<u8> {
    let mut serials = Vec::new();
    let mut body = Vec::new();
    for value in values {
        match value {
            Ok(text) => {
                let (code, len) = super::varint(text.len() as u64 * 2 + 13);
                serials.extend_from_slice(&code[..len]);
                body.extend_from_slice(text.as_bytes());
            }
            Err(number) => {
                serials.push(1);
                body.push(number as u8);
            }
        }
    }
    let mut out = Vec::new();
    let (len, bytes) = super::varint(serials.len() as u64 + 1);
    out.extend_from_slice(&len[..bytes]);
    out.extend_from_slice(&serials);
    out.extend_from_slice(&body);
    out
}

#[test]
fn a_computed_column_that_cannot_be_computed_is_refused() {
    use crate::db::Error;
    // The table reads the schema page as its own rows, so that a row
    // exists to compute a column of. Its last column names itself.
    for text in [
        "CREATE TABLE a(p,q,r,s,t,u AS (u+1))",
        "CREATE TABLE a(p,q,r,s,t,u AS (nosuch))",
    ] {
        let file = schema_file(&[record([Ok("table"), Ok("a"), Ok("a"), Err(1), Ok(text)])]);
        let database = Database::open(&file).unwrap();
        assert_eq!(
            database.query(b"SELECT * FROM a").unwrap_err(),
            Error::Computed,
            "{text}"
        );
    }
}

#[test]
fn a_schema_row_that_is_not_a_table_is_passed_over() {
    // A file is not obliged to hold the schema the shell would write.
    // Five rows the reader must walk past rather than trip on: one whose
    // type is not text at all, one that is not a table, one whose root
    // page is not a number, one with no statement, and one that calls
    // itself a table and holds an index.
    let file = schema_file(&[
        record([Err(1), Ok("a"), Ok("a"), Err(2), Ok("CREATE TABLE a(x)")]),
        record([
            Ok("index"),
            Ok("i"),
            Ok("t"),
            Err(3),
            Ok("CREATE INDEX i ON t(x)"),
        ]),
        record([
            Ok("table"),
            Ok("b"),
            Ok("b"),
            Ok("two"),
            Ok("CREATE TABLE b(x)"),
        ]),
        record([Ok("table"), Ok("c"), Ok("c"), Err(4), Ok("")]),
        record([
            Ok("table"),
            Ok("e"),
            Ok("e"),
            Err(6),
            Ok("CREATE INDEX i ON t(x)"),
        ]),
        record([
            Ok("table"),
            Ok("d"),
            Ok("d"),
            Err(5),
            Ok("CREATE TABLE d(x)"),
        ]),
        // Three index rows the reader must walk past as well: one whose
        // root page is not a number, one that calls itself an index and
        // holds a table, and one over a table the file does not have.
        record([
            Ok("index"),
            Ok("j"),
            Ok("d"),
            Ok("two"),
            Ok("CREATE INDEX j ON d(x)"),
        ]),
        record([
            Ok("index"),
            Ok("l"),
            Ok("d"),
            Err(7),
            Ok("CREATE TABLE l(x)"),
        ]),
        record([
            Ok("index"),
            Ok("n"),
            Ok("nosuch"),
            Err(8),
            Ok("CREATE INDEX n ON nosuch(x)"),
        ]),
        // Two view rows the reader must walk past as well: one whose
        // statement it cannot read, and one that calls itself a view
        // and holds a table.
        record([Ok("view"), Ok("p"), Ok("p"), Err(0), Ok("CREATE VIEW p AS")]),
        record([
            Ok("view"),
            Ok("q"),
            Ok("q"),
            Err(0),
            Ok("CREATE TABLE q(x)"),
        ]),
        record([
            Ok("view"),
            Ok("r"),
            Ok("r"),
            Err(0),
            Ok("CREATE VIEW r AS SELECT x FROM d"),
        ]),
    ]);
    let database = Database::open(&file).expect("a database");
    let names: Vec<String> = database
        .tables()
        .map(|table| String::from_utf8_lossy(&table.name).into_owned())
        .collect();
    assert_eq!(names, ["d"]);
    // The view the reader could read is the only one it holds, and a
    // statement that names either of the others answers no table.
    assert!(database.view(b"r").is_some());
    assert!(database.view(b"p").is_none());
    assert!(database.view(b"q").is_none());
    assert!(database.query(b"SELECT x FROM p").is_err());
}

#[test]
fn a_schema_row_that_is_not_a_trigger_is_passed_over() {
    // Two trigger rows the reader must walk past: one whose statement
    // it cannot read, and one that calls itself a trigger and holds a
    // table.
    let file = schema_file(&[
        record([
            Ok("table"),
            Ok("d"),
            Ok("d"),
            Err(2),
            Ok("CREATE TABLE d(x)"),
        ]),
        record([
            Ok("trigger"),
            Ok("s"),
            Ok("d"),
            Err(0),
            Ok("CREATE TRIGGER s AFTER"),
        ]),
        record([
            Ok("trigger"),
            Ok("u"),
            Ok("d"),
            Err(0),
            Ok("CREATE TABLE u(x)"),
        ]),
        record([
            Ok("trigger"),
            Ok("w"),
            Ok("d"),
            Err(0),
            Ok("CREATE TRIGGER w AFTER INSERT ON d BEGIN SELECT 1; END"),
        ]),
    ]);
    let database = Database::open(&file).expect("a database");
    // The trigger the reader could read is the only one it holds.
    assert!(database.trigger(b"w").is_some());
    assert!(database.trigger(b"s").is_none());
    assert!(database.trigger(b"u").is_none());
}

#[test]
fn a_payload_longer_than_the_file_is_refused_before_room_is_made_for_it() {
    use crate::db::Error;
    use crate::error;
    // The length of a row is a number the file gives, and a file may
    // give one no machine holds. The reader refuses it rather than
    // asking for the room: a cell that is whole on its page but claims a
    // million million bytes on overflow pages.
    const PAGE: usize = 4096;
    const TOTAL: u64 = 1 << 40;
    // What the format leaves on the page for a payload that long.
    const LOCAL: usize = 1024;
    let mut file = super::SMALL[..100].to_vec();
    file.resize(PAGE, 0);
    file[28..32].copy_from_slice(&1u32.to_be_bytes());
    let (length, length_bytes) = super::varint(TOTAL);
    let mut cell = Vec::new();
    cell.extend_from_slice(&length[..length_bytes]);
    cell.push(1);
    cell.extend(core::iter::repeat_n(b'a', LOCAL));
    // The overflow page, which is never read because the length is
    // refused first.
    cell.extend_from_slice(&2u32.to_be_bytes());
    let at = PAGE - cell.len();
    file[at..].copy_from_slice(&cell);
    file[100] = 13;
    file[103..105].copy_from_slice(&1u16.to_be_bytes());
    file[105..107].copy_from_slice(&(at as u16).to_be_bytes());
    file[108..110].copy_from_slice(&(at as u16).to_be_bytes());
    assert_eq!(
        Database::open(&file).unwrap_err(),
        Error::Image(error::Error::Overrun)
    );
}
