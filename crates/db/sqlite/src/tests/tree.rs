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
fn a_schema_larger_than_one_page_is_refused() {
    use crate::error::Error;
    // The schema table begins on page one, which carries the database
    // header before its own, so growing it is the balance this crate
    // does not write.
    let mut pages = Pages::new(512, 0).unwrap();
    let mut refused = 0;
    for number in 1..=20_i64 {
        let name = alloc::format!("t{number}");
        let sql = alloc::format!("CREATE TABLE {name}(a, b, c, d, e, f, g, h)");
        let row = schema_row(&name, number + 1, &sql, Encoding::Utf8);
        if insert(&mut pages, 1, number, &row) == Err(Error::Balance) {
            refused += 1;
        }
    }
    assert!(refused > 0, "page one took every row it was given");
}

#[test]
fn what_the_two_halves_of_the_balance_refuse() {
    use crate::error::Error;
    use crate::page::{Cell, Payload, write_cell};
    use crate::tree::{deepen, quick};
    // A tree of index pages has no key a divider names.
    let mut pages = Pages::new(512, 0).unwrap();
    let index = pages.add(Kind::LeafIndex, 0).unwrap();
    assert_eq!(deepen(&mut pages, index), Err(Error::Balance));
    // The schema's own tree begins on the page the database header is
    // on, so it cannot become the page above a child.
    assert_eq!(deepen(&mut pages, 1), Err(Error::Balance));
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
