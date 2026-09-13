// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::tree`.
//!
//! What holds the writer is the file the shell wrote: a database built
//! here out of a schema and its rows has to be the bytes of the fixture,
//! every one of them, which is what says the C library reads it.

#![allow(clippy::arithmetic_side_effects)]

use crate::header::{Encoding, Header, LIBRARY_VERSION};
use crate::page::Kind;
use crate::record::write;
use crate::tree::{Pages, insert};
use crate::value::{Affinity, Value};

/// The columns of `sqlite_schema`, which every schema row is written
/// with: `type`, `name`, `tbl_name`, `rootpage` and `sql`.
const SCHEMA: [Affinity; 5] = [
    Affinity::Text,
    Affinity::Text,
    Affinity::Text,
    Affinity::Integer,
    Affinity::Text,
];

/// The header a file of `pages` pages carries, with everything the two
/// statements of the fixture left behind.
fn header(page_size: u32, encoding: Encoding, changes: u32) -> Header {
    Header {
        page_size,
        write_version: 1,
        read_version: 1,
        reserved: 0,
        change_counter: changes,
        pages: 0,
        freelist: 0,
        freelist_pages: 0,
        schema_cookie: 1,
        schema_format: 4,
        cache_size: 0,
        largest_root: 0,
        encoding,
        user_version: 0,
        incremental_vacuum: 0,
        application_id: 0,
        version_valid_for: changes,
        library_version: LIBRARY_VERSION,
    }
}

/// One row of `sqlite_schema` for a table.
fn schema_row(name: &str, root: i64, sql: &str, encoding: Encoding) -> Vec<u8> {
    let text = |s: &str| Value::Text(crate::value::stored(s.as_bytes(), encoding));
    write(
        &[
            text("table"),
            text(name),
            text(name),
            Value::Int(root),
            text(sql),
        ],
        &SCHEMA,
        4,
    )
}

#[test]
fn a_database_written_from_a_schema_and_its_rows_is_the_file_the_shell_wrote() {
    // `CREATE TABLE t(a INTEGER, b TEXT, c REAL, d BLOB);` and the three
    // rows of `small.db`, which `sh tools/sqlite-fixtures.sh` wrote with
    // two statements, so the file counts two changes.
    let mut pages = Pages::new(4096, 0).unwrap();
    assert_eq!(pages.add(Kind::LeafTable).unwrap(), 2);
    let sql = "CREATE TABLE t(a INTEGER, b TEXT, c REAL, d BLOB)";
    let row = schema_row("t", 2, sql, Encoding::Utf8);
    assert!(insert(&mut pages, 1, 1, &row).unwrap());
    let columns = [
        Affinity::Integer,
        Affinity::Text,
        Affinity::Real,
        Affinity::Blob,
    ];
    let rows = [
        [
            Value::Int(1),
            Value::Text(b"one".to_vec()),
            Value::Real(1.5),
            Value::Blob(alloc::vec![1, 2]),
        ],
        [
            Value::Int(2),
            Value::Text(b"two".to_vec()),
            Value::Real(-2.5),
            Value::Null,
        ],
        [
            Value::Int(-3),
            Value::Text(Vec::new()),
            Value::Real(0.0),
            Value::Blob(alloc::vec![0xff]),
        ],
    ];
    for (at, values) in rows.iter().enumerate() {
        let rowid = i64::try_from(at).unwrap() + 1;
        let record = write(values, &columns, 4);
        assert!(insert(&mut pages, 2, rowid, &record).unwrap());
    }
    let written = pages.written(&header(4096, Encoding::Utf8, 2));
    let fixture = crate::tests::SMALL;
    assert_eq!(written.len(), fixture.len());
    if let Some(at) = (0..fixture.len()).find(|at| written[*at] != fixture[*at]) {
        panic!(
            "byte {at} of page {} is {} and was {}",
            at / 4096 + 1,
            written[at],
            fixture[at]
        );
    }
}

/// The bytes of a database built here, against the bytes of the fixture
/// it was built to be.
fn same(name: &str, written: &[u8], fixture: &[u8], page_size: usize) {
    assert_eq!(written.len(), fixture.len(), "{name} is another length");
    if let Some(at) = (0..fixture.len()).find(|at| written[*at] != fixture[*at]) {
        panic!(
            "{name}: byte {at} of page {} is {} and was {}",
            at / page_size + 1,
            written[at],
            fixture[at]
        );
    }
}

#[test]
fn a_database_whose_text_is_utf16_is_written_as_the_shell_wrote_it() {
    // `PRAGMA encoding='UTF-16le'; CREATE TABLE u(t TEXT);` and the two
    // rows of `utf16.db`. The text of a row and the text of the schema
    // are both in the encoding the file names.
    let encoding = Encoding::Utf16Le;
    let mut pages = Pages::new(4096, 0).unwrap();
    assert_eq!(pages.add(Kind::LeafTable).unwrap(), 2);
    let row = schema_row("u", 2, "CREATE TABLE u(t TEXT)", encoding);
    assert!(insert(&mut pages, 1, 1, &row).unwrap());
    for (at, text) in ["abc", "äöü"].iter().enumerate() {
        let value = Value::Text(crate::value::stored(text.as_bytes(), encoding));
        let record = write(&[value], &[Affinity::Text], 4);
        let rowid = i64::try_from(at).unwrap() + 1;
        assert!(insert(&mut pages, 2, rowid, &record).unwrap());
    }
    let written = pages.written(&header(4096, encoding, 2));
    same("utf16.db", &written, crate::tests::UTF16, 4096);
}

#[test]
fn a_database_of_three_tables_is_written_as_the_shell_wrote_it() {
    // The three tables of `joins.db`, each created and then filled, so
    // the file counts six changes and three schemas.
    let encoding = Encoding::Utf8;
    let text = |s: &str| Value::Text(s.as_bytes().to_vec());
    let mut pages = Pages::new(4096, 0).unwrap();
    let columns = [
        [Affinity::Integer, Affinity::Text],
        [Affinity::Integer, Affinity::Text],
        [Affinity::Text, Affinity::Integer],
    ];
    let statements = [
        ("a", "CREATE TABLE a(x INTEGER, y TEXT)"),
        ("b", "CREATE TABLE b(x INTEGER, z TEXT)"),
        ("c", "CREATE TABLE c(y TEXT COLLATE NOCASE, w INTEGER)"),
    ];
    let rows: [Vec<[Value; 2]>; 3] = [
        alloc::vec![
            [Value::Int(1), text("one")],
            [Value::Int(2), text("two")],
            [Value::Int(3), Value::Null],
        ],
        alloc::vec![
            [Value::Int(1), text("B1")],
            [Value::Int(1), text("B1b")],
            [Value::Int(4), text("B4")],
            [Value::Null, text("Bn")],
            [Value::Int(i64::MIN), text("Bmin")],
        ],
        alloc::vec![[text("ONE"), Value::Int(10)], [text("two"), Value::Int(20)],],
    ];
    for (at, (name, sql)) in statements.iter().enumerate() {
        let root = pages.add(Kind::LeafTable).unwrap();
        assert_eq!(root, u32::try_from(at).unwrap() + 2);
        let schema = schema_row(name, i64::from(root), sql, encoding);
        let rowid = i64::try_from(at).unwrap() + 1;
        assert!(insert(&mut pages, 1, rowid, &schema).unwrap());
        for (index, values) in rows[at].iter().enumerate() {
            let record = write(values, &columns[at], 4);
            let rowid = i64::try_from(index).unwrap() + 1;
            assert!(insert(&mut pages, root, rowid, &record).unwrap());
        }
    }
    let mut written = header(4096, encoding, 6);
    written.schema_cookie = 3;
    let written = pages.written(&written);
    same("joins.db", &written, crate::tests::JOINS, 4096);
}

#[test]
fn a_row_longer_than_a_page_runs_onto_the_overflow_pages_the_shell_wrote() {
    // `CREATE TABLE big(t TEXT); INSERT INTO big VALUES (eighteen
    // thousand letters);` which is `overflow.db`: four thousand of the
    // record stay on the leaf and the rest is four overflow pages.
    let mut pages = Pages::new(4096, 0).unwrap();
    assert_eq!(pages.add(Kind::LeafTable).unwrap(), 2);
    let row = schema_row("big", 2, "CREATE TABLE big(t TEXT)", Encoding::Utf8);
    assert!(insert(&mut pages, 1, 1, &row).unwrap());
    let text = Value::Text(alloc::vec![b'x'; 18_000]);
    let record = write(&[text], &[Affinity::Text], 4);
    assert!(insert(&mut pages, 2, 1, &record).unwrap());
    assert_eq!(pages.count(), 6);
    let written = pages.written(&header(4096, Encoding::Utf8, 2));
    same("overflow.db", &written, crate::tests::OVERFLOW, 4096);
}

#[test]
fn a_row_goes_on_the_leaf_the_descent_of_the_tree_finds() {
    use crate::page::{Cell, Writer, write_cell};
    // A tree of one interior page over two leaves, built by hand because
    // nothing else makes one yet: the row of a key the interior cell
    // names goes on the child it points at, and a key above it goes on
    // the page every key above the last cell lives in.
    let mut pages = Pages::new(4096, 0).unwrap();
    assert_eq!(pages.add(Kind::InteriorTable).unwrap(), 2);
    assert_eq!(pages.add(Kind::LeafTable).unwrap(), 3);
    assert_eq!(pages.add(Kind::LeafTable).unwrap(), 4);
    {
        let mut writer = pages.writer(2).unwrap();
        let cell = write_cell(&Cell::TableInterior {
            child: 3,
            rowid: 10,
        });
        assert!(writer.insert(0, &cell).unwrap());
    }
    // The right-most pointer is the interior page's own, which no cell
    // carries, so it is written where the header holds it.
    {
        let mut bytes = pages.bytes(2).unwrap().to_vec();
        bytes[8..12].copy_from_slice(&4_u32.to_be_bytes());
        let writer = Writer::open(&mut bytes, 2, 4096).unwrap();
        assert_eq!(writer.page().right_most(), Some(4));
        pages.put_page(2, &bytes).unwrap();
    }
    let record = write(&[Value::Int(1)], &[Affinity::None], 4);
    assert!(insert(&mut pages, 2, 5, &record).unwrap());
    assert!(insert(&mut pages, 2, 10, &record).unwrap());
    assert!(insert(&mut pages, 2, 11, &record).unwrap());
    assert_eq!(pages.page(3).unwrap().cells(), 2);
    assert_eq!(pages.page(4).unwrap().cells(), 1);
    // The two that went on the first leaf are in key order.
    let leaf = pages.page(3).unwrap();
    let keys: Vec<i64> = (0..leaf.cells())
        .map(|at| match leaf.cell(at).unwrap() {
            Cell::TableLeaf { rowid, .. } => rowid,
            _ => panic!("a cell of another shape"),
        })
        .collect();
    assert_eq!(keys, [5, 10]);
}

#[test]
fn a_page_size_the_format_does_not_allow_is_refused() {
    use crate::error::Error;
    let refusal = |size, reserved| Pages::new(size, reserved).err();
    assert_eq!(refusal(500, 0), Some(Error::PageSize(500)));
    assert_eq!(refusal(1000, 0), Some(Error::PageSize(1000)));
    assert_eq!(refusal(131_072, 0), Some(Error::PageSize(131_072)));
    assert_eq!(refusal(512, 33), Some(Error::Reserved(33)));
    assert!(Pages::new(512, 32).is_ok());
    assert!(Pages::new(65_536, 0).is_ok());
}

/// Points the right-most pointer of an interior page at `child`.
fn point(pages: &mut Pages, number: u32, child: u32) {
    let mut bytes = pages.bytes(number).unwrap().to_vec();
    let at = usize::from(number == 1) * crate::header::HEADER_LEN + 8;
    bytes[at..at + 4].copy_from_slice(&child.to_be_bytes());
    pages.put_page(number, &bytes).unwrap();
}

#[test]
fn a_tree_that_is_not_a_table_and_a_tree_deeper_than_the_walk_are_refused() {
    use crate::error::Error;
    use crate::page::{Cell, Payload, write_cell};
    let record = write(&[Value::Int(1)], &[Affinity::None], 4);
    // A tree of index pages holds no key a row belongs under.
    let mut pages = Pages::new(512, 0).unwrap();
    let root = pages.add(Kind::LeafIndex).unwrap();
    let cell = write_cell(&Cell::IndexLeaf {
        payload: Payload {
            local: b"\x02\x09",
            total: 2,
            overflow: None,
        },
    });
    assert!(pages.writer(root).unwrap().insert(0, &cell).unwrap());
    assert_eq!(
        insert(&mut pages, root, 1, &record),
        Err(Error::PageKind(10))
    );

    // A tree of interior pages that never reaches a leaf is one the
    // descent stops in rather than following forever.
    let mut pages = Pages::new(512, 0).unwrap();
    let mut above = 0;
    for _ in 0..34 {
        let number = pages.add(Kind::InteriorTable).unwrap();
        if above != 0 {
            point(&mut pages, above, number);
        }
        above = number;
    }
    assert_eq!(insert(&mut pages, 2, 1, &record), Err(Error::Depth));
}
