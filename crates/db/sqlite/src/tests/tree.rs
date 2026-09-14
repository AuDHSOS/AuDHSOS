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
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
    let sql = "CREATE TABLE t(a INTEGER, b TEXT, c REAL, d BLOB)";
    let row = schema_row("t", 2, sql, Encoding::Utf8);
    insert(&mut pages, 1, 1, &row).unwrap();
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
        insert(&mut pages, 2, rowid, &record).unwrap();
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
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
    let row = schema_row("u", 2, "CREATE TABLE u(t TEXT)", encoding);
    insert(&mut pages, 1, 1, &row).unwrap();
    for (at, text) in ["abc", "äöü"].iter().enumerate() {
        let value = Value::Text(crate::value::stored(text.as_bytes(), encoding));
        let record = write(&[value], &[Affinity::Text], 4);
        let rowid = i64::try_from(at).unwrap() + 1;
        insert(&mut pages, 2, rowid, &record).unwrap();
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
        let root = pages.add(Kind::LeafTable, 0).unwrap();
        assert_eq!(root, u32::try_from(at).unwrap() + 2);
        let schema = schema_row(name, i64::from(root), sql, encoding);
        let rowid = i64::try_from(at).unwrap() + 1;
        insert(&mut pages, 1, rowid, &schema).unwrap();
        for (index, values) in rows[at].iter().enumerate() {
            let record = write(values, &columns[at], 4);
            let rowid = i64::try_from(index).unwrap() + 1;
            insert(&mut pages, root, rowid, &record).unwrap();
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
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
    let row = schema_row("big", 2, "CREATE TABLE big(t TEXT)", Encoding::Utf8);
    insert(&mut pages, 1, 1, &row).unwrap();
    let text = Value::Text(alloc::vec![b'x'; 18_000]);
    let record = write(&[text], &[Affinity::Text], 4);
    insert(&mut pages, 2, 1, &record).unwrap();
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
    assert_eq!(pages.add(Kind::InteriorTable, 0).unwrap(), 2);
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 3);
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 4);
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
    insert(&mut pages, 2, 5, &record).unwrap();
    insert(&mut pages, 2, 10, &record).unwrap();
    insert(&mut pages, 2, 11, &record).unwrap();
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
    let root = pages.add(Kind::LeafIndex, 0).unwrap();
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
        let number = pages.add(Kind::InteriorTable, 0).unwrap();
        if above != 0 {
            point(&mut pages, above, number);
        }
        above = number;
    }
    assert_eq!(insert(&mut pages, 2, 1, &record), Err(Error::Depth));
    assert_eq!(crate::tree::largest(&pages, 2), Err(Error::Depth));
    // The walk of an index tree stops the same way.
    let mut pages = Pages::new(512, 0).unwrap();
    let mut above = 0;
    for _ in 0..34 {
        let number = pages.add(Kind::InteriorIndex, 0).unwrap();
        if above != 0 {
            point(&mut pages, above, number);
        }
        above = number;
    }
    assert_eq!(
        crate::tree::insert_entry(&mut pages, 2, &record, &[Value::Int(1)], &[], false),
        Err(Error::Depth)
    );
    // The descent that looks for one entry stops the same way.
    assert_eq!(
        crate::tree::remove_entry(&mut pages, 2, &[Value::Int(1)], &[]),
        Err(Error::Depth)
    );
    // The walk to the entry before one on an interior page stops the
    // same way: the root holds the entry and the pages under it never
    // reach a leaf.
    let mut pages = Pages::new(512, 0).unwrap();
    let root = pages.add(Kind::InteriorIndex, 0).unwrap();
    let mut above = 0;
    for _ in 0..34 {
        let number = pages.add(Kind::InteriorIndex, 0).unwrap();
        if above != 0 {
            point(&mut pages, above, number);
        }
        above = number;
    }
    let cell = write_cell(&Cell::IndexInterior {
        child: 3,
        payload: Payload {
            local: &record,
            total: record.len(),
            overflow: None,
        },
    });
    assert!(pages.writer(root).unwrap().insert(0, &cell).unwrap());
    assert_eq!(
        crate::tree::remove_entry(&mut pages, root, &[Value::Int(1)], &[]),
        Err(Error::Depth)
    );
    // The walk that frees a tree stops the same way, which a page that
    // names itself is what shows.
    let mut pages = Pages::new(512, 0).unwrap();
    let root = pages.add(Kind::InteriorTable, 0).unwrap();
    point(&mut pages, root, root);
    assert_eq!(crate::tree::destroy(&mut pages, root), Err(Error::Depth));
}

#[test]
fn a_table_that_outgrows_one_page_is_the_tree_the_shell_wrote() {
    // `PRAGMA page_size=512; CREATE TABLE t(n INTEGER, s TEXT);` and four
    // hundred rows in key order, which is `tall.db`: the leaf the rows go
    // on fills, the root grows a child under it, and every leaf after
    // that is a sibling with a divider on the root.
    let mut pages = Pages::new(512, 0).unwrap();
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
    let row = schema_row("t", 2, "CREATE TABLE t(n INTEGER, s TEXT)", Encoding::Utf8);
    insert(&mut pages, 1, 1, &row).unwrap();
    let columns = [Affinity::Integer, Affinity::Text];
    for number in 1..=400_i64 {
        let text = alloc::format!("row {number}");
        let values = [Value::Int(number), Value::Text(text.into_bytes())];
        let record = write(&values, &columns, 4);
        insert(&mut pages, 2, number, &record).unwrap();
    }
    assert_eq!(pages.count(), 15);
    let written = pages.written(&header(512, Encoding::Utf8, 2));
    same("tall.db", &written, crate::tests::TALL, 512);
}

/// One row of the tall table, as its record.
fn tall_row(number: i64) -> Vec<u8> {
    let text = alloc::format!("row {number}");
    write(
        &[Value::Int(number), Value::Text(text.into_bytes())],
        &[Affinity::Integer, Affinity::Text],
        4,
    )
}

#[test]
fn a_schema_larger_than_one_page_grows_a_tree_under_page_one() {
    // The schema table begins on page one, which carries the database
    // header before its own header, so the root of its tree hands a
    // child a hundred bytes more than any other root does.
    let mut pages = Pages::new(512, 0).unwrap();
    for number in 1..=20_i64 {
        let name = alloc::format!("t{number}");
        let sql = alloc::format!("CREATE TABLE {name}(a, b, c, d, e, f, g, h)");
        let row = schema_row(&name, number + 1, &sql, Encoding::Utf8);
        insert(&mut pages, 1, number, &row).unwrap();
    }
    assert_eq!(pages.page(1).unwrap().kind(), Kind::InteriorTable);
    assert_eq!(crate::tree::largest(&pages, 1).unwrap(), Some(20));
}

#[test]
fn what_the_two_halves_of_the_balance_refuse() {
    use crate::error::Error;
    use crate::page::{Cell, Payload, write_cell};
    use crate::tree::{deepen, quick};
    let mut pages = Pages::new(512, 0).unwrap();
    // A tree of index pages deepens the way a tree of rows does.
    let index = pages.add(Kind::LeafIndex, 0).unwrap();
    let deeper = pages.add(Kind::LeafIndex, 0).unwrap();
    let child = deepen(&mut pages, deeper).unwrap();
    assert_eq!(pages.page(deeper).unwrap().kind(), Kind::InteriorIndex);
    assert_eq!(pages.page(child).unwrap().kind(), Kind::LeafIndex);
    // A page with no cell has no largest key to divide on.
    let empty = pages.add(Kind::LeafTable, 0).unwrap();
    let parent = pages.add(Kind::InteriorTable, 0).unwrap();
    let cell = write_cell(&Cell::TableLeaf {
        rowid: 1,
        payload: Payload {
            local: b"\x02\x09",
            total: 2,
            overflow: None,
        },
    });
    assert_eq!(quick(&mut pages, parent, empty, &cell), Err(Error::Balance));
    // A page whose last cell is not a row of a table names no key.
    assert!(
        pages
            .writer(index)
            .unwrap()
            .insert(
                0,
                &write_cell(&Cell::IndexLeaf {
                    payload: Payload {
                        local: b"\x02\x09",
                        total: 2,
                        overflow: None,
                    },
                })
            )
            .unwrap()
    );
    assert_eq!(quick(&mut pages, parent, index, &cell), Err(Error::Balance));
    // A cell of more than a page holds is one no sibling takes.
    let leaf = pages.add(Kind::LeafTable, 0).unwrap();
    assert!(pages.writer(leaf).unwrap().insert(0, &cell).unwrap());
    let big = write_cell(&Cell::TableLeaf {
        rowid: 2,
        payload: Payload {
            local: &alloc::vec![7u8; 600],
            total: 600,
            overflow: None,
        },
    });
    assert_eq!(quick(&mut pages, parent, leaf, &big), Err(Error::Balance));
    // A parent with no room for the divider is one the tree cannot grow
    // under either.
    let full = pages.add(Kind::InteriorTable, 0).unwrap();
    let mut at = 0;
    while pages
        .writer(full)
        .unwrap()
        .insert(
            at,
            &write_cell(&Cell::TableInterior {
                child: 2,
                rowid: i64::try_from(at).unwrap(),
            }),
        )
        .unwrap()
    {
        at += 1;
    }
    assert_eq!(quick(&mut pages, full, leaf, &cell), Err(Error::Balance));
}

#[test]
fn a_table_filled_by_a_key_that_jumps_about_is_the_tree_the_shell_wrote() {
    // The rows of `shuffled.db`, put in by a key that lands in the
    // middle of a page every time, so that the page and its siblings are
    // written again rather than appended to.
    let mut pages = Pages::new(512, 0).unwrap();
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
    let row = schema_row("t", 2, "CREATE TABLE t(n INTEGER, s TEXT)", Encoding::Utf8);
    insert(&mut pages, 1, 1, &row).unwrap();
    for number in 1..=400_i64 {
        let rowid = (number * 137) % 401;
        insert(&mut pages, 2, rowid, &tall_row(number)).unwrap();
    }
    let written = pages.written(&header(512, Encoding::Utf8, 2));
    same("shuffled.db", &written, crate::tests::SHUFFLED, 512);
}

#[test]
fn a_tree_that_grows_a_third_level_is_the_tree_the_shell_wrote() {
    // The rows of `deep.db`: enough of them that the root fills, the
    // tree grows a third level, and the interior pages of that level are
    // balanced against each other, which runs the balance up from the
    // leaf to the root.
    let mut pages = Pages::new(512, 0).unwrap();
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
    let row = schema_row("t", 2, "CREATE TABLE t(n INTEGER, s TEXT)", Encoding::Utf8);
    insert(&mut pages, 1, 1, &row).unwrap();
    for number in 1..=4000_i64 {
        let rowid = (number * 1373) % 4201;
        insert(&mut pages, 2, rowid, &tall_row(number)).unwrap();
    }
    let written = pages.written(&header(512, Encoding::Utf8, 2));
    same("deep.db", &written, crate::tests::DEEP, 512);
}

#[test]
fn a_swap_of_pages_the_database_does_not_hold_is_refused() {
    use crate::error::Error;
    let mut pages = Pages::new(512, 0).unwrap();
    pages.add(Kind::LeafTable, 0).unwrap();
    assert_eq!(pages.swap(1, 3), Err(Error::Page(0)));
    assert_eq!(pages.swap(3, 1), Err(Error::Page(0)));
    assert_eq!(pages.swap(0, 1), Err(Error::Page(0)));
    assert_eq!(pages.swap(1, 2), Ok(()));
}

/// Fills the leaf `number` with rows of `payload` bytes, `step` apart in
/// key from `first`, and answers how many of them it took.
fn fill(pages: &mut Pages, number: u32, first: i64, step: i64, payload: usize) -> i64 {
    use crate::page::{Cell, Payload, write_cell};
    let mut rowid = first;
    loop {
        let cell = write_cell(&Cell::TableLeaf {
            rowid,
            payload: Payload {
                local: &alloc::vec![7u8; payload],
                total: payload,
                overflow: None,
            },
        });
        let at = pages.page(number).unwrap().cells();
        if !pages.writer(number).unwrap().insert(at, &cell).unwrap() {
            return rowid;
        }
        rowid += step;
    }
}

#[test]
fn a_balance_over_siblings_of_two_kinds_is_refused() {
    use crate::error::Error;
    use crate::page::{Cell, write_cell};
    // A parent whose right-most child belongs to an index tree, which
    // the balance of the leaf beside it reads as a sibling.
    let mut pages = Pages::new(512, 0).unwrap();
    let parent = pages.add(Kind::InteriorTable, 0).unwrap();
    let leaf = pages.add(Kind::LeafTable, 0).unwrap();
    let other = pages.add(Kind::LeafIndex, 0).unwrap();
    let divider = write_cell(&Cell::TableInterior {
        child: leaf,
        rowid: 10_000,
    });
    assert!(pages.writer(parent).unwrap().insert(0, &divider).unwrap());
    point(&mut pages, parent, other);
    fill(&mut pages, leaf, 2, 2, 20);
    assert_eq!(
        insert(&mut pages, parent, 3, &tall_row(3)),
        Err(Error::Balance)
    );
}

#[test]
fn the_write_path_under_every_configuration_is_the_file_the_shell_wrote() {
    // The matrix of document 16, section 16.11, over writing: the same
    // four hundred rows under every page size, every encoding and every
    // reserved tail, each held to the file the shell wrote under it.
    for (name, page_size, reserved, encoding, fixture) in crate::tests::CONFIGURED {
        let mut pages = Pages::new(page_size, reserved).unwrap();
        assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
        let sql = "CREATE TABLE t(n INTEGER, s TEXT)";
        insert(&mut pages, 1, 1, &schema_row("t", 2, sql, encoding)).unwrap();
        for number in 1..=400_i64 {
            let rowid = (number * 137) % 401;
            let text = alloc::format!("row {number}");
            let row = write(
                &[
                    Value::Int(number),
                    Value::Text(crate::value::stored(text.as_bytes(), encoding)),
                ],
                &[Affinity::Integer, Affinity::Text],
                4,
            );
            insert(&mut pages, 2, rowid, &row).unwrap();
        }
        let mut head = header(page_size, encoding, 2);
        head.reserved = reserved;
        let written = pages.written(&head);
        same(
            name,
            &written,
            fixture,
            crate::bytes::size(u64::from(page_size)),
        );
    }
}

/// The four hundred rows of `shuffled.db` in a table on page two, with
/// the schema row of that table on page one.
fn filled(page_size: u32) -> Pages {
    let mut pages = Pages::new(page_size, 0).unwrap();
    pages.add(Kind::LeafTable, 0).unwrap();
    let sql = "CREATE TABLE t(n INTEGER, s TEXT)";
    let row = schema_row("t", 2, sql, Encoding::Utf8);
    insert(&mut pages, 1, 1, &row).unwrap();
    for number in 1..=400_i64 {
        let rowid = (number * 137) % 401;
        insert(&mut pages, 2, rowid, &tall_row(number)).unwrap();
    }
    pages.begin();
    pages
}

#[test]
fn rows_taken_out_again_leave_the_file_the_shell_left() {
    use crate::tree::remove;
    // Three deletes over the rows of `shuffled.db`: one that only evens
    // the leaves out, one that frees pages, and one that takes every row
    // out. Each is one statement of the shell, so the file counts three
    // changes.
    /// One delete: the file it leaves, and which keys it takes out.
    type Delete = (&'static str, &'static [u8], fn(i64) -> bool);
    let cases: [Delete; 3] = [
        ("deleted.db", crate::tests::DELETED, |rowid| rowid % 3 == 0),
        ("emptied.db", crate::tests::EMPTIED, |rowid| rowid % 4 != 0),
        ("cleared.db", crate::tests::CLEARED, |_| true),
    ];
    for (name, fixture, goes) in cases {
        let mut pages = filled(512);
        for rowid in 1..=400_i64 {
            if goes(rowid) {
                assert!(remove(&mut pages, 2, rowid).unwrap());
            }
        }
        let written = pages.written(&header(512, Encoding::Utf8, 3));
        same(name, &written, fixture, 512);
    }
}

#[test]
fn a_row_that_ran_onto_overflow_pages_gives_them_back() {
    use crate::tree::remove;
    // The rows of `unchained.db`: forty of them, each longer than the
    // one before, so that the later ones run onto chains of pages the
    // delete has to give back.
    let mut pages = Pages::new(512, 0).unwrap();
    pages.add(Kind::LeafTable, 0).unwrap();
    let sql = "CREATE TABLE t(n INTEGER, s TEXT)";
    insert(&mut pages, 1, 1, &schema_row("t", 2, sql, Encoding::Utf8)).unwrap();
    for number in 1..=40_i64 {
        let text = "x".repeat(usize::try_from(number).unwrap() * 60);
        let row = write(
            &[Value::Int(number), Value::Text(text.into_bytes())],
            &[Affinity::Integer, Affinity::Text],
            4,
        );
        insert(&mut pages, 2, (number * 17) % 41, &row).unwrap();
    }
    pages.begin();
    for rowid in 1..=40_i64 {
        if rowid % 3 != 0 {
            assert!(remove(&mut pages, 2, rowid).unwrap());
        }
    }
    let written = pages.written(&header(512, Encoding::Utf8, 3));
    same("unchained.db", &written, crate::tests::UNCHAINED, 512);
}

#[test]
fn a_key_the_table_does_not_hold_takes_no_row_out() {
    use crate::tree::remove;
    let mut pages = filled(512);
    assert!(!remove(&mut pages, 2, 0).unwrap());
    assert!(!remove(&mut pages, 2, 401).unwrap());
    assert!(remove(&mut pages, 2, 200).unwrap());
    assert!(!remove(&mut pages, 2, 200).unwrap());
}

#[test]
fn the_pages_a_delete_freed_are_the_ones_the_next_insert_takes() {
    use crate::tree::remove;
    // The three statements of `reused.db`, each its own transaction, so
    // the file counts four changes: the schema, four thousand rows,
    // seven in eight taken out again, and a thousand put in after that.
    let mut pages = Pages::new(512, 0).unwrap();
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
    let sql = "CREATE TABLE t(n INTEGER, s TEXT)";
    insert(&mut pages, 1, 1, &schema_row("t", 2, sql, Encoding::Utf8)).unwrap();
    for number in 1..=4000_i64 {
        insert(&mut pages, 2, (number * 1373) % 4201, &tall_row(number)).unwrap();
    }
    pages.begin();
    for rowid in 1..=4200_i64 {
        if rowid % 8 != 0 {
            remove(&mut pages, 2, rowid).unwrap();
        }
    }
    pages.begin();
    for number in 1..=1000_i64 {
        let text = alloc::format!("more {number}");
        let row = write(
            &[Value::Int(number), Value::Text(text.into_bytes())],
            &[Affinity::Integer, Affinity::Text],
            4,
        );
        insert(&mut pages, 2, 5000 + (number * 379) % 1009, &row).unwrap();
    }
    let written = pages.written(&header(512, Encoding::Utf8, 4));
    same("reused.db", &written, crate::tests::REUSED, 512);
}

#[test]
fn what_the_free_list_refuses() {
    use crate::error::Error;
    let mut pages = Pages::new(512, 0).unwrap();
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
    // Page one holds the header of the database and is on no free list,
    // and a page the database does not hold is on none either.
    assert_eq!(pages.release(1), Err(Error::Page(1)));
    assert_eq!(pages.release(9), Err(Error::Page(9)));
    assert_eq!(pages.release(2), Ok(()));
    // The page comes back off the list as noughts, because the pager
    // reads nothing for a page nothing needs.
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
    assert_eq!(pages.page(2).unwrap().cells(), 0);
}

#[test]
fn an_overflow_chain_that_turns_back_on_itself_is_refused() {
    use crate::error::Error;
    use crate::tree::remove;
    // A row of eight hundred bytes, which runs onto a chain of overflow
    // pages, with the first of them pointed back at itself.
    let mut pages = Pages::new(512, 0).unwrap();
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
    let row = write(&[Value::Blob(alloc::vec![7u8; 2000])], &[Affinity::None], 4);
    insert(&mut pages, 2, 1, &row).unwrap();
    let last = pages.count();
    assert!(last > 3);
    let mut bytes = pages.bytes(last).unwrap().to_vec();
    bytes[0..4].copy_from_slice(&last.to_be_bytes());
    pages.put_page(last, &bytes).unwrap();
    assert_eq!(remove(&mut pages, 2, 1), Err(Error::Overflow(last)));
}

#[test]
fn the_journal_a_commit_leaves_is_the_one_the_shell_left() {
    use crate::journal::{Journal, Mode, committed};
    use crate::tree::remove;
    // The delete of `emptied.db` under a journal mode that keeps the
    // file. The nonce every checksum begins at comes from SQLite's
    // random source, so the one the fixture was written with is read
    // back out of it: the first record says the nonce and the eight
    // after it say the nonce is right.
    let mut pages = filled(512);
    let mut was = header(512, Encoding::Utf8, 2);
    was.pages = pages.count();
    for rowid in 1..=400_i64 {
        if rowid % 4 != 0 {
            remove(&mut pages, 2, rowid).unwrap();
        }
    }
    let theirs = crate::tests::JOURNALLED;
    let nonce = nonce_of(theirs, &pages.journal(&was, 0, 512));
    let mine = committed(&pages.journal(&was, nonce, 512), Mode::Persist).unwrap();
    same("journalled.db-journal", &mine, theirs, 512);
    // The journal this crate wrote is one this crate reads back: every
    // page it holds is the page the transaction began with.
    let hot = pages.journal(&was, nonce, 512);
    let read = Journal::open(&hot);
    assert!(read.hot());
    assert_eq!(read.page_size(), 512);
    assert_eq!(read.pages(), was.pages);
    assert_eq!(
        read.page_bytes(2),
        Some(&hot[512 + 520 + 4..512 + 520 + 516])
    );
}

/// The nonce `theirs` was written with, which is what its first record
/// says once the sum of that page is taken off it. A journal this crate
/// wrote with a nonce of nought holds that sum.
fn nonce_of(theirs: &[u8], plain: &[u8]) -> u32 {
    let at = 512 + 512 + 4;
    let sum = u32::from_be_bytes(plain[at..at + 4].try_into().unwrap());
    u32::from_be_bytes(theirs[at..at + 4].try_into().unwrap()).wrapping_sub(sum)
}

#[test]
fn a_commit_puts_page_one_in_the_journal_last() {
    use crate::journal::{Mode, committed};
    // The four hundred rows of `shuffled.db` put in by one statement,
    // which writes no cell of page one: the journal holds the leaf and
    // then page one, which the commit writes the change counter into.
    let mut pages = Pages::new(512, 0).unwrap();
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
    let sql = "CREATE TABLE t(n INTEGER, s TEXT)";
    insert(&mut pages, 1, 1, &schema_row("t", 2, sql, Encoding::Utf8)).unwrap();
    let mut was = header(512, Encoding::Utf8, 1);
    was.pages = pages.count();
    was.schema_cookie = 1;
    pages.begin();
    for number in 1..=400_i64 {
        insert(&mut pages, 2, (number * 137) % 401, &tall_row(number)).unwrap();
    }
    let theirs = crate::tests::APPENDED;
    let nonce = nonce_of(theirs, &pages.journal(&was, 0, 512));
    let mine = committed(&pages.journal(&was, nonce, 512), Mode::Persist).unwrap();
    same("appended.db-journal", &mine, theirs, 512);
}

#[test]
fn what_each_journal_mode_leaves_behind() {
    use crate::journal::{Mode, committed};
    let journal = alloc::vec![9u8; 600];
    assert_eq!(committed(&journal, Mode::Delete), None);
    assert_eq!(committed(&journal, Mode::Memory), None);
    assert_eq!(committed(&journal, Mode::Off), None);
    assert_eq!(committed(&journal, Mode::Truncate), Some(Vec::new()));
    let kept = committed(&journal, Mode::Persist).unwrap();
    assert_eq!(kept.len(), 600);
    assert!(kept[..28].iter().all(|byte| *byte == 0));
    assert!(kept[28..].iter().all(|byte| *byte == 9));
}

#[test]
fn a_table_written_into_a_log_is_the_log_the_shell_wrote() {
    use crate::wal::{Log, Wal};
    // The two statements of `shuffled.db` under a write-ahead log that
    // was never checkpointed: the database holds the one page `PRAGMA
    // journal_mode=wal` left and the log holds everything after it.
    // The two salts come from SQLite's random source, so the ones the
    // fixture was written with are read back out of its header; every
    // checksum of every frame is then what says they are right.
    let theirs = crate::tests::LOGGING_WAL;
    let word = |at: usize| u32::from_be_bytes(theirs[at..at + 4].try_into().unwrap());
    let mut log = Log::new(512, (word(16), word(20)), 0, false);
    let mut pages = Pages::new(512, 0).unwrap();
    let mut now = header(512, Encoding::Utf8, 1);
    now.write_version = 2;
    now.read_version = 2;
    now.schema_cookie = 0;
    now.schema_format = 0;
    now.encoding = Encoding::Utf8;
    now.version_valid_for = 1;
    // What the database holds is the page that pragma left, because
    // nothing after it was checkpointed.
    same(
        "logging.db",
        &pages.written(&now),
        crate::tests::LOGGING,
        512,
    );
    // The first statement writes the schema, which is where the format
    // and the encoding are written with it.
    now.change_counter = 2;
    now.version_valid_for = 2;
    now.schema_cookie = 1;
    now.schema_format = 4;
    pages.begin();
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
    let sql = "CREATE TABLE t(n INTEGER, s TEXT)";
    insert(&mut pages, 1, 1, &schema_row("t", 2, sql, Encoding::Utf8)).unwrap();
    now.pages = pages.count();
    log.commit(&pages.frames(&now), pages.count());
    pages.begin();
    for number in 1..=400_i64 {
        insert(&mut pages, 2, (number * 137) % 401, &tall_row(number)).unwrap();
    }
    now.pages = pages.count();
    log.commit(&pages.frames(&now), pages.count());
    // A third statement that takes rows out without freeing a page:
    // the count does not change, so the log holds no page one for it.
    pages.begin();
    for rowid in 1..=400_i64 {
        if rowid % 3 == 0 {
            assert!(crate::tree::remove(&mut pages, 2, rowid).unwrap());
        }
    }
    let frames = pages.frames(&now);
    assert!(!frames.iter().any(|(number, _)| *number == 1));
    log.commit(&frames, pages.count());
    same("logging.db-wal", log.bytes(), theirs, 512);
    // The log this crate wrote is one this crate reads back: the second
    // transaction wrote every page, so every page comes back as that
    // transaction left it.
    let read = Wal::open(log.bytes()).unwrap();
    assert_eq!(read.page_size(), log.page_size());
    assert_eq!(read.pages(), pages.count());
    for (number, page) in &frames {
        assert_eq!(read.page_bytes(*number), Some(page.as_slice()));
    }
    // A log whose checksums read their words big-endian is the same
    // file under the other magic, and reads back the same way.
    let mut other = Log::new(512, (word(16), word(20)), 0, true);
    other.commit(&frames, pages.count());
    let read = Wal::open(other.bytes()).unwrap();
    assert_eq!(read.pages(), pages.count());
    for (number, page) in &frames {
        assert_eq!(read.page_bytes(*number), Some(page.as_slice()));
    }
}

#[test]
fn statements_run_from_their_text_write_the_file_the_shell_wrote() {
    use crate::change::Writer;
    // Four databases built by running the statements the shell was
    // given, each held to the file the shell wrote.
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a INTEGER, b TEXT, c REAL, d BLOB)")
        .unwrap();
    writer
        .run(b"INSERT INTO t VALUES (1,'one',1.5,x'0102'), (2,'two',-2.5,NULL), (-3,'',0.0,x'ff')")
        .unwrap();
    same("small.db", &writer.written(), crate::tests::SMALL, 4096);

    // The encoding the file names is the one the text is stored in.
    let mut writer = Writer::new(4096, 0, Encoding::Utf16Le).unwrap();
    writer.run(b"CREATE TABLE u(t TEXT)").unwrap();
    writer
        .run("INSERT INTO u VALUES ('abc'), ('\u{e4}\u{f6}\u{fc}')".as_bytes())
        .unwrap();
    same("utf16.db", &writer.written(), crate::tests::UTF16, 4096);

    // Three tables, each created and then filled, so the file counts
    // six changes and three schema rows.
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    for sql in [
        "CREATE TABLE a(x INTEGER, y TEXT)",
        "INSERT INTO a VALUES (1,'one'),(2,'two'),(3,NULL)",
        "CREATE TABLE b(x INTEGER, z TEXT)",
        "INSERT INTO b VALUES (1,'B1'),(1,'B1b'),(4,'B4'),(NULL,'Bn'),(-9223372036854775808,'Bmin')",
        "CREATE TABLE c(y TEXT COLLATE NOCASE, w INTEGER)",
        "INSERT INTO c VALUES ('ONE',10),('two',20)",
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    same("joins.db", &writer.written(), crate::tests::JOINS, 4096);

    // A key that is one of the columns, rows with it given and rows
    // without, columns named in another order, and rows read out of one
    // table into another.
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    for sql in [
        "CREATE TABLE r(id INTEGER PRIMARY KEY, v TEXT)",
        "INSERT INTO r VALUES (5,'five'),(2,'two'),(9,'nine')",
        "INSERT INTO r(v) VALUES ('ten')",
        "CREATE TABLE s(a, b)",
        "INSERT INTO s SELECT id, v FROM r",
        "INSERT INTO s(b,a) VALUES ('x',1)",
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    same("stated.db", &writer.written(), crate::tests::STATED, 4096);
}

#[test]
fn what_a_statement_that_changes_a_database_refuses() {
    use crate::change::Writer;
    use crate::db::Error;
    let refuse = |statements: &[&str]| -> Error {
        let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
        let mut last = None;
        for sql in statements {
            last = Some(writer.run(sql.as_bytes()));
        }
        match last {
            Some(Err(error)) => error,
            _ => panic!("the statement was not refused"),
        }
    };
    // A page size the format does not allow is refused before a
    // statement is read at all.
    assert!(Writer::new(500, 0, Encoding::Utf8).is_err());
    // A statement this crate does not write.
    assert!(matches!(refuse(&["SELECT 1"]), Error::Parse(_)));
    assert!(matches!(
        refuse(&["CREATE INDEX i ON t(a)"]),
        Error::Unsupported
    ));
    assert!(matches!(
        refuse(&["CREATE TABLE t AS SELECT 1"]),
        Error::Unsupported
    ));
    // A table the database does not hold, a column the table does not
    // have, and a row of another width.
    assert!(matches!(
        refuse(&["INSERT INTO nowhere VALUES (1)"]),
        Error::Unsupported
    ));
    assert!(matches!(
        refuse(&["CREATE TABLE t(a)", "INSERT INTO t(b) VALUES (1)"]),
        Error::Unsupported
    ));
    assert!(matches!(
        refuse(&["CREATE TABLE t(a)", "INSERT INTO t VALUES (1,2)"]),
        Error::Unsupported
    ));
    // A key that is not a whole number.
    assert!(matches!(
        refuse(&["CREATE TABLE t(a)", "INSERT INTO t(rowid,a) VALUES ('x',1)"]),
        Error::Unsupported
    ));
    // A table whose rows are kept in the key's own tree.
    assert!(matches!(
        refuse(&[
            "CREATE TABLE w(a TEXT, b, PRIMARY KEY(a)) WITHOUT ROWID",
            "INSERT INTO w VALUES ('x',1)",
        ]),
        Error::Unsupported
    ));
}

#[test]
fn a_statement_is_stored_as_its_own_text_without_what_ends_it() {
    use crate::change::Writer;
    // `sqlite_schema` holds the statement as it was written, with the
    // space around it and the semicolon that ends it taken off, which
    // is the span the parser read it from.
    let written = |sql: &[u8]| {
        let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
        writer.run(sql).unwrap();
        writer.written()
    };
    assert_eq!(
        written(b"  CREATE TABLE t(a) ;  "),
        written(b"CREATE TABLE t(a)")
    );
}

#[test]
fn the_largest_key_of_a_tree_is_the_last_of_its_right_most_leaf() {
    use crate::tree::largest;
    // A tree of one page, and one deep enough that the walk follows an
    // interior page's right-most pointer.
    let mut pages = Pages::new(512, 0).unwrap();
    let root = pages.add(Kind::LeafTable, 0).unwrap();
    assert_eq!(largest(&pages, root).unwrap(), None);
    let tall = filled(512);
    assert_eq!(largest(&tall, 2).unwrap(), Some(400));
    assert!(tall.page(2).unwrap().kind().is_interior());
}

#[test]
fn a_key_given_by_name_is_the_key_the_row_is_put_in_under() {
    use crate::change::Writer;
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer
        .run(b"INSERT INTO t(rowid,a) VALUES (7,'x')")
        .unwrap();
    writer.run(b"INSERT INTO t(a) VALUES ('y')").unwrap();
    let written = writer.written();
    let database = crate::db::Database::open(&written).unwrap();
    let answer = database.query(b"SELECT rowid, a FROM t").unwrap();
    assert_eq!(
        answer.rows,
        alloc::vec![
            alloc::vec![Value::Int(7), Value::Text(b"x".to_vec())],
            alloc::vec![Value::Int(8), Value::Text(b"y".to_vec())],
        ]
    );
}

#[test]
fn a_schema_that_outgrows_page_one_is_refused_by_the_statement() {
    use crate::change::Writer;
    use crate::db::Error;
    // The schema table begins on the page the database header is on, so
    // a root that outgrows it is the balance this crate does not write.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    let mut at = 0;
    loop {
        let name = alloc::format!("t{at}");
        let sql = alloc::format!("CREATE TABLE {name}(a, b, c, d, e, f, g, h, i, j, k, l)");
        match writer.run(sql.as_bytes()) {
            Ok(_) => at += 1,
            Err(error) => {
                assert!(matches!(error, Error::Image(_)), "{error:?}");
                assert!(at > 1, "the first statement was refused");
                return;
            }
        }
    }
}

#[test]
fn what_the_reader_of_a_statement_that_changes_a_database_refuses() {
    use crate::parse::change;
    // A word after the statement, which nothing may follow.
    assert!(change(b"INSERT INTO t VALUES (1) extra").is_err());
    // `INSERT OR` and the five words that may follow it, and `REPLACE`,
    // which is `INSERT OR REPLACE`.
    for sql in [
        "INSERT OR ROLLBACK INTO t VALUES (1)",
        "INSERT OR ABORT INTO t VALUES (1)",
        "INSERT OR FAIL INTO t VALUES (1)",
        "INSERT OR IGNORE INTO t VALUES (1)",
        "INSERT OR REPLACE INTO t VALUES (1)",
        "REPLACE INTO t VALUES (1)",
        "WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x",
    ] {
        assert!(change(sql.as_bytes()).is_ok(), "{sql}");
    }
    assert!(change(b"INSERT OR").is_err());
    assert!(change(b"INSERT OR NOTHING INTO t VALUES (1)").is_err());
    assert!(change(b"INSERT t VALUES (1)").is_err());
}

/// The four hundred rows of `shuffled.db` as one statement.
fn sql_of_rows() -> alloc::string::String {
    use core::fmt::Write;
    let mut sql = alloc::string::String::from("INSERT INTO t(rowid,n,s) VALUES ");
    for number in 1..=400_i64 {
        if number > 1 {
            sql.push(',');
        }
        let _ = write!(
            sql,
            "({},{},'row {}')",
            (number * 137) % 401,
            number,
            number
        );
    }
    sql
}

#[test]
fn rows_taken_out_by_a_statement_leave_the_file_the_shell_left() {
    use crate::change::Writer;
    // The rows of `shuffled.db` put in by one statement that names the
    // key of each, and then three deletes: one that only evens the
    // leaves out, one that frees pages, and one that takes every row
    // out.
    let filled = {
        let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
        writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
        writer.run(sql_of_rows().as_bytes()).unwrap();
        same(
            "shuffled.db",
            &writer.written(),
            crate::tests::SHUFFLED,
            512,
        );
        writer.written()
    };
    let cases: [(&str, &[u8], &str); 3] = [
        (
            "deleted.db",
            crate::tests::DELETED,
            "DELETE FROM t WHERE rowid%3=0",
        ),
        (
            "emptied.db",
            crate::tests::EMPTIED,
            "DELETE FROM t WHERE rowid%4!=0",
        ),
        (
            "cleared.db",
            crate::tests::CLEARED,
            "DELETE FROM t WHERE rowid>0",
        ),
    ];
    for (name, fixture, sql) in cases {
        let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
        writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
        writer.run(sql_of_rows().as_bytes()).unwrap();
        assert_eq!(writer.written(), filled);
        writer.run(sql.as_bytes()).unwrap();
        same(name, &writer.written(), fixture, 512);
    }
}

#[test]
fn a_delete_reads_the_columns_of_the_row_it_is_asked_about() {
    use crate::change::Writer;
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a INTEGER, b TEXT)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES (1,'one'),(2,'two'),(3,'three'),(4,NULL)")
        .unwrap();
    // A column written with the table's name before it, a column
    // written without, and the three names of the key.
    writer.run(b"DELETE FROM t WHERE t.b = 'two'").unwrap();
    writer.run(b"DELETE FROM t WHERE a = 3").unwrap();
    writer.run(b"DELETE FROM t WHERE oid = 4").unwrap();
    let written = writer.written();
    let database = crate::db::Database::open(&written).unwrap();
    let answer = database.query(b"SELECT rowid, a, b FROM t").unwrap();
    assert_eq!(
        answer.rows,
        alloc::vec![alloc::vec![
            Value::Int(1),
            Value::Int(1),
            Value::Text(b"one".to_vec())
        ]]
    );
    // A `WHERE` that answers nothing takes no row out, and one over a
    // column the table does not have is refused.
    writer.run(b"DELETE FROM t WHERE b = 'nine'").unwrap();
    assert_eq!(writer.written(), written);
    assert!(writer.run(b"DELETE FROM t WHERE nowhere = 1").is_err());
    // A schema or a table the row does not belong to names no column
    // of it.
    assert!(writer.run(b"DELETE FROM t WHERE other.a = 1").is_err());
    assert!(writer.run(b"DELETE FROM t WHERE temp.t.a = 1").is_err());
    assert!(writer.run(b"DELETE FROM nowhere WHERE a = 1").is_err());
    // The row the statement above named is still there, and a column
    // written with its schema and its table before it takes it out.
    assert!(writer.run(b"DELETE FROM t WHERE main.t.a = 1").is_ok());
    let written = writer.written();
    let database = crate::db::Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"SELECT count(*) FROM t").unwrap().rows,
        alloc::vec![alloc::vec![Value::Int(0)]]
    );
    // A table that keeps its rows in the key's own tree is one this
    // crate does not walk to change.
    writer
        .run(b"CREATE TABLE w(a TEXT, b, PRIMARY KEY(a)) WITHOUT ROWID")
        .unwrap();
    assert!(writer.run(b"DELETE FROM w").is_err());
    // A statement with no `WHERE` takes every row out, and a `WHERE`
    // that reads the bytes as they are stored reads them in the
    // encoding the file names.
    let mut writer = Writer::new(4096, 0, Encoding::Utf16Le).unwrap();
    writer.run(b"CREATE TABLE u(t TEXT)").unwrap();
    writer
        .run(b"INSERT INTO u VALUES ('abc'),('de'),('f')")
        .unwrap();
    writer
        .run(b"DELETE FROM u WHERE octet_length(t) = 4")
        .unwrap();
    let written = writer.written();
    let database = crate::db::Database::open(&written).unwrap();
    let answer = database.query(b"SELECT t FROM u").unwrap();
    assert_eq!(
        answer.rows,
        alloc::vec![
            alloc::vec![Value::Text(b"abc".to_vec())],
            alloc::vec![Value::Text(b"f".to_vec())]
        ]
    );
    writer.run(b"DELETE FROM u").unwrap();
    let written = writer.written();
    let database = crate::db::Database::open(&written).unwrap();
    assert!(database.query(b"SELECT t FROM u").unwrap().rows.is_empty());
}

#[test]
fn a_row_that_is_not_a_record_is_refused_by_the_walk_of_a_table() {
    // The walk a statement that changes rows reads is the one that
    // answers what each column holds, so a row whose bytes are not a
    // record is refused there rather than read as nothing.
    let mut pages = Pages::new(512, 0).unwrap();
    assert_eq!(pages.add(Kind::LeafTable, 0).unwrap(), 2);
    let sql = "CREATE TABLE t(a)";
    insert(&mut pages, 1, 1, &schema_row("t", 2, sql, Encoding::Utf8)).unwrap();
    // A record whose header says more bytes than the row holds.
    insert(&mut pages, 2, 1, &[0xff, 0xff]).unwrap();
    let written = pages.written(&header(512, Encoding::Utf8, 2));
    let database = crate::db::Database::open(&written).unwrap();
    assert!(database.rows_of(b"t").is_err());
}

#[test]
fn what_the_reader_of_a_delete_refuses() {
    use crate::parse::change;
    assert!(change(b"DELETE FROM t").is_ok());
    assert!(change(b"DELETE FROM t WHERE a = 1").is_ok());
    assert!(change(b"DELETE t WHERE a = 1").is_err());
    assert!(change(b"DELETE FROM t WHERE a = 1 extra").is_err());
    // A `WITH` before a `DELETE` names tables its `WHERE` may read,
    // which this crate does not answer yet.
    assert!(change(b"WITH x AS (SELECT 1) DELETE FROM t").is_err());
}

#[test]
fn what_the_reader_of_an_update_refuses() {
    use crate::parse::change;
    assert!(change(b"UPDATE t SET a = 1").is_ok());
    assert!(change(b"UPDATE t SET a = 1, b = 2 WHERE c = 3").is_ok());
    assert!(change(b"UPDATE OR REPLACE t SET a = 1").is_ok());
    assert!(change(b"UPDATE t a = 1").is_err());
    assert!(change(b"UPDATE t SET a 1").is_err());
    assert!(change(b"UPDATE t SET a = 1 extra").is_err());
    // A `WITH` before an `UPDATE` names tables its `WHERE` may read,
    // which this crate does not answer yet.
    assert!(change(b"WITH x AS (SELECT 1) UPDATE t SET a = 1").is_err());
}

#[test]
fn a_statement_that_names_no_rows_to_leave_out_writes_over_every_row() {
    use crate::change::Writer;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a INTEGER, b TEXT)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES (1,'one'),(2,'two'),(3,'three')")
        .unwrap();
    writer.run(b"UPDATE t SET b='all'").unwrap();
    writer
        .run(b"UPDATE t SET rowid=rowid+10 WHERE a=2")
        .unwrap();
    same("keyed.db", &writer.written(), crate::tests::KEYED, 512);
}

#[test]
fn a_key_the_statement_writes_is_refused_where_it_is_not_a_number() {
    use crate::change::Writer;
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a INTEGER)").unwrap();
    writer.run(b"INSERT INTO t VALUES (1)").unwrap();
    assert!(writer.run(b"UPDATE t SET rowid = 'x'").is_err());
    let mut keyed = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    keyed
        .run(b"CREATE TABLE u(id INTEGER PRIMARY KEY, a)")
        .unwrap();
    keyed.run(b"INSERT INTO u VALUES (1,2)").unwrap();
    // The column the key is another name for holds integers, so text
    // that is not a number is refused rather than stored.
    assert!(keyed.run(b"UPDATE u SET id = 'x'").is_err());
    assert!(keyed.run(b"UPDATE u SET id = '7'").is_ok());
}

#[test]
fn a_column_the_table_does_not_hold_is_not_written() {
    use crate::change::Writer;
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a INTEGER)").unwrap();
    writer.run(b"INSERT INTO t VALUES (1)").unwrap();
    assert!(writer.run(b"UPDATE t SET nosuch = 1").is_err());
}

#[test]
fn rows_written_over_by_a_statement_leave_the_file_the_shell_left() {
    use crate::change::Writer;
    // The rows of `shuffled.db` written over three times: a row that
    // keeps its length, a row that shrinks, and a row that grows past
    // the free space its page holds.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
    writer.run(sql_of_rows().as_bytes()).unwrap();
    for sql in [
        "UPDATE t SET n=n+1000 WHERE rowid%5=0",
        "UPDATE t SET s='x' WHERE rowid%7=0",
        "UPDATE t SET s=s||'-longer-text-here' WHERE rowid%11=0",
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    same("updated.db", &writer.written(), crate::tests::UPDATED, 512);
}

#[test]
fn a_statement_that_writes_the_key_takes_the_row_out_and_puts_it_back() {
    use crate::change::Writer;
    // The key moves under both of its names, and the row whose payload
    // ran onto an overflow page shrinks back onto its leaf, which frees
    // the chain.
    let long: alloc::string::String = core::iter::repeat_n('y', 1200).collect();
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(id INTEGER PRIMARY KEY, s TEXT)")
        .unwrap();
    writer
        .run(
            alloc::format!("INSERT INTO t VALUES (1,'a'),(2,'bb'),(3,'ccc'),(4,'{long}'),(5,'e')")
                .as_bytes(),
        )
        .unwrap();
    for sql in [
        "UPDATE t SET s=s||s WHERE id=2",
        "UPDATE t SET id=id+100 WHERE id=3",
        "UPDATE t SET s='short' WHERE id=4",
        "UPDATE t SET rowid=9 WHERE id=5",
    ] {
        writer.run(sql.as_bytes()).unwrap();
    }
    same("moved.db", &writer.written(), crate::tests::MOVED, 512);
}

#[test]
fn a_new_payload_the_length_of_the_old_one_lies_where_the_old_one_lay() {
    use crate::change::Writer;
    // The first row keeps its payload length, so the cell and the chain
    // it runs onto stay and only the bytes change. The second row grows
    // past its leaf onto a chain, and the cell is the length it was
    // because the four bytes of the chain make up for the payload the
    // leaf no longer holds.
    let long: alloc::string::String = core::iter::repeat_n('y', 1200).collect();
    let short: alloc::string::String = core::iter::repeat_n('y', 97).collect();
    let grown: alloc::string::String = core::iter::repeat_n('z', 600).collect();
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(s TEXT)").unwrap();
    writer
        .run(alloc::format!("INSERT INTO t VALUES ('{long}'), ('{short}')").as_bytes())
        .unwrap();
    writer
        .run(b"UPDATE t SET s=replace(s,'y','z') WHERE rowid=1")
        .unwrap();
    writer
        .run(alloc::format!("UPDATE t SET s='{grown}' WHERE rowid=2").as_bytes())
        .unwrap();
    same(
        "overwritten.db",
        &writer.written(),
        crate::tests::OVERWRITTEN,
        512,
    );
}

#[test]
fn a_key_the_table_does_not_hold_writes_over_no_row() {
    use crate::tree::update;
    let mut pages = filled(512);
    let record = crate::record::write(&[Value::Int(1)], &[Affinity::Blob], 4);
    assert!(!update(&mut pages, 2, 401, &record).unwrap());
    assert!(crate::tree::remove(&mut pages, 2, 200).unwrap());
    assert!(!update(&mut pages, 2, 200, &record).unwrap());
}

#[test]
fn the_journal_mode_changes_what_lies_beside_the_file_and_not_what_is_in_it() {
    use crate::change::Writer;
    use crate::journal::Mode;
    // The journal-mode dimension of document 16, section 16.11, over
    // the write path: the same two statements under every mode a
    // rollback journal has. The nonce comes from SQLite's random
    // source, so the one `journalled.db-journal` was written with is
    // read back out of it.
    let theirs = crate::tests::JOURNALLED;
    let run = |mode, nonce| {
        let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
        writer.journalling(mode, nonce, 512);
        writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
        writer.run(sql_of_rows().as_bytes()).unwrap();
        writer.run(b"DELETE FROM t WHERE rowid%4!=0").unwrap();
        writer
    };
    let nonce = nonce_of(theirs, run(Mode::Persist, 0).journal().unwrap());
    for mode in [Mode::Delete, Mode::Memory, Mode::Off] {
        let writer = run(mode, nonce);
        same("emptied.db", &writer.written(), crate::tests::EMPTIED, 512);
        assert_eq!(writer.journal(), None, "{mode:?} left a journal");
    }
    let writer = run(Mode::Truncate, nonce);
    same("emptied.db", &writer.written(), crate::tests::EMPTIED, 512);
    assert_eq!(writer.journal(), Some(&[][..]));
    let writer = run(Mode::Persist, nonce);
    same("emptied.db", &writer.written(), crate::tests::EMPTIED, 512);
    same(
        "journalled.db-journal",
        writer.journal().unwrap(),
        theirs,
        512,
    );
}

#[test]
fn a_statement_run_in_logging_mode_writes_the_log_the_shell_wrote() {
    use crate::change::Writer;
    // The last point of the journal-mode dimension: the file stays as
    // `PRAGMA journal_mode=wal` left it and the three statements are
    // three transactions in the log. The two salts come from SQLite's
    // random source, so the ones the fixture was written with are read
    // back out of its header.
    let theirs = crate::tests::LOGGING_WAL;
    let word = |at: usize| u32::from_be_bytes(theirs[at..at + 4].try_into().unwrap());
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.logging((word(16), word(20)));
    same("logging.db", &writer.written(), crate::tests::LOGGING, 512);
    writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
    writer.run(sql_of_rows().as_bytes()).unwrap();
    writer.run(b"DELETE FROM t WHERE rowid%3=0").unwrap();
    same("logging.db-wal", writer.log().unwrap(), theirs, 512);
    // The file is the one the pragma left, whatever the log holds.
    same("logging.db", &writer.written(), crate::tests::LOGGING, 512);
}

/// The forty rows whose payloads run onto overflow chains, put in by
/// one statement that names the key of each.
fn sql_of_chained() -> alloc::string::String {
    use core::fmt::Write as _;
    let mut sql = alloc::string::String::from("INSERT INTO t(rowid,n,s) VALUES ");
    for number in 1..=40_i64 {
        if number > 1 {
            sql.push(',');
        }
        let wide: alloc::string::String =
            core::iter::repeat_n('x', usize::try_from(number.saturating_mul(60)).unwrap_or(0))
                .collect();
        let _ = write!(sql, "({},{},'{}')", (number * 17) % 41, number, wide);
    }
    sql
}

#[test]
fn the_pointer_maps_a_file_that_vacuums_itself_keeps_are_the_ones_the_shell_wrote() {
    use crate::change::Writer;
    // The auto-vacuum dimension of document 16, section 16.11: page two
    // is the first pointer map, so the first table takes page three,
    // and every page a tree or a chain holds is named in a map. A file
    // that vacuums itself whole moves the pages at its end into the
    // free pages below them at the commit and is cut back; a file that
    // vacuums a step at a time keeps the free list instead.
    for (name, incremental, chained, taken, fixture) in crate::tests::VACUUMING {
        let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
        writer.vacuuming(incremental);
        writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
        let rows = if chained {
            sql_of_chained()
        } else {
            sql_of_rows()
        };
        writer.run(rows.as_bytes()).unwrap();
        if !taken.is_empty() {
            writer.run(taken.as_bytes()).unwrap();
        }
        same(name, &writer.written(), fixture, 512);
    }
}

#[test]
fn the_page_a_pointer_map_lies_at_is_the_one_the_c_library_counts() {
    use crate::tree::map_page;
    // `ptrmapPageno` over pages of 512 usable bytes: one map page
    // carries one entry per five bytes and one more for itself, so the
    // maps lie at page two and every hundred and third page after it.
    assert_eq!(map_page(512, 2), 2);
    assert_eq!(map_page(512, 3), 2);
    assert_eq!(map_page(512, 104), 2);
    assert_eq!(map_page(512, 105), 105);
    assert_eq!(map_page(512, 106), 105);
    // A page size the reserved tail cuts into moves them.
    assert_eq!(map_page(480, 98), 2);
    assert_eq!(map_page(480, 99), 99);
}

#[test]
fn a_chain_steps_past_the_page_a_pointer_map_lies_at() {
    let mut pages = Pages::new(512, 0).unwrap();
    // A file that keeps no pointer maps steps past none.
    assert_eq!(pages.after_maps(104), 105);
    assert!(!pages.is_map(105));
    pages.vacuums();
    assert_eq!(pages.after_maps(1), 2 + 1);
    assert_eq!(pages.after_maps(103), 104);
    assert_eq!(pages.after_maps(104), 106);
    assert!(pages.is_map(105));
}

#[test]
fn what_a_pointer_map_says_about_a_page_reads_back() {
    use crate::tree::Point;
    let mut pages = Pages::new(512, 0).unwrap();
    pages.vacuums();
    // Page two is the map, page three the root of the first tree.
    for _ in 0..4 {
        pages.add(Kind::LeafTable, 0).unwrap();
    }
    for (number, kind, parent) in [
        (3, Point::Root, 0),
        (4, Point::Branch, 3),
        (5, Point::Head, 4),
        (6, Point::Tail, 5),
    ] {
        pages.point(number, kind, parent).unwrap();
        assert_eq!(pages.point_of(number).unwrap(), (kind, parent));
    }
    // A page the map holds no entry for yet is no kind of page.
    assert!(matches!(
        pages.point_of(7),
        Err(crate::error::Error::PageKind(0))
    ));
    // A page freed is named by no page.
    pages.release(6).unwrap();
    assert_eq!(pages.point_of(6).unwrap(), (Point::Free, 0));
}

#[test]
fn the_vacuum_refuses_a_pointer_map_it_cannot_act_on() {
    use crate::tree::Point;
    let mut pages = Pages::new(512, 0).unwrap();
    pages.vacuums();
    // Page two is the map and the seven after it are pages of a tree,
    // so the file holds nine.
    for _ in 0..7 {
        pages.add(Kind::LeafTable, 0).unwrap();
    }
    pages.point(3, Point::Root, 0).unwrap();
    for number in 4..=9 {
        pages.point(number, Point::Branch, 3).unwrap();
    }
    // Two free pages leave the file cut back to page seven, and the
    // map says a root lies past that, which no file holds.
    pages.release(8).unwrap();
    pages.release(9).unwrap();
    pages.point(9, Point::Root, 0).unwrap();
    assert!(matches!(
        pages.vacuum_commit(),
        Err(crate::error::Error::Page(9))
    ));
    // The map saying every page past the end is a page of a tree asks
    // the free list for more pages than it holds.
    pages.point(9, Point::Branch, 3).unwrap();
    pages.point(8, Point::Branch, 3).unwrap();
    assert!(matches!(
        pages.vacuum_commit(),
        Err(crate::error::Error::Balance)
    ));
}

#[test]
fn a_page_no_pointer_map_carries_an_entry_for_is_refused() {
    let mut pages = Pages::new(512, 0).unwrap();
    // A file that keeps no maps answers for no page.
    assert!(pages.point_of(3).is_err());
    pages.vacuums();
    // Page one is named by no map, because the maps begin at page two.
    assert!(pages.point_of(1).is_err());
}

#[test]
fn the_end_a_file_that_vacuums_itself_is_cut_back_to_is_the_one_the_c_library_counts() {
    use crate::tree::{final_size, map_page};
    // `finalDbSize` over pages of 512 usable bytes: the file gives up
    // its free pages and the map pages those free pages leave with no
    // entry to carry.
    assert_eq!(final_size(512, 17, 10), 7);
    assert_eq!(final_size(512, 111, 75), 35);
    // The count of map pages given up is what puts the end past the
    // last of them, so no file is cut back to a page a map lies at.
    for origin in 3..600_u32 {
        for free in 1..origin {
            let last = final_size(512, origin, free);
            assert_ne!(map_page(512, last), last, "{origin} {free}");
        }
    }
}

/// The nonce `theirs` was written with, which is what the checksum of
/// its first record says once the sum of that page is taken off it. A
/// journal this crate wrote with a nonce of nought holds that sum.
///
/// The header takes one sector and the record that follows it is a page
/// number, the page, and the checksum.
fn nonce_at(theirs: &[u8], plain: &[u8], sector: usize, page_size: usize) -> u32 {
    let at = sector.saturating_add(4).saturating_add(page_size);
    let word = |bytes: &[u8]| {
        u32::from_be_bytes(
            bytes
                .get(at..at.saturating_add(4))
                .and_then(|slice| slice.try_into().ok())
                .unwrap_or([0; 4]),
        )
    };
    word(theirs).wrapping_sub(word(plain))
}

#[test]
fn every_pair_of_the_matrix_is_written_under_some_configuration() {
    use crate::journal::Mode;
    use crate::tests::{ARRAY, Keeping};
    // Every value of every dimension appears, and every pair of values
    // from two dimensions appears together, which is what makes thirty
    // configurations stand for the eight hundred and ten the full cross
    // of document 16, section 16.11, would hold.
    let value = |row: &crate::tests::Configuration, dimension: usize| -> u32 {
        match dimension {
            0 => match row.encoding {
                Encoding::Utf8 => 0,
                Encoding::Utf16Le => 1,
                Encoding::Utf16Be => 2,
            },
            1 => row.page_size,
            2 => u32::from(row.reserved),
            3 => match row.keeping {
                Keeping::Journal(Mode::Delete) => 0,
                Keeping::Journal(Mode::Truncate) => 1,
                Keeping::Journal(Mode::Persist) => 2,
                Keeping::Journal(Mode::Memory) => 3,
                Keeping::Journal(Mode::Off) => 4,
                Keeping::Log => 5,
            },
            _ => match row.vacuum {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            },
        }
    };
    let widths = [3, 5, 3, 6, 3];
    let mut pairs = 0;
    for one in 0..widths.len() {
        for other in one.saturating_add(1)..widths.len() {
            let mut seen: Vec<(u32, u32)> = Vec::new();
            for row in &ARRAY {
                let pair = (value(row, one), value(row, other));
                if !seen.contains(&pair) {
                    seen.push(pair);
                }
            }
            let want =
                widths.get(one).copied().unwrap_or(0) * widths.get(other).copied().unwrap_or(0);
            assert_eq!(seen.len(), want, "dimensions {one} and {other}");
            pairs += seen.len();
        }
    }
    assert_eq!(pairs, 156);
}

#[test]
fn the_covering_array_of_the_matrix_writes_the_files_the_shell_wrote() {
    use crate::change::Writer;
    use crate::journal::Mode;
    use crate::tests::{ARRAY, Keeping};
    // The write path under thirty configurations, each holding the same
    // four hundred rows put in by a key that jumps about. The nonce of
    // a journal and the two salts of a log come from SQLite's random
    // source, so each is read back out of the file the shell wrote.
    for row in &ARRAY {
        let page_size = usize::try_from(row.page_size).unwrap();
        let build = |nonce: u32| {
            let mut writer = Writer::new(row.page_size, row.reserved, row.encoding).unwrap();
            if let Some(incremental) = row.vacuum {
                writer.vacuuming(incremental);
            }
            match row.keeping {
                Keeping::Journal(mode) => writer.journalling(mode, nonce, 512),
                Keeping::Log => {
                    let word = |at: usize| {
                        u32::from_be_bytes(
                            row.beside
                                .and_then(|bytes| bytes.get(at..at.saturating_add(4)))
                                .and_then(|slice| slice.try_into().ok())
                                .unwrap_or([0; 4]),
                        )
                    };
                    writer.logging((word(16), word(20)));
                }
            }
            writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
            writer.run(sql_of_rows().as_bytes()).unwrap();
            writer
        };
        // A journal that keeps its records is written twice: once to
        // read the nonce out of the file the shell wrote, and once with
        // that nonce.
        let writer = match (row.keeping, row.beside) {
            (Keeping::Journal(Mode::Persist), Some(theirs)) => {
                let plain = build(0);
                let nonce = nonce_at(theirs, plain.journal().unwrap(), 512, page_size);
                build(nonce)
            }
            _ => build(0),
        };
        same(row.name, &writer.written(), row.bytes, page_size);
        match (row.keeping, row.beside) {
            (Keeping::Log, Some(theirs)) => {
                let name = alloc::format!("{}-wal", row.name);
                same(&name, writer.log().unwrap(), theirs, page_size);
            }
            (Keeping::Journal(_), Some(theirs)) => {
                let name = alloc::format!("{}-journal", row.name);
                same(&name, writer.journal().unwrap(), theirs, page_size);
            }
            // A log is always read back, so a row that keeps one names
            // the file it lies in.
            (Keeping::Journal(_) | Keeping::Log, None) => {
                assert_eq!(writer.journal(), None, "{}", row.name);
                assert_eq!(writer.log(), None, "{}", row.name);
            }
        }
    }
}

#[test]
fn a_column_the_statement_names_no_value_for_holds_what_it_falls_back_to() {
    use crate::change::Writer;
    // `defaults.db`: a statement that names one of three columns leaves
    // the other two holding what they fall back to, which is what
    // `sqlite3ExprCodeGetColumnOfTable` writes for a column the
    // statement passed over.
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a, b DEFAULT 7, c TEXT DEFAULT 'z')")
        .unwrap();
    writer.run(b"INSERT INTO t(a) VALUES(1)").unwrap();
    same(
        "defaults.db",
        &writer.written(),
        crate::tests::DEFAULTS,
        4096,
    );
    // A table the database does not hold has no column to fall back.
    let bytes = writer.written();
    let database = crate::db::Database::open(&bytes).unwrap();
    assert!(database.defaults(b"nosuch").unwrap().is_empty());
    assert_eq!(database.defaults(b"t").unwrap().len(), 3);
}

#[test]
fn the_schema_format_a_file_names_is_the_one_its_rows_were_written_under() {
    // The schema format dimension of document 16, section 16.11. The
    // formats differ in what they store rather than in what they
    // answer, so the bytes are what the test reads: format 4 stores the
    // whole numbers 0 and 1 with no payload at all, under serial types
    // 8 and 9, and format 1 stores a byte for each.
    let format = |bytes: &[u8]| crate::bytes::u32_at(bytes, 44).unwrap_or(0);
    assert_eq!(format(crate::tests::FORMAT1), 1);
    assert_eq!(format(crate::tests::FORMAT3), 3);
    assert_eq!(format(crate::tests::FORMAT4), 4);
    let first_row = |bytes: &[u8]| {
        let page = crate::page::Page::parse(bytes.get(4096..8192).unwrap(), 2, 4096).unwrap();
        page.row(0).unwrap().1.total
    };
    assert_eq!(first_row(crate::tests::FORMAT1), 7);
    assert_eq!(first_row(crate::tests::FORMAT4), 5);
    // What the two answer for those rows is the same, which is what the
    // twelve cases of `query.corpus` over the three files hold.
    let rows = |bytes: &'static [u8]| {
        crate::db::Database::open(bytes)
            .unwrap()
            .query(b"SELECT a, b, c FROM t")
            .unwrap()
            .rows
    };
    assert_eq!(rows(crate::tests::FORMAT1), rows(crate::tests::FORMAT4));
}

#[test]
fn the_pragmas_that_configure_a_file_write_what_the_calls_write() {
    use crate::change::Writer;
    // `PRAGMA` is how SQLite is configured, and the three calls that
    // stand where the pragmas stand write the same file, so every
    // configuration of the covering array that needs no salt is
    // written both ways and the two are held to each other.
    for (name, incremental, chained, taken, fixture) in crate::tests::VACUUMING {
        let vacuum = if incremental { "incremental" } else { "full" };
        let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
        assert!(writer.run(b"PRAGMA page_size=512").unwrap().is_empty());
        assert!(writer.run(b"PRAGMA encoding='UTF-8'").unwrap().is_empty());
        assert!(
            writer
                .run(alloc::format!("PRAGMA auto_vacuum={vacuum}").as_bytes())
                .unwrap()
                .is_empty()
        );
        writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
        let rows = if chained {
            sql_of_chained()
        } else {
            sql_of_rows()
        };
        writer.run(rows.as_bytes()).unwrap();
        if !taken.is_empty() {
            writer.run(taken.as_bytes()).unwrap();
        }
        same(name, &writer.written(), fixture, 512);
    }
}

#[test]
fn what_a_pragma_answers_out_of_a_file() {
    use crate::change::Writer;
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    // The journal mode is the one pragma that answers the mode it left
    // the connection in.
    assert_eq!(
        writer.run(b"PRAGMA journal_mode=memory").unwrap(),
        alloc::vec![alloc::vec![Value::Text(b"memory".to_vec())]]
    );
    // One the file does not hold is answered and changes nothing.
    assert!(writer.run(b"PRAGMA cache_size=-4000").unwrap().is_empty());
    writer.run(b"PRAGMA page_size=512").unwrap();
    writer.run(b"PRAGMA encoding='UTF-16le'").unwrap();
    writer.run(b"PRAGMA auto_vacuum=incremental").unwrap();
    writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
    let written = writer.written();
    let database = crate::db::Database::open(&written).unwrap();
    let asked = |sql: &[u8]| database.query(sql).unwrap().rows;
    assert_eq!(asked(b"PRAGMA page_size"), [[Value::Int(512)]]);
    assert_eq!(
        asked(b"PRAGMA encoding"),
        [[Value::Text(b"UTF-16le".to_vec())]]
    );
    assert_eq!(asked(b"PRAGMA auto_vacuum"), [[Value::Int(2)]]);
    assert_eq!(
        asked(b"PRAGMA journal_mode"),
        [[Value::Text(b"delete".to_vec())]]
    );
    assert_eq!(asked(b"PRAGMA page_count"), [[Value::Int(3)]]);
    assert_eq!(asked(b"PRAGMA freelist_count"), [[Value::Int(0)]]);
    assert_eq!(asked(b"PRAGMA schema_version"), [[Value::Int(1)]]);
    assert_eq!(asked(b"PRAGMA user_version"), [[Value::Int(0)]]);
    assert_eq!(asked(b"PRAGMA application_id"), [[Value::Int(0)]]);
    assert_eq!(asked(b"PRAGMA schema_format"), [[Value::Int(4)]]);
    assert_eq!(asked(b"PRAGMA reserved_bytes"), [[Value::Int(0)]]);
    // The three encodings, and the setting that turns the vacuuming
    // off, which is what a file with no pointer maps is written under.
    for (written, read) in [
        ("'UTF-8'", b"UTF-8".as_slice()),
        ("'UTF-16be'", b"UTF-16be"),
        ("'utf16le'", b"UTF-16le"),
    ] {
        let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
        writer
            .run(alloc::format!("PRAGMA encoding={written}").as_bytes())
            .unwrap();
        writer.run(b"PRAGMA auto_vacuum=none").unwrap();
        // A pragma that sets nothing is read out of the connection,
        // which holds the encoding a file with no table does not.
        assert_eq!(
            writer.run(b"PRAGMA page_size").unwrap(),
            [[Value::Int(512)]]
        );
        assert_eq!(
            writer.run(b"PRAGMA encoding").unwrap(),
            [[Value::Text(read.to_vec())]]
        );
        writer.run(b"CREATE TABLE t(a)").unwrap();
        let written = writer.written();
        let database = crate::db::Database::open(&written).unwrap();
        assert_eq!(
            database.query(b"PRAGMA encoding").unwrap().rows,
            [[Value::Text(read.to_vec())]]
        );
        assert_eq!(
            database.query(b"PRAGMA auto_vacuum").unwrap().rows,
            [[Value::Int(0)]]
        );
    }
    // A file that has been in write-ahead logging says so.
    let mut logged = Writer::new(512, 0, Encoding::Utf8).unwrap();
    logged.logging((1, 2));
    let written = logged.written();
    let database = crate::db::Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"PRAGMA journal_mode").unwrap().rows,
        [[Value::Text(b"wal".to_vec())]]
    );
}

#[test]
fn what_a_drop_leaves_is_the_file_the_shell_wrote() {
    use crate::change::Writer;
    use core::fmt::Write as _;
    let wide: alloc::string::String = core::iter::repeat_n('a', 1800).collect();
    let other: alloc::string::String = core::iter::repeat_n('b', 1800).collect();
    for (name, shape, fixture) in crate::tests::DROPPED {
        let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
        match *shape {
            "deep" | "indexed" | "index" => {
                writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
                if *shape == "deep" {
                    writer.run(sql_of_rows().as_bytes()).unwrap();
                } else {
                    let mut sql = alloc::string::String::from("INSERT INTO t(rowid,n,s) VALUES ");
                    for number in 1..=60_i64 {
                        if number > 1 {
                            sql.push(',');
                        }
                        let _ = write!(sql, "({number},{number},'row {number}')");
                    }
                    writer.run(sql.as_bytes()).unwrap();
                    writer.run(b"CREATE INDEX ts ON t(s)").unwrap();
                }
            }
            "wide" => {
                writer.run(b"CREATE TABLE t(a)").unwrap();
                writer
                    .run(alloc::format!("INSERT INTO t VALUES('{wide}'),('{other}')").as_bytes())
                    .unwrap();
            }
            "last" => {
                writer.run(b"CREATE TABLE t(a)").unwrap();
                writer.run(b"INSERT INTO t VALUES(1)").unwrap();
            }
            _ => {
                writer.run(b"CREATE TABLE t(a,b)").unwrap();
                writer.run(b"INSERT INTO t VALUES(1,'x'),(2,'y')").unwrap();
            }
        }
        if *shape != "index" {
            writer.run(b"CREATE TABLE u(c)").unwrap();
            if *shape != "wide" {
                writer.run(b"INSERT INTO u VALUES(9)").unwrap();
            }
        }
        let statement: &[u8] = match *shape {
            "index" => b"DROP INDEX ts",
            "last" => b"DROP TABLE u",
            _ => b"DROP TABLE t",
        };
        writer.run(statement).unwrap();
        same(name, &writer.written(), fixture, 512);
    }
}

#[test]
fn a_statement_answers_how_many_rows_it_changed_where_the_pragma_says_so() {
    use crate::change::Writer;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    // The pragma is off to begin with, so a statement answers no row.
    assert_eq!(
        writer.run(b"PRAGMA count_changes").unwrap(),
        [[Value::Int(0)]]
    );
    assert!(
        writer
            .run(b"INSERT INTO t VALUES(1,'x'),(2,'y'),(3,'z')")
            .unwrap()
            .is_empty()
    );
    writer.run(b"PRAGMA count_changes=ON").unwrap();
    assert_eq!(
        writer.run(b"PRAGMA count_changes").unwrap(),
        [[Value::Int(1)]]
    );
    assert_eq!(
        writer.run(b"INSERT INTO t VALUES(4,'w')").unwrap(),
        [[Value::Int(1)]]
    );
    assert_eq!(
        writer.run(b"UPDATE t SET b='q' WHERE a<3").unwrap(),
        [[Value::Int(2)]]
    );
    assert_eq!(
        writer.run(b"DELETE FROM t WHERE a>2").unwrap(),
        [[Value::Int(2)]]
    );
    // A statement that changes no row answers nought, and one that
    // makes a table answers nothing of its own.
    assert_eq!(
        writer.run(b"DELETE FROM t WHERE a>99").unwrap(),
        [[Value::Int(0)]]
    );
    assert_eq!(writer.run(b"CREATE TABLE u(c)").unwrap(), [[Value::Int(0)]]);
    // A number says it as well, which is what `sqlite3GetBoolean`
    // takes for one.
    writer.run(b"PRAGMA count_changes=0").unwrap();
    assert!(writer.run(b"DELETE FROM t WHERE a=99").unwrap().is_empty());
    writer.run(b"PRAGMA count_changes=1").unwrap();
    assert_eq!(
        writer.run(b"DELETE FROM t WHERE a=2").unwrap(),
        [[Value::Int(1)]]
    );
    writer.run(b"PRAGMA count_changes=off").unwrap();
    assert!(writer.run(b"DELETE FROM t WHERE a=1").unwrap().is_empty());
    // A value that is not a truth is refused.
    assert!(writer.run(b"PRAGMA count_changes=maybe").is_err());
}

#[test]
fn a_transaction_writes_what_its_statements_write_and_counts_once() {
    use crate::change::Writer;
    use core::fmt::Write as _;
    // The statements between a `BEGIN` and a `COMMIT` write the file
    // the same statements write on their own; the change counter is
    // what says the transaction was one unit of work.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x')").unwrap();
    writer.run(b"INSERT INTO t VALUES(2,'y')").unwrap();
    writer.run(b"COMMIT").unwrap();
    same("tx-one.db", &writer.written(), crate::tests::ONE_UNIT, 512);
    // A transaction that frees pages and takes one of them back leaves
    // the free list where the same statements leave it.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
    let mut sql = alloc::string::String::from("INSERT INTO t(rowid,n,s) VALUES ");
    for number in 1..=60_i64 {
        if number > 1 {
            sql.push(',');
        }
        let _ = write!(sql, "({number},{number},'row {number}')");
    }
    writer.run(sql.as_bytes()).unwrap();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"DELETE FROM t WHERE n%3=0").unwrap();
    writer
        .run(b"INSERT INTO t(rowid,n,s) VALUES(500,500,'row 500')")
        .unwrap();
    writer.run(b"COMMIT").unwrap();
    same("tx-grown.db", &writer.written(), crate::tests::GROWN, 512);
}

#[test]
fn a_rollback_leaves_the_file_the_transaction_began_with() {
    use crate::change::Writer;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x')").unwrap();
    let before = writer.written();
    writer.run(b"BEGIN").unwrap();
    writer.run(b"INSERT INTO t VALUES(2,'y')").unwrap();
    writer.run(b"DELETE FROM t WHERE a=1").unwrap();
    writer.run(b"ROLLBACK").unwrap();
    let after = writer.written();
    assert_eq!(before, after, "the rollback left the file changed");
    same("tx-back.db", &after, crate::tests::ROLLED_BACK, 512);
    // The rows are the ones the transaction began with, and the
    // connection writes on from there.
    let database = crate::db::Database::open(&after).unwrap();
    assert_eq!(
        database.query(b"SELECT a FROM t").unwrap().rows,
        [[Value::Int(1)]]
    );
    writer.run(b"INSERT INTO t VALUES(3,'z')").unwrap();
    let written = writer.written();
    let database = crate::db::Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"SELECT a FROM t").unwrap().rows,
        [[Value::Int(1)], [Value::Int(3)]]
    );
}

#[test]
fn what_bounding_a_transaction_refuses() {
    use crate::change::Writer;
    use crate::db::Error;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    // A `COMMIT` and a `ROLLBACK` outside a transaction have none to
    // bound.
    assert_eq!(writer.run(b"COMMIT"), Err(Error::NoTransaction));
    assert_eq!(writer.run(b"ROLLBACK"), Err(Error::NoTransaction));
    writer.run(b"BEGIN").unwrap();
    // A transaction does not open inside a transaction.
    assert_eq!(writer.run(b"BEGIN"), Err(Error::Nested));
    writer.run(b"END").unwrap();
    // The words that stand beside each of the three.
    for sql in [
        b"BEGIN DEFERRED TRANSACTION".as_slice(),
        b"COMMIT TRANSACTION",
        b"BEGIN IMMEDIATE",
        b"ROLLBACK TRANSACTION",
        b"BEGIN EXCLUSIVE TRANSACTION",
        b"END TRANSACTION",
    ] {
        writer.run(sql).unwrap();
    }
    // A `ROLLBACK TO` names a savepoint, which is a statement of its
    // own.
    writer.run(b"BEGIN").unwrap();
    assert!(writer.run(b"ROLLBACK TO one").is_err());
}

#[test]
fn a_view_answers_the_statement_it_names() {
    use crate::change::Writer;
    use crate::db::Database;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES(1,'x'),(2,'y'),(3,'z')")
        .unwrap();
    writer
        .run(b"CREATE VIEW v AS SELECT a FROM t WHERE a>1")
        .unwrap();
    let written = writer.written();
    same("view-one.db", &written, crate::tests::VIEW_ONE, 512);
    // The rows are the ones the statement answers, read where the view
    // is named.
    let database = Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"SELECT a FROM v").unwrap().rows,
        [[Value::Int(2)], [Value::Int(3)]]
    );
    // A view stands where a table stands: in a join, under a `WHERE`,
    // and under an aggregate.
    assert_eq!(
        database.query(b"SELECT count(*) FROM v").unwrap().rows,
        [[Value::Int(2)]]
    );
    assert_eq!(
        database
            .query(b"SELECT v.a, t.b FROM v JOIN t ON t.a=v.a ORDER BY v.a")
            .unwrap()
            .rows,
        [
            alloc::vec![Value::Int(2), Value::Text(b"y".to_vec())],
            alloc::vec![Value::Int(3), Value::Text(b"z".to_vec())]
        ]
    );
    // A view reads what the table holds now, because its statement is
    // answered where it is named.
    writer.run(b"INSERT INTO t VALUES(4,'w')").unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"SELECT count(*) FROM v").unwrap().rows,
        [[Value::Int(3)]]
    );
}

#[test]
fn a_view_answers_its_columns_under_the_names_it_was_written_with() {
    use crate::change::Writer;
    use crate::db::Database;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x'),(2,'y')").unwrap();
    writer
        .run(b"CREATE VIEW v(one,two) AS SELECT b,a FROM t ORDER BY a DESC")
        .unwrap();
    let written = writer.written();
    same("view-named.db", &written, crate::tests::VIEW_NAMED, 512);
    let database = Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"SELECT one,two FROM v").unwrap().rows,
        [
            alloc::vec![Value::Text(b"y".to_vec()), Value::Int(2)],
            alloc::vec![Value::Text(b"x".to_vec()), Value::Int(1)]
        ]
    );
    // A `DROP VIEW` takes the row away and leaves no tree behind,
    // because a view names none.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x')").unwrap();
    writer.run(b"CREATE VIEW v AS SELECT a FROM t").unwrap();
    writer.run(b"DROP VIEW v").unwrap();
    let written = writer.written();
    same("view-gone.db", &written, crate::tests::VIEW_GONE, 512);
    let database = Database::open(&written).unwrap();
    assert!(database.query(b"SELECT a FROM v").is_err());
}

#[test]
fn what_a_view_refuses() {
    use crate::change::Writer;
    use crate::db::{Database, Error};
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"CREATE VIEW v AS SELECT a FROM t").unwrap();
    // A view is not a table and a table is not a view.
    assert!(writer.run(b"DROP VIEW t").is_err());
    assert!(writer.run(b"DROP TABLE v").is_err());
    assert!(writer.run(b"DROP VIEW nosuch").is_err());
    writer.run(b"DROP VIEW IF EXISTS nosuch").unwrap();
    // A view that names itself is one the reader stops in rather than
    // following forever.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE VIEW v AS SELECT a FROM v").unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"SELECT a FROM v").err(),
        Some(Error::Unsupported)
    );
}

#[test]
fn a_column_added_to_a_table_is_the_statement_the_shell_wrote() {
    use crate::change::Writer;
    for (name, statement, fixture) in crate::tests::ADDED_COLUMN {
        let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
        if *name == "alter-typed.db" {
            writer
                .run(b"CREATE TABLE t(n INTEGER, s TEXT, PRIMARY KEY(n))")
                .unwrap();
        } else {
            writer.run(b"CREATE TABLE t(a,b)").unwrap();
        }
        writer.run(b"INSERT INTO t VALUES(1,'x')").unwrap();
        writer.run(statement.as_bytes()).unwrap();
        same(name, &writer.written(), fixture, 512);
    }
}

#[test]
fn a_row_written_before_a_column_was_added_answers_what_it_falls_back_to() {
    use crate::change::Writer;
    use crate::db::Database;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'x')").unwrap();
    writer.run(b"ALTER TABLE t ADD COLUMN c DEFAULT 7").unwrap();
    // The row holds two values and the table has three columns, so the
    // third answers what it falls back to.
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"SELECT a,b,c FROM t").unwrap().rows,
        [alloc::vec![
            Value::Int(1),
            Value::Text(b"x".to_vec()),
            Value::Int(7)
        ]]
    );
    // A row written after it holds a value of its own for the column.
    writer.run(b"INSERT INTO t VALUES(2,'y',9)").unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"SELECT c FROM t ORDER BY a").unwrap().rows,
        [[Value::Int(7)], [Value::Int(9)]]
    );
}

#[test]
fn what_adding_a_column_refuses() {
    use crate::change::Writer;
    use crate::db::Error;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    // A table the database does not hold.
    assert_eq!(
        writer.run(b"ALTER TABLE nosuch ADD COLUMN c"),
        Err(Error::NoTable)
    );
    // `sqlite3AlterFinishAddColumn` refuses a column that would need an
    // index over the rows the table already holds, and one that may not
    // be nothing where the rows hold nothing for it.
    assert_eq!(
        writer.run(b"ALTER TABLE t ADD COLUMN c UNIQUE"),
        Err(Error::Unsupported)
    );
    assert_eq!(
        writer.run(b"ALTER TABLE t ADD COLUMN c PRIMARY KEY"),
        Err(Error::Unsupported)
    );
    assert_eq!(
        writer.run(b"ALTER TABLE t ADD COLUMN c NOT NULL"),
        Err(Error::Unsupported)
    );
    // One that may not be nothing and falls back to something is
    // allowed, because every row then holds one.
    writer
        .run(b"ALTER TABLE t ADD COLUMN d DEFAULT 3 NOT NULL")
        .unwrap();
    // The row the statement writes again is found by name and by kind,
    // so the walk passes over a table of another name and over a row
    // that is not a table at all.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"CREATE INDEX ta ON t(a)").unwrap();
    writer.run(b"CREATE TABLE u(b)").unwrap();
    // The text of the column is kept as it was written, with the
    // trailing semicolon and the space before it taken off.
    writer
        .run(b"ALTER TABLE u ADD COLUMN c DEFAULT 7 ;  ")
        .unwrap();
    let written = writer.written();
    let database = crate::db::Database::open(&written).unwrap();
    assert_eq!(
        database.written_as(b"u").map(|(sql, _)| sql),
        Some(b"CREATE TABLE u(b, c DEFAULT 7)".as_slice())
    );
}

#[test]
fn true_and_false_are_numbers_where_no_column_answers_to_them() {
    use crate::change::Writer;
    use crate::db::Database;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES(1,TRUE),(2,false),(3,NULL)")
        .unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"SELECT b FROM t ORDER BY a").unwrap().rows,
        [[Value::Int(1)], [Value::Int(0)], [Value::Null]]
    );
    // They are numbers, so they count and compare as numbers.
    assert_eq!(
        database
            .query(b"SELECT true, false, TRUE+1, typeof(true)")
            .unwrap()
            .rows,
        [alloc::vec![
            Value::Int(1),
            Value::Int(0),
            Value::Int(2),
            Value::Text(b"integer".to_vec())
        ]]
    );
    // A column of that name answers instead, which is what makes this
    // the fallback and not the rule.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE u(true)").unwrap();
    writer.run(b"INSERT INTO u VALUES(9)").unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"SELECT true FROM u").unwrap().rows,
        [[Value::Int(9)]]
    );
    // A name in quotes is a column and never a number, under any of the
    // four quotes, and a name with a table in front of it is a column
    // as well.
    for sql in [
        b"SELECT \"true\" FROM u".as_slice(),
        b"SELECT [true] FROM u",
        b"SELECT `true` FROM u",
        b"SELECT u.true FROM u",
        b"SELECT \"u\".\"true\" FROM u",
    ] {
        assert_eq!(database.query(sql).unwrap().rows, [[Value::Int(9)]]);
    }
    // A name no table answers to, in quotes, is refused rather than
    // read as a number.
    assert!(database.query(b"SELECT \"false\" FROM u").is_err());
    assert!(database.query(b"SELECT t.true FROM u").is_err());
}

#[test]
fn a_column_is_named_with_its_quotes_off() {
    use crate::change::Writer;
    use crate::db::Database;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1)").unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    for sql in [
        b"SELECT a FROM t".as_slice(),
        b"SELECT \"a\" FROM t",
        b"SELECT [a] FROM t",
        b"SELECT `a` FROM t",
        b"SELECT \"t\".\"a\" FROM \"t\"",
        b"SELECT a FROM t ORDER BY \"a\"",
        b"SELECT a AS m FROM t ORDER BY \"m\"",
        b"SELECT a FROM t GROUP BY \"a\"",
        b"SELECT a COLLATE \"nocase\" FROM t",
        b"SELECT \"count\"(*) FROM t",
        b"SELECT CAST(a AS \"int\") FROM t",
        b"SELECT \"abs\"(a) FROM t",
    ] {
        assert_eq!(database.query(sql).unwrap().rows, [[Value::Int(1)]]);
    }
}

#[test]
fn what_a_drop_refuses() {
    use crate::change::Writer;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    // A name the database does not hold, unless the statement allows it.
    assert!(writer.run(b"DROP TABLE nosuch").is_err());
    assert!(writer.run(b"DROP INDEX nosuch").is_err());
    writer.run(b"DROP TABLE IF EXISTS nosuch").unwrap();
    writer.run(b"DROP INDEX IF EXISTS nosuch").unwrap();
    // A table is not an index and an index is not a table.
    assert!(writer.run(b"DROP INDEX t").is_err());
}

#[test]
fn what_a_pragma_refuses() {
    use crate::change::Writer;
    use crate::parse::pragma;
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    // A pragma this crate does not answer, and a value it does not
    // name.
    assert!(writer.run(b"PRAGMA nosuch=1").is_err());
    assert!(writer.run(b"PRAGMA auto_vacuum=sometimes").is_err());
    assert!(writer.run(b"PRAGMA journal_mode=nosuch").is_err());
    assert!(writer.run(b"PRAGMA encoding='UTF-32'").is_err());
    assert!(writer.run(b"PRAGMA page_size=five").is_err());
    assert!(writer.run(b"PRAGMA page_size=99999999999").is_err());
    // The page count is what the file holds, not what a statement sets.
    assert!(writer.run(b"PRAGMA page_count=7").is_err());
    // The three that say how the first table is written stand before
    // it, and one written after it changes nothing.
    writer.run(b"PRAGMA auto_vacuum=full").unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    assert!(writer.run(b"PRAGMA page_size=512").unwrap().is_empty());
    assert_eq!(
        writer.run(b"PRAGMA page_size").unwrap(),
        [[Value::Int(4096)]]
    );
    let written = writer.written();
    let database = crate::db::Database::open(&written).unwrap();
    // A file being read is not being configured.
    assert!(database.query(b"PRAGMA page_size=1024").is_err());
    assert!(database.query(b"PRAGMA nosuch").is_err());
    // The pragma the connection holds has no answer out of a file, and
    // a pragma that answers nothing at all has none either.
    assert!(database.query(b"PRAGMA count_changes").is_err());
    assert!(database.query(b"PRAGMA cache_spill").is_err());
    // The journal mode belongs to the connection, so it is set after a
    // table is there as well; a file in write-ahead logging leaves that
    // mode through a checkpoint, which this crate does not write.
    let mut logging = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    logging.run(b"CREATE TABLE t(a)").unwrap();
    for mode in [
        b"delete".as_slice(),
        b"truncate",
        b"persist",
        b"memory",
        b"off",
    ] {
        let sql = alloc::format!(
            "PRAGMA journal_mode={}",
            alloc::string::String::from_utf8_lossy(mode)
        );
        assert_eq!(
            logging.run(sql.as_bytes()).unwrap(),
            [[Value::Text(mode.to_vec())]]
        );
    }
    assert_eq!(
        logging.run(b"PRAGMA journal_mode=wal").unwrap(),
        [[Value::Text(b"wal".to_vec())]]
    );
    // The pragma answers the same for a connection already logging.
    assert_eq!(
        logging.run(b"PRAGMA journal_mode=wal").unwrap(),
        [[Value::Text(b"wal".to_vec())]]
    );
    assert!(logging.run(b"PRAGMA journal_mode=delete").is_err());
    assert!(logging.log().is_some());
    // What the reader of a pragma refuses.
    assert!(pragma(b"PRAGMA main.page_size").is_ok());
    assert!(pragma(b"PRAGMA page_size(512)").is_ok());
    assert!(pragma(b"PRAGMA page_size;").is_ok());
    assert!(pragma(b"PRAGMA").is_err());
    assert!(pragma(b"PRAGMA page_size(512").is_err());
    assert!(pragma(b"PRAGMA page_size extra").is_err());
    assert!(pragma(b"SELECT 1").is_err());
}

#[test]
fn the_index_trees_a_statement_writes_are_the_ones_the_shell_wrote() {
    use crate::change::Writer;
    use core::fmt::Write as _;
    // `CREATE INDEX` sorts its entries before it writes any, so the
    // pages fill in the order the entries run and the left one is
    // filled before the right one is begun, which is what
    // `BTREE_BULKLOAD` asks for.
    for (name, rows, index, fixture) in crate::tests::TREES {
        let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
        let create: &[u8] = match rows {
            "shuffled" | "split" => b"CREATE TABLE t(n INTEGER, s TEXT)",
            _ => b"CREATE TABLE t(a,b)",
        };
        writer.run(create).unwrap();
        match rows {
            "small" => {
                writer
                    .run(b"INSERT INTO t VALUES(3,'c'),(1,'a'),(2,'b')")
                    .unwrap();
            }
            "shuffled" => {
                writer.run(sql_of_rows().as_bytes()).unwrap();
            }
            "repeated" => {
                writer
                    .run(b"INSERT INTO t VALUES('x',1),('x',2),('y',3)")
                    .unwrap();
            }
            "wide" => {
                let wide: alloc::string::String = core::iter::repeat_n('a', 800).collect();
                writer
                    .run(alloc::format!("INSERT INTO t VALUES('{wide}',1),('b',2)").as_bytes())
                    .unwrap();
            }
            "split" => {
                let mut sql = alloc::string::String::from("INSERT INTO t(rowid,n,s) VALUES ");
                for number in 1..=60_i64 {
                    if number > 1 {
                        sql.push(',');
                    }
                    let _ = write!(sql, "({number},{number},'row {number}')");
                }
                writer.run(sql.as_bytes()).unwrap();
            }
            _ => {}
        }
        writer.run(index.as_bytes()).unwrap();
        same(name, &writer.written(), fixture, 512);
    }
}

#[test]
fn the_entries_an_index_gains_as_rows_are_put_in_are_the_ones_the_shell_wrote() {
    use crate::change::Writer;
    // An index made before the rows gains an entry per row as each is
    // written, which is what `sqlite3GenerateConstraintChecks` writes
    // beside the row.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"CREATE INDEX ta ON t(a)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES(3,'c'),(1,'a'),(2,'b'),(2,'d')")
        .unwrap();
    same("index-kept.db", &writer.written(), crate::tests::KEPT, 512);

    // Over four hundred rows the index has a page above its leaves, so
    // an entry that belongs in a leaf that is not the last is reached
    // through a child pointer rather than through the right pointer.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
    writer.run(sql_of_rows().as_bytes()).unwrap();
    writer.run(b"CREATE INDEX ts ON t(s)").unwrap();
    writer
        .run(b"INSERT INTO t(rowid,n,s) VALUES(500,500,'row 1 and a half'),(501,501,'row 999')")
        .unwrap();
    same(
        "index-added.db",
        &writer.written(),
        crate::tests::ADDED,
        512,
    );

    // Every storage class stands in an index key, and two rows sharing
    // one are held apart by the key of the row.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"CREATE INDEX ta ON t(a)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES(NULL,1),(2.5,2),(x'0102',3),('t',4),(7,5),(NULL,6),(2.5,7)")
        .unwrap();
    same(
        "index-classes.db",
        &writer.written(),
        crate::tests::CLASSES,
        512,
    );
}

/// A writer over a table of `rows` rows with an index over its text,
/// which is what the tests of a statement over an indexed table begin
/// from.
fn indexed(rows: i64) -> crate::change::Writer {
    use core::fmt::Write as _;
    let mut writer = crate::change::Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
    if rows == 400 {
        writer.run(sql_of_rows().as_bytes()).unwrap();
    } else {
        let mut sql = alloc::string::String::from("INSERT INTO t(rowid,n,s) VALUES ");
        for number in 1..=rows {
            if number > 1 {
                sql.push(',');
            }
            let _ = write!(sql, "({number},{number},'row {number}')");
        }
        writer.run(sql.as_bytes()).unwrap();
    }
    writer.run(b"CREATE INDEX ts ON t(s)").unwrap();
    writer
}

#[test]
fn the_entries_a_delete_takes_out_of_an_index_are_the_ones_the_shell_wrote() {
    // An entry on a leaf is dropped and the leaf is balanced, which is
    // the shorter branch of `sqlite3BtreeDelete`.
    let mut writer = indexed(60);
    writer.run(b"DELETE FROM t WHERE n%3=0").unwrap();
    same("index-gone.db", &writer.written(), crate::tests::GONE, 512);
    // An entry on the page above the leaves is replaced by the entry
    // before it, which lies at the end of the right-most leaf under the
    // page the entry names.
    let mut writer = indexed(400);
    writer.run(b"DELETE FROM t WHERE n%2=0").unwrap();
    same(
        "index-hollow.db",
        &writer.written(),
        crate::tests::HOLLOW,
        512,
    );
    // A table emptied leaves its index one root page holding no entry.
    let mut writer = indexed(60);
    writer.run(b"DELETE FROM t WHERE n>0").unwrap();
    same(
        "index-emptied.db",
        &writer.written(),
        crate::tests::SWEPT,
        512,
    );
}

#[test]
fn an_entry_moved_up_into_a_page_with_no_room_for_it_is_balanced_with_it() {
    use crate::page::{Cell, Payload, write_cell};
    use crate::tree::{find_entry, remove_entry};
    use core::fmt::Write as _;
    // A root nearly full of short dividers, over thirteen leaves of one
    // entry each, the first of which is as long as an entry lies on a
    // page. Taking the first divider out moves that entry up, and the
    // root has no room for it, so the balance of the leaf reads it as
    // one of its dividers.
    let mut pages = Pages::new(512, 0).unwrap();
    let root = pages.add(Kind::InteriorIndex, 0).unwrap();
    let mut keys: alloc::vec::Vec<alloc::vec::Vec<Value>> = alloc::vec::Vec::new();
    let mut leaves: alloc::vec::Vec<u32> = alloc::vec::Vec::new();
    for number in 0..13_i64 {
        let leaf = pages.add(Kind::LeafIndex, 0).unwrap();
        leaves.push(leaf);
        let mut text = alloc::string::String::new();
        let _ = write!(text, "k{number:02}a");
        if number == 0 {
            text.extend(core::iter::repeat_n('b', 94));
        }
        let key = alloc::vec![Value::Text(text.into_bytes()), Value::Int(number)];
        let record = write(&key, &[Affinity::None, Affinity::None], 4);
        let cell = write_cell(&Cell::IndexLeaf {
            payload: Payload {
                local: &record,
                total: record.len(),
                overflow: None,
            },
        });
        assert!(pages.writer(leaf).unwrap().insert(0, &cell).unwrap());
        keys.push(key);
    }
    let mut dividers: alloc::vec::Vec<alloc::vec::Vec<Value>> = alloc::vec::Vec::new();
    for number in 0..12_i64 {
        let mut text = alloc::string::String::new();
        let _ = write!(text, "k{number:02}z");
        text.extend(core::iter::repeat_n('d', 25));
        let key = alloc::vec![Value::Text(text.into_bytes()), Value::Int(number)];
        let record = write(&key, &[Affinity::None, Affinity::None], 4);
        let at = usize::try_from(number).unwrap();
        let cell = write_cell(&Cell::IndexInterior {
            child: leaves[at],
            payload: Payload {
                local: &record,
                total: record.len(),
                overflow: None,
            },
        });
        assert!(pages.writer(root).unwrap().insert(at, &cell).unwrap());
        dividers.push(key);
    }
    point(&mut pages, root, leaves[12]);
    // The root holds less room than the entry that moves up needs.
    assert!(pages.page(root).unwrap().free().unwrap() < 40);
    let gone = dividers[0].clone();
    assert!(remove_entry(&mut pages, root, &gone, &[]).unwrap());
    assert!(!remove_entry(&mut pages, root, &gone, &[]).unwrap());
    // The entry that moved up is still in the tree, as a divider.
    for kept in keys.iter().chain(dividers.iter().skip(1)) {
        assert!(
            matches!(find_entry(&pages, root, kept, &[]), Ok(Some(_))),
            "the tree lost an entry"
        );
    }
}

#[test]
fn every_entry_taken_out_of_an_index_leaves_the_ones_beside_it() {
    for (page_size, shape, stride) in SWEEPS {
        sweep(*page_size, *shape, *stride);
    }
}

/// How long the text of entry `number` is, which is what makes the
/// pages of a tree hold few entries or many.
fn length_of(shape: u8, number: u64) -> usize {
    let step = usize::try_from(number).unwrap_or(0);
    match shape {
        0 => step % 97,
        1 => {
            if step % 5 == 0 {
                400
            } else {
                2
            }
        }
        2 => step.saturating_mul(37) % 200,
        3 => {
            if step % 11 == 0 {
                300
            } else {
                step % 3
            }
        }
        _ => step.saturating_mul(53) % 7 + usize::from(step % 13 == 0) * 300,
    }
}

/// The page size, the shape of the entries, and how far apart the
/// entries are taken out, over the sweeps that between them reach every
/// balance a removal writes.
/// How many entries a sweep puts in.
const COUNT: i64 = 500;

const SWEEPS: &[(u32, u8, usize)] = &[
    (512, 0, 37),
    (512, 1, 37),
    (512, 2, 11),
    (1024, 1, 11),
    (512, 1, 3),
    (512, 3, 7),
    (512, 3, 29),
    (512, 4, 13),
    (512, 1, 1),
    (512, 1, 2),
    (512, 4, 1),
];

/// Puts `COUNT` entries in an index tree and takes every one of them out
/// again, with the tree read back after each.
fn sweep(page_size: u32, shape: u8, stride: usize) {
    use crate::tree::{insert_entry, remove_entry};
    use core::fmt::Write as _;
    let mut pages = Pages::new(page_size, 0).unwrap();
    assert_eq!(pages.add(Kind::LeafIndex, 0).unwrap(), 2);
    let mut keys: alloc::vec::Vec<alloc::vec::Vec<Value>> = alloc::vec::Vec::new();
    for number in 1..=COUNT {
        let mut text = alloc::string::String::new();
        let _ = write!(text, "row {number}");
        let length = length_of(shape, number.unsigned_abs());
        text.extend(core::iter::repeat_n('a', length));
        let key = alloc::vec![Value::Text(text.into_bytes()), Value::Int(number)];
        let record = write(&key, &[Affinity::None, Affinity::None], 4);
        insert_entry(&mut pages, 2, &record, &key, &[], false).unwrap();
        keys.push(key);
    }
    // The entries come out in an order that is neither the one they
    // went in nor the one they lie in, so the balance joins pages at
    // every place of the tree.
    let mut left = keys;
    let mut step = 0_usize;
    while !left.is_empty() {
        step = step.saturating_add(stride) % left.len();
        let key = left.remove(step);
        assert!(remove_entry(&mut pages, 2, &key, &[]).unwrap());
        // An entry the tree no longer holds is one the descent reaches
        // a leaf without finding.
        assert!(!remove_entry(&mut pages, 2, &key, &[]).unwrap());
        // Every entry left is read back every sixteenth removal, which
        // holds the cost of the sweep to O(n²/16) descents.
        if left.len().is_multiple_of(16) {
            for kept in &left {
                assert!(
                    matches!(crate::tree::find_entry(&pages, 2, kept, &[]), Ok(Some(_))),
                    "the tree lost an entry"
                );
            }
        }
    }
    assert_eq!(pages.page(2).unwrap().cells(), 0);
}

#[test]
fn a_delete_over_an_index_three_levels_deep_is_the_file_the_shell_wrote() {
    use crate::change::Writer;
    use core::fmt::Write as _;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(n INTEGER, s TEXT)").unwrap();
    let mut sql = alloc::string::String::from("INSERT INTO t(rowid,n,s) VALUES ");
    for number in 1..=900_i64 {
        if number > 1 {
            sql.push(',');
        }
        let pad: alloc::string::String =
            core::iter::repeat_n('a', usize::try_from(number % 53).unwrap_or(0)).collect();
        let _ = write!(
            sql,
            "({},{number},'row {number}{pad}')",
            (number * 541) % 1501
        );
    }
    writer.run(sql.as_bytes()).unwrap();
    writer.run(b"CREATE INDEX ts ON t(s)").unwrap();
    writer.run(b"DELETE FROM t WHERE n%3=0").unwrap();
    same(
        "index-tall.db",
        &writer.written(),
        crate::tests::TALL_INDEX,
        512,
    );
}

#[test]
fn the_entries_an_update_writes_again_are_the_ones_the_shell_wrote() {
    use crate::change::Writer;
    // The entry of a row comes out under the term the row held and goes
    // in under the term the statement gives it.
    let mut writer = indexed(60);
    writer
        .run(b"UPDATE t SET s='moved ' || n WHERE n%7=0")
        .unwrap();
    same(
        "index-moved.db",
        &writer.written(),
        crate::tests::SHIFTED,
        512,
    );
    // A statement that writes the key writes the entry again as well,
    // because the key is the last column of every entry.
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"CREATE INDEX ta ON t(a)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES(1,'a'),(2,'b'),(3,'c')")
        .unwrap();
    writer.run(b"UPDATE t SET rowid=9 WHERE a=2").unwrap();
    same(
        "index-rekeyed.db",
        &writer.written(),
        crate::tests::REKEYED,
        512,
    );
}

#[test]
fn an_index_over_the_column_the_key_names_holds_the_key_twice() {
    use crate::change::Writer;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE t(a INTEGER PRIMARY KEY, b)")
        .unwrap();
    writer
        .run(b"INSERT INTO t VALUES(7,'x'),(3,'y'),(9,'z')")
        .unwrap();
    writer.run(b"CREATE INDEX ta ON t(a)").unwrap();
    writer.run(b"DELETE FROM t WHERE b='y'").unwrap();
    same(
        "index-alias.db",
        &writer.written(),
        crate::tests::ALIAS,
        512,
    );
}

#[test]
fn what_a_statement_over_an_indexed_table_refuses() {
    use crate::change::Writer;
    // An index over another table leaves the statements of this one
    // alone.
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer.run(b"CREATE TABLE u(c,d)").unwrap();
    writer.run(b"CREATE INDEX uc ON u(c)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1,'a')").unwrap();
    assert!(writer.run(b"DELETE FROM t WHERE a=1").is_ok());
    // An index this crate cannot walk is one it cannot write, so the
    // statement that would make it is refused and no table carries one.
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    assert!(writer.run(b"CREATE INDEX ta ON t(a) WHERE a>0").is_err());
    assert!(writer.run(b"CREATE INDEX ta ON t(abs(a))").is_err());
    assert!(writer.run(b"CREATE INDEX ta ON nosuch(a)").is_err());
}

#[test]
fn a_table_is_named_with_its_quotes_off() {
    use crate::change::Writer;
    use crate::db::Database;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE \"t a\" (\"\"\"cb\"\"\")")
        .unwrap();
    writer
        .run(b"INSERT INTO \"t a\" (\"\"\"cb\"\"\") VALUES (1)")
        .unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    // A quote doubled inside a name is one quote of the name, so the
    // column here is called `"cb"` with its quotes in it.
    for sql in [
        b"SELECT * FROM \"t a\"".as_slice(),
        b"SELECT \"t a\".* FROM \"t a\"",
        b"SELECT \"\"\"cb\"\"\" FROM \"t a\"",
        b"SELECT \"t a\".\"\"\"cb\"\"\" FROM \"t a\"",
    ] {
        assert_eq!(database.query(sql).unwrap().rows, [[Value::Int(1)]]);
    }
    assert_eq!(
        database.query(b"SELECT * FROM \"t a\"").unwrap().names,
        [b"\"cb\"".to_vec()]
    );
}

#[test]
fn a_row_a_right_join_answered_is_matched_for_the_join_after_it() {
    use crate::change::Writer;
    use crate::db::Database;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for sql in [
        b"CREATE TABLE t1(x)".as_slice(),
        b"CREATE TABLE t2(y)",
        b"CREATE TABLE t3(z)",
        b"CREATE TABLE t4(w)",
        b"INSERT INTO t1 VALUES(10)",
        b"INSERT INTO t3 VALUES(20),(30)",
        b"INSERT INTO t4 VALUES(50)",
    ] {
        writer.run(sql).unwrap();
    }
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    // `t2` holds nothing, so the two `t3` rows are answered by the
    // first `RIGHT JOIN`; each matches the one `t4` row, so the second
    // `RIGHT JOIN` adds no row of its own.
    let answer = database
        .query(
            b"SELECT * FROM t1 INNER JOIN t2 ON true \
              RIGHT JOIN t3 ON t2.y IS NOT NULL \
              RIGHT JOIN t4 ON true ORDER BY t3.z",
        )
        .unwrap();
    assert_eq!(
        answer.rows,
        [
            alloc::vec![Value::Null, Value::Null, Value::Int(20), Value::Int(50)],
            alloc::vec![Value::Null, Value::Null, Value::Int(30), Value::Int(50)],
        ]
    );
}

#[test]
fn a_with_term_that_reads_itself_is_answered_row_by_row() {
    use crate::change::Writer;
    use crate::db::Database;
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.run(b"CREATE TABLE t(a)").unwrap();
    writer.run(b"INSERT INTO t VALUES(1),(2)").unwrap();
    // A `WITH` clause in front of an `INSERT` names the term the rows
    // come from.
    writer
        .run(
            b"WITH RECURSIVE c(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM c WHERE x<4) \
              INSERT INTO t SELECT x FROM c",
        )
        .unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    let text = |sql: &[u8]| match database.query(sql).unwrap().rows.first() {
        Some(row) => row.first().cloned(),
        None => None,
    };
    assert_eq!(
        text(b"SELECT group_concat(a) FROM t"),
        Some(Value::Text(b"1,2,1,2,3,4".to_vec()))
    );
    // The rows stand in the order the walk took them off the front, so
    // the cores after the recursive one answer behind the ones before
    // it.
    for (sql, want) in [
        (
            b"WITH RECURSIVE c(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM c WHERE x<4) \
              SELECT group_concat(x) FROM c"
                .as_slice(),
            b"1,2,3,4".as_slice(),
        ),
        // `UNION` answers a row once, so the two rows the walk starts
        // from are one row.
        (
            b"WITH RECURSIVE c(x) AS (VALUES(1),(1) UNION SELECT x+1 FROM c WHERE x<4) \
              SELECT group_concat(x) FROM c",
            b"1,2,3,4",
        ),
        // Two cores stand in front of the recursive one.
        (
            b"WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT 2 UNION ALL \
              SELECT x+10 FROM c WHERE x<5) SELECT group_concat(x) FROM c",
            b"1,2,11,12",
        ),
        // `RECURSIVE` says nothing: a term reads itself where it names
        // itself, and is answered once where it does not.
        (
            b"WITH c(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM c WHERE x<4) \
              SELECT group_concat(x) FROM c",
            b"1,2,3,4",
        ),
        (
            b"WITH RECURSIVE c(x) AS (SELECT a FROM t WHERE a<2) SELECT group_concat(x) FROM c",
            b"1,1",
        ),
    ] {
        assert_eq!(text(sql), Some(Value::Text(want.to_vec())), "{sql:?}");
    }
}

#[test]
fn what_a_with_term_that_reads_itself_refuses() {
    use crate::change::Writer;
    use crate::db::Database;
    let writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    for sql in [
        // The first core reads the term, so the walk starts from
        // nothing.
        b"WITH RECURSIVE c(x) AS (SELECT x FROM c) SELECT * FROM c".as_slice(),
        // An operator that is neither `UNION` nor `UNION ALL`.
        b"WITH RECURSIVE c(x) AS (VALUES(1) EXCEPT SELECT x FROM c) SELECT * FROM c",
        b"WITH RECURSIVE c(x) AS (VALUES(1) UNION ALL SELECT x FROM c INTERSECT VALUES(1)) \
          SELECT * FROM c",
        // The recursive core answers a different number of columns
        // from the cores in front of it.
        b"WITH RECURSIVE c(x,y) AS (SELECT 1,2 UNION ALL SELECT x FROM c) SELECT * FROM c",
        b"WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT 1,2 UNION ALL SELECT x FROM c) \
          SELECT * FROM c",
    ] {
        assert!(database.query(sql).is_err(), "{sql:?}");
    }
    // A term that does not stop is stopped by the count this crate
    // answers rows into memory up to.
    assert_eq!(
        database.query(
            b"WITH RECURSIVE c(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM c) SELECT * FROM c LIMIT 3"
        ),
        Err(crate::db::Error::Recursion)
    );
}

#[test]
fn random_and_randomblob_answer_the_bytes_the_seed_draws() {
    use crate::change::Writer;
    use crate::db::Database;
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer.randomness(7);
    writer.run(b"CREATE TABLE t(a,b)").unwrap();
    writer
        .run(b"INSERT INTO t VALUES(1, randomblob(400))")
        .unwrap();
    writer
        .run(b"INSERT INTO t VALUES(2, randomblob(400))")
        .unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    let answer = database
        .query(b"SELECT length(b), typeof(b) FROM t ORDER BY a")
        .unwrap();
    assert_eq!(
        answer.rows,
        [
            alloc::vec![Value::Int(400), Value::Text(b"blob".to_vec())],
            alloc::vec![Value::Int(400), Value::Text(b"blob".to_vec())],
        ]
    );
    // Two statements of one connection draw from where the connection
    // stands, so neither answers what the other did.
    let bytes = database.query(b"SELECT b FROM t ORDER BY a").unwrap().rows;
    assert_ne!(bytes.first(), bytes.get(1));
    // A blob longer than a value holds is refused, which is
    // `SQLITE_MAX_LENGTH`.
    assert_eq!(
        database.query(b"SELECT randomblob(1000000001)").err(),
        Some(crate::db::Error::Eval(crate::eval::Error::TooBig))
    );
    // `randomblob` answers one byte where the count is less than one,
    // and a database that is told no seed answers the bytes nought
    // draws.
    assert_eq!(
        database
            .query(b"SELECT length(randomblob(0)), length(randomblob(-5)), typeof(random())")
            .unwrap()
            .rows,
        [alloc::vec![
            Value::Int(1),
            Value::Int(1),
            Value::Text(b"integer".to_vec())
        ]]
    );
    // A seed says what the bytes are, so a database told the same seed
    // answers the same bytes.
    let one = Database::open(&written).unwrap().seeded(11);
    let other = Database::open(&written).unwrap().seeded(11);
    let draw = |database: &Database<'_>| database.query(b"SELECT randomblob(8)").unwrap().rows;
    assert_eq!(draw(&one), draw(&other));
    // A generated column is answered against the row alone, which has
    // no source under it, so the column is refused where it is read.
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    writer
        .run(b"CREATE TABLE u(a, b AS (randomblob(4)))")
        .unwrap();
    writer.run(b"INSERT INTO u(a) VALUES(1)").unwrap();
    let written = writer.written();
    let database = Database::open(&written).unwrap();
    assert!(database.query(b"SELECT b FROM u").is_err());
}

#[test]
fn the_schema_tree_grows_past_one_page_as_the_shell_grows_it() {
    use crate::change::Writer;
    let mut writer = Writer::new(512, 0, Encoding::Utf8).unwrap();
    for at in 1..=8u32 {
        let sql = alloc::format!(
            "CREATE TABLE t{at}(a TEXT NOT NULL, b INTEGER NOT NULL, c TEXT NOT NULL, \
             d TEXT NOT NULL, e TEXT NOT NULL, f INTEGER NOT NULL DEFAULT 0, \
             g TEXT NOT NULL, h TEXT NOT NULL)"
        );
        writer.run(sql.as_bytes()).unwrap();
    }
    same(
        "schema-deep.db",
        &writer.written(),
        crate::tests::SCHEMA_DEEP,
        512,
    );
}

#[test]
fn the_pragmas_a_connection_keeps_answer_what_it_was_told() {
    use crate::change::Writer;
    let mut writer = Writer::new(4096, 0, Encoding::Utf8).unwrap();
    let text = |bytes: &[u8]| Value::Text(bytes.to_vec());
    // What a connection told nothing answers, which is the shell's own
    // answer for a database with nothing set.
    for (sql, want) in [
        (b"PRAGMA mmap_size".as_slice(), Value::Int(0)),
        (b"PRAGMA locking_mode", text(b"normal")),
        (b"PRAGMA secure_delete", Value::Int(0)),
        (b"PRAGMA max_page_count", Value::Int(4_294_967_294)),
        (b"PRAGMA journal_size_limit", Value::Int(-1)),
        (b"PRAGMA wal_autocheckpoint", Value::Int(1000)),
        (b"PRAGMA query_only", Value::Int(0)),
        (b"PRAGMA automatic_index", Value::Int(1)),
        (b"PRAGMA synchronous", Value::Int(2)),
        (b"PRAGMA cache_size", Value::Int(-2000)),
        (b"PRAGMA data_version", Value::Int(1)),
    ] {
        assert_eq!(writer.run(sql).unwrap(), [[want]], "{sql:?}");
    }
    // Setting one answers the value it was set to where that pragma
    // answers a row, and nothing where it does not.
    for (sql, want) in [
        // This crate maps no file, so `mmap_size` stands at nought
        // whatever a statement sets it to, and `data_version` is raised
        // by a write from another connection and by nothing else.
        (b"PRAGMA mmap_size=8000000".as_slice(), Value::Int(0)),
        (b"PRAGMA data_version=1234", Value::Int(1)),
        (b"PRAGMA locking_mode=EXCLUSIVE", text(b"exclusive")),
        (b"PRAGMA secure_delete=fast", Value::Int(2)),
        (b"PRAGMA secure_delete=on", Value::Int(1)),
        (b"PRAGMA max_page_count=50", Value::Int(50)),
        (b"PRAGMA busy_timeout=500", Value::Int(500)),
    ] {
        assert_eq!(writer.run(sql).unwrap(), [[want]], "{sql:?}");
    }
    for sql in [
        b"PRAGMA query_only=1".as_slice(),
        b"PRAGMA cache_size=-4000",
        b"PRAGMA cache_size=2000",
        b"PRAGMA synchronous=normal",
        b"PRAGMA synchronous=full",
        b"PRAGMA synchronous=extra",
        b"PRAGMA synchronous=OFF",
        b"PRAGMA temp_store=default",
        b"PRAGMA temp_store=file",
        b"PRAGMA temp_store=memory",
        b"PRAGMA temp_store=1",
        b"PRAGMA synchronous=3",
        b"PRAGMA foreign_keys=ON",
    ] {
        assert!(writer.run(sql).unwrap().is_empty(), "{sql:?}");
    }
    assert_eq!(writer.run(b"PRAGMA temp_store").unwrap(), [[Value::Int(1)]]);
    // What was set is what the connection answers after.
    assert_eq!(
        writer.run(b"PRAGMA cache_size").unwrap(),
        [[Value::Int(2000)]]
    );
    assert_eq!(
        writer.run(b"PRAGMA synchronous").unwrap(),
        [[Value::Int(3)]]
    );
    // A value the pragma does not name is refused.
    assert!(writer.run(b"PRAGMA locking_mode=sometimes").is_err());
    assert!(writer.run(b"PRAGMA mmap_size=lots").is_err());
    assert!(writer.run(b"PRAGMA mmap_size=''").is_err());
    assert!(writer.run(b"PRAGMA query_only=maybe").is_err());
    assert!(writer.run(b"PRAGMA synchronous=sometimes").is_err());
    assert!(writer.run(b"PRAGMA temp_store=disk").is_err());
    assert!(writer.run(b"PRAGMA secure_delete=sometimes").is_err());
    // A pragma the file does not hold and the connection answers
    // nothing for is accepted and changes nothing.
    assert!(writer.run(b"PRAGMA cache_spill=0").unwrap().is_empty());
    // A database read answers what a connection told nothing answers.
    let written = writer.written();
    let database = crate::db::Database::open(&written).unwrap();
    assert_eq!(
        database.query(b"PRAGMA mmap_size").unwrap().rows,
        [[Value::Int(0)]]
    );
    assert_eq!(
        database.query(b"PRAGMA locking_mode").unwrap().rows,
        [[text(b"normal")]]
    );
}
