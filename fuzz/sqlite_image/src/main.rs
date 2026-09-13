// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The SQLite file format reader against arbitrary bytes: no input may
//! panic, no walk may run forever, and nothing a reader hands back may
//! reach past the page it came from.
//!
//! A database is a file of pointers — page numbers, cell offsets, payload
//! lengths, overflow links — and every one of them is a number a file can
//! lie about. What this target holds is that a lie is answered with a
//! refusal: a cell that reaches past its page, a chain that turns back on
//! itself, a tree that points into itself.

use db_sqlite::{Cell, Image, Kind, Record, Value};

/// The most rows a walk reads. A file can describe more; reading them adds
/// no coverage and costs the fuzzer its time.
const MAX_ROWS: usize = 4096;

/// The largest payload this target assembles. A record can claim a length
/// no machine holds, and refusing to allocate it is the point.
const MAX_PAYLOAD: usize = 1 << 20;

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    let Ok(image) = Image::open(bytes) else {
        return;
    };
    let header = *image.header();
    assert!(header.page_size.is_power_of_two(), "a page size that is not");
    assert!(header.usable() <= header.page_size, "usable past the page");

    // Every page of the file, read as a page: the pointer array and the
    // cells of whatever type the first byte claims.
    for number in 1..=image.pages().min(64) {
        let Ok(page) = image.page(number) else {
            continue;
        };
        for index in 0..page.cells() {
            let Ok(cell) = page.cell(index) else {
                continue;
            };
            check(&image, page.kind(), &cell);
        }
        assert_eq!(
            page.right_most().is_some(),
            page.kind().is_interior(),
            "a right-most pointer on a page without children"
        );
    }

    // The schema, and then every tree it names.
    let mut roots = Vec::new();
    walk(&image, 1, &mut roots);
    for root in roots.iter().take(32) {
        walk(&image, *root, &mut Vec::new());
    }
});

/// Walks one table tree, reading every record it holds and collecting the
/// root pages the records name.
fn walk(image: &Image<'_>, root: u32, roots: &mut Vec<u32>) {
    for row in image.rows(root).take(MAX_ROWS) {
        let Ok(row) = row else {
            return;
        };
        assert!(
            row.payload.local.len() <= row.payload.total,
            "more of a payload on the page than the payload has"
        );
        let mut whole = Vec::new();
        let payload = if row.payload.is_whole() {
            row.payload.local
        } else if row.payload.total <= MAX_PAYLOAD {
            whole.resize(row.payload.total, 0);
            if image.read_payload(&row.payload, &mut whole).is_err() {
                continue;
            }
            &whole
        } else {
            continue;
        };
        let Ok(record) = Record::parse(payload) else {
            continue;
        };
        for value in record.values() {
            match value {
                Ok(Value::Text(text) | Value::Blob(text)) => {
                    assert!(text.len() <= payload.len(), "a value longer than its record");
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        if let Ok(Some(Value::Int(page))) = record.value(3) {
            if let Ok(page) = u32::try_from(page) {
                roots.push(page);
            }
        }
    }
}

/// What every cell of a page of this type has to hold.
fn check(image: &Image<'_>, kind: Kind, cell: &Cell<'_>) {
    match (kind, cell) {
        (Kind::LeafTable, Cell::TableLeaf { payload, .. })
        | (Kind::LeafIndex, Cell::IndexLeaf { payload })
        | (Kind::InteriorIndex, Cell::IndexInterior { payload, .. }) => {
            assert!(
                payload.local.len() <= payload.total,
                "more of a payload on the page than the payload has"
            );
            if let Some(first) = payload.overflow {
                assert!(first > 0, "an overflow chain that begins at page zero");
                let _ = image.page_bytes(first);
            }
        }
        (Kind::InteriorTable, Cell::TableInterior { child, .. }) => {
            assert!(*child > 0, "a child page of zero");
        }
        _ => panic!("a cell of the wrong shape for its page"),
    }
}
