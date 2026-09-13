// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::page`.

#![allow(clippy::arithmetic_side_effects)]

use crate::error::Error;
use crate::header::Header;
use crate::image::Image;
use crate::page::{Cell, Kind, Page};
use crate::tests::{INDEXED, PAGE512, SMALL};

#[test]
fn every_page_type_is_the_byte_the_format_gives_it() {
    assert_eq!(Kind::from_byte(2), Ok(Kind::InteriorIndex));
    assert_eq!(Kind::from_byte(5), Ok(Kind::InteriorTable));
    assert_eq!(Kind::from_byte(10), Ok(Kind::LeafIndex));
    assert_eq!(Kind::from_byte(13), Ok(Kind::LeafTable));
    for byte in [0u8, 1, 3, 4, 6, 9, 11, 12, 14, 255] {
        assert_eq!(Kind::from_byte(byte), Err(Error::PageKind(byte)));
    }
    // The kind a page of the same tree has when it has children, which
    // is the byte without its leaf bit.
    assert_eq!(Kind::LeafTable.interior(), Kind::InteriorTable);
    assert_eq!(Kind::InteriorTable.interior(), Kind::InteriorTable);
    assert_eq!(Kind::LeafIndex.interior(), Kind::InteriorIndex);
    assert_eq!(Kind::InteriorIndex.interior(), Kind::InteriorIndex);
}

#[test]
fn an_interior_page_has_a_longer_header_and_a_right_most_pointer() {
    assert_eq!(Kind::InteriorTable.header_len(), 12);
    assert_eq!(Kind::LeafTable.header_len(), 8);
    assert!(Kind::InteriorIndex.is_interior());
    assert!(!Kind::LeafIndex.is_interior());
    assert!(Kind::InteriorTable.is_table());
    assert!(!Kind::InteriorIndex.is_table());
}

#[test]
fn page_one_carries_the_database_header_before_its_own() {
    let image = Image::open(SMALL).unwrap();
    let page = image.page(1).unwrap();
    assert_eq!(page.kind(), Kind::LeafTable);
    // One row of the schema table: the one table of the fixture.
    assert_eq!(page.cells(), 1);
    assert_eq!(page.right_most(), None);
    assert!(page.content_start() < 4096);
}

#[test]
fn the_root_of_a_tall_tree_is_an_interior_page_that_points_at_its_children() {
    let image = Image::open(PAGE512).unwrap();
    let root = image.page(2).unwrap();
    assert_eq!(root.kind(), Kind::InteriorTable);
    assert!(root.cells() > 0);
    let right = root.right_most().unwrap();
    assert!(right > 1);
    let Ok(Cell::TableInterior { child, rowid }) = root.cell(0) else {
        panic!("an interior table page holds interior table cells");
    };
    assert!(child > 1);
    assert!(rowid > 0);
    assert_eq!(root.cell(root.cells()), Err(Error::Overrun));
}

#[test]
fn an_index_page_holds_records_and_no_rowid_of_its_own() {
    let image = Image::open(INDEXED).unwrap();
    // The schema names eleven indexes; every one of their roots is an
    // index page, whatever the index is over.
    let mut roots = Vec::new();
    for row in image.schema() {
        let row = row.unwrap();
        let record = row.record().unwrap();
        if record.value(0).unwrap() == Some(crate::record::Value::Text(b"index"))
            && let Some(crate::record::Value::Int(root)) = record.value(3).unwrap()
        {
            roots.push(u32::try_from(root).unwrap());
        }
    }
    assert_eq!(roots.len(), 11);
    for root in roots {
        let page = image.page(root).unwrap();
        assert_eq!(page.kind(), Kind::LeafIndex);
        assert!(page.cells() > 0);
        let Ok(Cell::IndexLeaf { payload }) = page.cell(0) else {
            panic!("an index leaf holds index cells");
        };
        assert!(payload.is_whole());
        assert!(payload.total > 0);
    }
}

#[test]
fn a_page_shorter_than_its_own_header_is_refused() {
    let header = Header::parse(SMALL).unwrap();
    assert_eq!(Page::parse(&[], 2, header.usable()), Err(Error::Overrun));
    assert_eq!(
        Page::parse(&SMALL[..10], 1, header.usable()),
        Err(Error::Overrun)
    );
}

#[test]
fn a_page_whose_first_byte_is_not_a_page_type_is_refused() {
    let mut page = [0u8; 4096];
    page.copy_from_slice(&SMALL[4096..8192]);
    page[0] = 7;
    assert_eq!(Page::parse(&page, 2, 4096), Err(Error::PageKind(7)));
}

#[test]
fn a_cell_pointer_array_that_does_not_fit_the_page_is_refused() {
    let mut page = [0u8; 4096];
    page.copy_from_slice(&SMALL[4096..8192]);
    // Two thousand and forty-eight cells need 4096 bytes of pointers,
    // which is the whole page and more than what is left of it.
    page[3] = 0x08;
    page[4] = 0x00;
    assert_eq!(Page::parse(&page, 2, 4096), Err(Error::Overrun));
}

#[test]
fn a_cell_pointer_that_points_outside_the_page_is_refused() {
    let mut page = [0u8; 4096];
    page.copy_from_slice(&SMALL[4096..8192]);
    // The first pointer of a leaf page lies at offset 8.
    page[8] = 0xff;
    page[9] = 0xff;
    let parsed = Page::parse(&page, 2, 4096).unwrap();
    assert_eq!(parsed.cell_offset(0), Err(Error::Overrun));
    assert_eq!(parsed.cell(0), Err(Error::Overrun));
}

#[test]
fn the_content_area_of_a_page_that_says_zero_is_the_largest_page_there_is() {
    let mut page = [0u8; 4096];
    page.copy_from_slice(&SMALL[4096..8192]);
    page[5] = 0;
    page[6] = 0;
    let parsed = Page::parse(&page, 2, 4096).unwrap();
    assert_eq!(parsed.content_start(), 65536);
}

/// A page of `kind` with one cell, whose bytes are `cell`, laid out the
/// way the format does: header, pointer array, then the cell at the end.
fn one_cell(kind: u8, right_most: u32, cell: &[u8]) -> [u8; 512] {
    let mut page = [0u8; 512];
    let header = if kind == 2 || kind == 5 { 12 } else { 8 };
    page[0] = kind;
    page[3] = 0;
    page[4] = 1;
    let offset = 512 - cell.len();
    page[5] = u8::try_from(offset >> 8).unwrap();
    page[6] = u8::try_from(offset & 0xff).unwrap();
    page[8..12].copy_from_slice(&right_most.to_be_bytes());
    page[header] = u8::try_from(offset >> 8).unwrap();
    page[header + 1] = u8::try_from(offset & 0xff).unwrap();
    page[offset..].copy_from_slice(cell);
    page
}

#[test]
fn a_child_pointer_of_zero_names_a_page_no_file_has() {
    // An interior table cell: four bytes of child, then a rowid.
    let page = one_cell(5, 3, &[0, 0, 0, 0, 42]);
    let parsed = Page::parse(&page, 2, 512).unwrap();
    assert_eq!(parsed.kind(), Kind::InteriorTable);
    assert_eq!(parsed.cell(0), Err(Error::Page(0)));

    // An interior index cell: four bytes of child, a payload length, then
    // the payload.
    let page = one_cell(2, 3, &[0, 0, 0, 0, 2, 1, 9]);
    let parsed = Page::parse(&page, 2, 512).unwrap();
    assert_eq!(parsed.cell(0), Err(Error::Page(0)));

    // The same cells with a child that exists read as cells.
    let page = one_cell(5, 3, &[0, 0, 0, 4, 42]);
    let parsed = Page::parse(&page, 2, 512).unwrap();
    assert_eq!(
        parsed.cell(0),
        Ok(Cell::TableInterior {
            child: 4,
            rowid: 42
        })
    );
}

#[test]
fn the_threshold_rule_decides_how_much_of_a_payload_stays_on_its_page() {
    use crate::page::local_len;
    // Section 1.6: a table leaf keeps a payload of X = U - 35 whole.
    assert_eq!(local_len(100, 4096, Kind::LeafTable), 100);
    assert_eq!(local_len(4061, 4096, Kind::LeafTable), 4061);
    // Above that, K = M + (P - M) % (U - 4) is what stays if it is no
    // larger than X, and M otherwise. M is 489 for a 4096-byte page, and
    // the remainder is what makes the last overflow page a full one.
    assert_eq!(local_len(5000, 4096, Kind::LeafTable), 908);
    assert_eq!(local_len(100_000, 4096, Kind::LeafTable), 1792);
    // The two payloads either side of X, where K comes out above X and
    // the rule falls back to M.
    assert_eq!(local_len(4062, 4096, Kind::LeafTable), 489);
    assert_eq!(local_len(4089, 4096, Kind::LeafTable), 489);
    // An index page keeps far less whole: X = ((U - 12) * 64 / 255) - 23.
    assert_eq!(local_len(1002, 4096, Kind::LeafIndex), 1002);
    assert_eq!(local_len(1003, 4096, Kind::InteriorIndex), 489);
    assert_eq!(local_len(6000, 4096, Kind::LeafIndex), 489);
    // One that lands under the index threshold, so K is what stays.
    assert_eq!(local_len(4681, 4096, Kind::LeafIndex), 589);
    // A payload never keeps more than it has.
    for total in [0, 1, 35, 36, 477, 478, 3000] {
        assert!(local_len(total, 512, Kind::LeafTable) <= total);
    }
}

#[test]
fn a_cell_of_a_page_type_is_the_shape_that_page_type_holds() {
    // An index leaf cell: a payload length, then the payload.
    let page = one_cell(10, 0, &[2, 1, 9]);
    let parsed = Page::parse(&page, 2, 512).unwrap();
    assert_eq!(parsed.kind(), Kind::LeafIndex);
    let Ok(Cell::IndexLeaf { payload }) = parsed.cell(0) else {
        panic!("an index leaf holds index cells");
    };
    assert_eq!(payload.total, 2);
    assert_eq!(payload.local, &[1, 9]);
    assert!(payload.is_whole());

    // An interior index cell: the child, the length, then the payload.
    let page = one_cell(2, 7, &[0, 0, 0, 5, 2, 1, 9]);
    let parsed = Page::parse(&page, 2, 512).unwrap();
    let Ok(Cell::IndexInterior { child, payload }) = parsed.cell(0) else {
        panic!("an interior index holds interior index cells");
    };
    assert_eq!(child, 5);
    assert_eq!(payload.total, 2);
    assert_eq!(parsed.right_most(), Some(7));
}

#[test]
fn a_cell_that_ends_where_the_page_does_is_refused() {
    // A table leaf cell whose payload length says more than the page has
    // left after it.
    let page = one_cell(13, 0, &[100, 1, 1, 2, 3]);
    let parsed = Page::parse(&page, 2, 512).unwrap();
    assert_eq!(parsed.cell(0), Err(Error::Overrun));
    // A cell whose varint runs off the end of the page.
    let page = one_cell(13, 0, &[0x81]);
    let parsed = Page::parse(&page, 2, 512).unwrap();
    assert_eq!(parsed.cell(0), Err(Error::Varint));
}

#[test]
fn a_cell_of_each_shape_that_reaches_past_its_page_is_refused() {
    // An interior table cell needs four bytes of child and a rowid.
    let page = one_cell(5, 9, &[0, 0, 0]);
    assert_eq!(
        Page::parse(&page, 2, 512).unwrap().cell(0),
        Err(Error::Overrun)
    );
    // An interior table cell with the child but no rowid after it.
    let page = one_cell(5, 9, &[0, 0, 0, 4]);
    assert_eq!(
        Page::parse(&page, 2, 512).unwrap().cell(0),
        Err(Error::Varint)
    );
    // An index leaf whose payload length says more than is there.
    let page = one_cell(10, 0, &[100, 1]);
    assert_eq!(
        Page::parse(&page, 2, 512).unwrap().cell(0),
        Err(Error::Overrun)
    );
    // An interior index cell cut off after its child.
    let page = one_cell(2, 9, &[0, 0, 0, 4]);
    assert_eq!(
        Page::parse(&page, 2, 512).unwrap().cell(0),
        Err(Error::Varint)
    );
    let page = one_cell(2, 9, &[0, 0, 0]);
    assert_eq!(
        Page::parse(&page, 2, 512).unwrap().cell(0),
        Err(Error::Overrun)
    );
}

#[test]
fn a_payload_whose_overflow_pointer_is_cut_off_is_refused() {
    // A table leaf cell claiming six hundred bytes: ninety-two stay on the
    // page and a four-byte pointer follows them, which this cell lacks.
    let mut cell = vec![0x84, 0x58, 1];
    cell.extend(core::iter::repeat_n(b'a', 92));
    cell.extend_from_slice(&[0, 0]);
    let page = one_cell(13, 0, &cell);
    assert_eq!(
        Page::parse(&page, 2, 512).unwrap().cell(0),
        Err(Error::Overrun)
    );
}

#[test]
fn a_row_is_asked_of_a_leaf_and_a_child_of_an_interior_page() {
    let leaf = one_cell(13, 0, &[2, 1, 9, 9]);
    let leaf = Page::parse(&leaf, 2, 512).unwrap();
    assert_eq!(leaf.row(0).map(|(rowid, _)| rowid), Ok(1));
    assert_eq!(leaf.child(0), Err(Error::PageKind(13)));

    let interior = one_cell(5, 9, &[0, 0, 0, 4, 42]);
    let interior = Page::parse(&interior, 2, 512).unwrap();
    assert_eq!(interior.child(0), Ok(4));
    assert_eq!(interior.row(0), Err(Error::PageKind(5)));

    let index = one_cell(10, 0, &[2, 1, 9]);
    let index = Page::parse(&index, 2, 512).unwrap();
    assert_eq!(index.row(0), Err(Error::PageKind(10)));
    assert_eq!(index.child(0), Err(Error::PageKind(10)));

    let interior_index = one_cell(2, 9, &[0, 0, 0, 5, 2, 1, 9]);
    let interior_index = Page::parse(&interior_index, 2, 512).unwrap();
    assert_eq!(interior_index.child(0), Ok(5));
    assert_eq!(interior_index.row(0), Err(Error::PageKind(2)));
}

#[test]
fn every_page_type_answers_the_byte_it_is_written_as() {
    for kind in [
        Kind::InteriorIndex,
        Kind::InteriorTable,
        Kind::LeafIndex,
        Kind::LeafTable,
    ] {
        assert_eq!(Kind::from_byte(kind.byte()), Ok(kind));
    }
}

#[test]
fn the_cells_of_a_page_are_read_in_the_order_the_pointers_name_them() {
    let image = Image::open(SMALL).unwrap();
    let page = image.page(1).unwrap();
    let cells: Vec<_> = page.cells_iter().collect();
    assert_eq!(cells.len(), page.cells());
    assert!(cells.iter().all(Result::is_ok));
}

#[test]
fn a_cell_whose_key_or_length_is_cut_off_is_refused_as_a_varint() {
    // A table leaf whose payload length is there and whose rowid is not.
    let page = one_cell(13, 0, &[2]);
    assert_eq!(
        Page::parse(&page, 2, 512).unwrap().cell(0),
        Err(Error::Varint)
    );
    // An index leaf whose length does not end inside the page.
    let page = one_cell(10, 0, &[0x81]);
    assert_eq!(
        Page::parse(&page, 2, 512).unwrap().cell(0),
        Err(Error::Varint)
    );
    // Asking for a child where the cell itself cannot be read.
    let page = one_cell(5, 9, &[0, 0, 0]);
    assert_eq!(
        Page::parse(&page, 2, 512).unwrap().child(0),
        Err(Error::Overrun)
    );
    // And asking for a row where the same is true.
    let page = one_cell(13, 0, &[0x81]);
    assert_eq!(
        Page::parse(&page, 2, 512).unwrap().row(0),
        Err(Error::Varint)
    );
}

#[test]
fn an_interior_index_cell_whose_payload_is_short_is_refused() {
    // The child, a length of a hundred, and two bytes of payload.
    let page = one_cell(2, 9, &[0, 0, 0, 4, 100, 1, 2]);
    assert_eq!(
        Page::parse(&page, 2, 512).unwrap().cell(0),
        Err(Error::Overrun)
    );
}

#[test]
fn the_entry_of_a_table_page_is_refused_by_the_type_of_the_page() {
    let image = crate::Image::open(super::SMALL).unwrap();
    let page = image.page(2).unwrap();
    assert_eq!(page.kind(), crate::Kind::LeafTable);
    assert_eq!(page.entry(0), Err(crate::Error::PageKind(13)));
}

/// Every page of every tree a fixture names, root first.
fn tree_pages(image: &crate::image::Image<'_>, root: u32, out: &mut Vec<u32>) {
    let Ok(page) = image.page(root) else {
        return;
    };
    out.push(root);
    if !page.kind().is_interior() {
        return;
    }
    for at in 0..page.cells() {
        if let Ok(child) = page.child(at) {
            tree_pages(image, child, out);
        }
    }
    if let Some(right) = page.right_most() {
        tree_pages(image, right, out);
    }
}

/// The roots a fixture names: the schema's own tree and every tree it
/// holds a row for.
fn roots(image: &crate::image::Image<'_>) -> Vec<u32> {
    let mut out = alloc::vec![1];
    let mut payload = Vec::new();
    for row in image.schema() {
        let Ok(row) = row else { continue };
        payload.resize(row.payload.total, 0);
        if image.read_payload(&row.payload, &mut payload).is_err() {
            continue;
        }
        let Ok(record) = crate::record::Record::parse(&payload) else {
            continue;
        };
        if let Ok(Some(crate::record::Value::Int(root))) = record.value(3)
            && let Ok(root) = u32::try_from(root)
        {
            out.push(root);
        }
    }
    out
}

#[test]
fn every_cell_of_every_fixture_is_written_back_as_the_bytes_it_lies_in() {
    use crate::image::Image;
    let mut counted = 0;
    let mut rebuilt = 0;
    for (name, bytes) in crate::tests::WRITTEN {
        let image = Image::open(bytes).unwrap_or_else(|error| panic!("{name}: {error:?}"));
        let usable = usize::try_from(image.header().usable()).unwrap();
        let page_size = usize::try_from(image.header().page_size).unwrap();
        let mut pages = Vec::new();
        for root in roots(&image) {
            tree_pages(&image, root, &mut pages);
        }
        for number in pages {
            let whole = image.page_bytes(number).unwrap();
            let page = image.page(number).unwrap();
            let mut cells = Vec::new();
            for at in 0..page.cells() {
                let cell = page.cell(at).unwrap();
                let written = crate::page::write_cell(&cell);
                let offset = page.cell_offset(at).unwrap();
                assert_eq!(
                    whole.get(offset..offset + written.len()),
                    Some(written.as_slice()),
                    "{name} page {number} cell {at}"
                );
                cells.push(written);
                counted += 1;
            }
            // A page that was filled in one pass holds its cells from the
            // end downward in the order the pointer array names them, and
            // has no freeblock and no fragmented byte. Such a page is the
            // one `build` writes, byte for byte.
            let start = usize::from(number == 1) * crate::header::HEADER_LEN;
            let freeblock = u16::from_be_bytes([whole[start + 1], whole[start + 2]]);
            let fragmented = whole[start + 7];
            let descending = (0..page.cells()).all(|at| {
                at == 0 || page.cell_offset(at).unwrap() < page.cell_offset(at - 1).unwrap()
            });
            if freeblock != 0 || fragmented != 0 || !descending {
                continue;
            }
            let built = crate::page::build(
                page.kind(),
                number,
                page_size,
                usable,
                &cells,
                page.right_most(),
            )
            .unwrap();
            // The header, the pointer array and the content area are the
            // page; what lies between the array and the content is
            // unallocated, and SQLite leaves whatever was there before in
            // it, so only the three are compared. Page 1 carries the
            // database header, which `build` leaves as it found it, and
            // the reserved tail is not the b-tree's either.
            let array = start + page.kind().header_len() + page.cells() * 2;
            assert_eq!(
                built.get(start..array),
                whole.get(start..array),
                "{name} page {number}, the header and the pointers"
            );
            let content = page.content_start();
            assert_eq!(
                built.get(content..usable),
                whole.get(content..usable),
                "{name} page {number}, the content"
            );
            rebuilt += 1;
        }
    }
    assert!(counted > 1600, "only {counted} cells were written back");
    assert!(rebuilt > 50, "only {rebuilt} pages were built back");
}

#[test]
fn a_page_that_cannot_hold_what_it_is_given_is_not_built() {
    use crate::page::build;
    let none: [Vec<u8>; 0] = [];
    // A usable size past the page itself, which no header names.
    assert_eq!(build(Kind::LeafTable, 2, 512, 1024, &none, None), None);
    // A page shorter than the database header page 1 carries first.
    assert_eq!(build(Kind::LeafTable, 1, 64, 64, &none, None), None);
    // Cells whose bytes alone are more than the page holds.
    let over = alloc::vec![alloc::vec![0u8; 8]; 65];
    assert_eq!(build(Kind::LeafTable, 2, 512, 512, &over, None), None);
    // Cells that fit with no room left for the pointers naming them.
    let tight = alloc::vec![alloc::vec![0u8; 8]; 60];
    assert_eq!(build(Kind::LeafTable, 2, 512, 512, &tight, None), None);
}

#[test]
fn a_page_built_from_cells_is_read_back_as_those_cells() {
    use crate::page::{Payload, build, write_cell};
    // A payload of six hundred bytes leaves ninety-two of them on a page
    // of five hundred and twelve, which is the rule of section 1.6.
    let local = crate::page::local_len(600, 512, Kind::LeafTable);
    assert_eq!(local, 92);
    let body = alloc::vec![0x41u8; local];
    let cells = [
        write_cell(&Cell::TableLeaf {
            rowid: 1,
            payload: Payload {
                local: b"\x03\x11one",
                total: 5,
                overflow: None,
            },
        }),
        write_cell(&Cell::TableLeaf {
            rowid: 9_223_372_036_854_775_807,
            payload: Payload {
                local: &body,
                total: 600,
                overflow: Some(7),
            },
        }),
    ];
    let built = build(Kind::LeafTable, 2, 512, 512, &cells, None).unwrap();
    let page = Page::parse(&built, 2, 512).unwrap();
    assert_eq!(page.kind(), Kind::LeafTable);
    assert_eq!(page.cells(), 2);
    assert_eq!(page.right_most(), None);
    // The first cell lies highest, which is where the content area
    // begins, and the second one below it.
    assert_eq!(page.content_start(), 512 - cells[0].len() - cells[1].len());
    assert!(page.cell_offset(0).unwrap() > page.cell_offset(1).unwrap());
    let Ok(Cell::TableLeaf { rowid, payload }) = page.cell(1) else {
        panic!("a cell of another shape");
    };
    assert_eq!(rowid, 9_223_372_036_854_775_807);
    assert_eq!(payload.overflow, Some(7));
    assert_eq!(payload.total, 600);
}

#[test]
fn an_interior_page_built_from_cells_keeps_its_right_most_pointer() {
    use crate::page::{build, write_cell};
    let cells = [write_cell(&Cell::TableInterior {
        child: 4,
        rowid: -1,
    })];
    let built = build(Kind::InteriorTable, 2, 512, 512, &cells, Some(9)).unwrap();
    let page = Page::parse(&built, 2, 512).unwrap();
    assert_eq!(page.right_most(), Some(9));
    assert_eq!(
        page.cell(0),
        Ok(Cell::TableInterior {
            child: 4,
            rowid: -1
        })
    );
}

#[test]
fn a_cell_taken_off_a_page_and_put_back_leaves_the_page_as_it_was() {
    use crate::image::Image;
    use crate::page::{Writer, write_cell};
    let mut counted = 0;
    for (name, bytes) in crate::tests::WRITTEN {
        let image = Image::open(bytes).unwrap();
        let usable = image.header().usable();
        let mut pages = Vec::new();
        for root in roots(&image) {
            tree_pages(&image, root, &mut pages);
        }
        for number in pages {
            let whole = image.page_bytes(number).unwrap();
            let page = image.page(number).unwrap();
            let start = usize::from(number == 1) * crate::header::HEADER_LEN;
            // A page that already holds a freeblock gives the cell back
            // to a list that is not empty, and what comes back is a slot
            // of another size at another place. Such a page is the
            // balance's to rewrite, not this test's.
            if whole[start + 1] != 0 || whole[start + 2] != 0 || whole[start + 7] != 0 {
                continue;
            }
            for at in 0..page.cells() {
                let cell = write_cell(&page.cell(at).unwrap());
                let mut copy = whole.to_vec();
                let mut writer = Writer::open(&mut copy, number, usable).unwrap();
                let free = writer.free().unwrap();
                writer.remove(at).unwrap();
                assert_eq!(
                    writer.free().unwrap(),
                    free + cell.len() + 2,
                    "{name} page {number} cell {at}, what the cell left"
                );
                assert!(writer.insert(at, &cell).unwrap());
                assert_eq!(writer.free().unwrap(), free);
                assert_eq!(copy, whole, "{name} page {number} cell {at}");
                counted += 1;
            }
        }
    }
    assert!(
        counted > 1300,
        "only {counted} cells were taken off and put back"
    );
}

/// One cell of `len` bytes of payload, with `rowid` as its key.
fn leaf_cell(rowid: i64, len: usize) -> Vec<u8> {
    let body = alloc::vec![u8::try_from(rowid & 0x7f).unwrap(); len];
    crate::page::write_cell(&Cell::TableLeaf {
        rowid,
        payload: crate::page::Payload {
            local: &body,
            total: len,
            overflow: None,
        },
    })
}

#[test]
fn a_page_holds_what_is_put_on_it_and_gives_back_what_is_taken_off() {
    use crate::page::{Writer, build};
    // A run of inserts and removes over a page of five hundred and
    // twelve bytes, sized so that the page fills, fragments, and is
    // moved together again. The page is read back after every step.
    let mut bytes = build(Kind::LeafTable, 2, 512, 512, &[], None).unwrap();
    let mut held: Vec<Vec<u8>> = Vec::new();
    let mut seed: u32 = 0x9e37_79b9;
    let mut inserted = 0;
    let mut removed = 0;
    let mut refused = 0;
    for step in 0..400_u32 {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let pick = usize::try_from(seed >> 16).unwrap();
        let mut writer = Writer::open(&mut bytes, 2, 512).unwrap();
        if held.is_empty() || pick % 3 != 0 {
            let len = 4 + pick % 40;
            let cell = leaf_cell(i64::from(step), len);
            let at = pick % (held.len() + 1);
            if writer.insert(at, &cell).unwrap() {
                held.insert(at, cell);
                inserted += 1;
            } else {
                refused += 1;
            }
        } else {
            let at = pick % held.len();
            writer.remove(at).unwrap();
            held.remove(at);
            removed += 1;
        }
        let page = Page::parse(&bytes, 2, 512).unwrap();
        assert_eq!(page.cells(), held.len(), "step {step}");
        for (at, cell) in held.iter().enumerate() {
            let offset = page.cell_offset(at).unwrap();
            assert_eq!(
                bytes.get(offset..offset + cell.len()),
                Some(cell.as_slice()),
                "step {step}, cell {at}"
            );
        }
    }
    // The run reaches every shape: a page that fills, a page that empties
    // again, and cells that go back on it.
    assert!(inserted > 100, "only {inserted} cells went on");
    assert!(removed > 50, "only {removed} cells came off");
    assert!(refused > 0, "the page never filled");
}

#[test]
fn a_page_whose_free_space_is_in_pieces_is_moved_together_to_take_a_cell() {
    use crate::page::{Writer, build};
    // Eight cells of forty bytes, then every other one taken off: the
    // page holds a hundred and sixty free bytes in four pieces, none of
    // which is forty-eight, so a cell of that size moves the page
    // together rather than fitting in a piece of it.
    let mut bytes = build(Kind::LeafTable, 2, 256, 256, &[], None).unwrap();
    let mut writer = Writer::open(&mut bytes, 2, 256).unwrap();
    for at in 0..5 {
        assert!(
            writer
                .insert(at, &leaf_cell(i64::try_from(at).unwrap() + 1, 36))
                .unwrap()
        );
    }
    for at in [3, 1] {
        writer.remove(at).unwrap();
    }
    let scattered = writer.free().unwrap();
    let big = leaf_cell(9, 70);
    assert!(big.len() + 2 <= scattered);
    assert!(writer.insert(1, &big).unwrap());
    // What is left is one piece: the cells lie at the end of the page
    // and nothing is on the free list.
    let page = Page::parse(&bytes, 2, 256).unwrap();
    assert_eq!(page.cells(), 4);
    assert_eq!(u16::from_be_bytes([bytes[1], bytes[2]]), 0);
    assert_eq!(bytes[7], 0);
    let offset = page.cell_offset(1).unwrap();
    assert_eq!(bytes.get(offset..offset + big.len()), Some(big.as_slice()));
}

#[test]
fn a_page_moved_together_keeps_every_cell_where_three_pieces_are_free() {
    use crate::page::{Writer, build};
    // Three freeblocks is one more than the fast path of
    // `defragmentPage` takes, so every cell is copied to the end
    // instead. The cells have to read back in the order they went on.
    let mut bytes = build(Kind::LeafTable, 2, 256, 256, &[], None).unwrap();
    let mut writer = Writer::open(&mut bytes, 2, 256).unwrap();
    for at in 0..7 {
        assert!(
            writer
                .insert(at, &leaf_cell(i64::try_from(at).unwrap() + 1, 24))
                .unwrap()
        );
    }
    for at in [5, 3, 1] {
        writer.remove(at).unwrap();
    }
    let big = leaf_cell(9, 60);
    assert!(writer.insert(0, &big).unwrap());
    let page = Page::parse(&bytes, 2, 256).unwrap();
    assert_eq!(page.cells(), 5);
    let keys: Vec<i64> = (0..page.cells())
        .map(|at| match page.cell(at).unwrap() {
            Cell::TableLeaf { rowid, .. } => rowid,
            _ => panic!("a cell of another shape"),
        })
        .collect();
    assert_eq!(keys, [9, 1, 3, 5, 7]);
}

#[test]
fn a_cell_index_no_page_has_is_refused() {
    use crate::page::{Writer, build};
    let mut bytes = build(Kind::LeafTable, 2, 256, 256, &[], None).unwrap();
    let mut writer = Writer::open(&mut bytes, 2, 256).unwrap();
    assert_eq!(writer.remove(0), Err(Error::Overrun));
    assert_eq!(writer.insert(1, &leaf_cell(1, 8)), Err(Error::Overrun));
    assert!(writer.insert(0, &leaf_cell(1, 8)).unwrap());
    assert_eq!(writer.remove(1), Err(Error::Overrun));
    // A cell of more than the page holds is answered, not written.
    assert_eq!(writer.insert(0, &leaf_cell(2, 250)), Ok(false));
}

#[test]
fn a_page_whose_free_space_does_not_count_up_is_refused_rather_than_written() {
    use crate::page::{Writer, build};
    // A page of five cells with two of them taken off again, so that it
    // holds a free list of two blocks, and then every byte that says
    // where the free space is set to each of a few values. What each one
    // breaks is a rule of section 1.6, and every one of them has to be a
    // refusal rather than a write past the page.
    let mut good = build(Kind::LeafTable, 2, 256, 256, &[], None).unwrap();
    {
        let mut writer = Writer::open(&mut good, 2, 256).unwrap();
        for at in 0..5 {
            let cell = leaf_cell(i64::try_from(at).unwrap() + 1, 30);
            assert!(writer.insert(at, &cell).unwrap());
        }
        writer.remove(3).unwrap();
        writer.remove(1).unwrap();
    }
    let first = usize::from(u16::from_be_bytes([good[1], good[2]]));
    let second = usize::from(u16::from_be_bytes([good[first], good[first + 1]]));
    assert!(first > 0 && second > first, "the page holds two freeblocks");
    let mut edits = alloc::vec![1, 2, 5, 6, 7];
    for block in [first, second] {
        edits.extend([block, block + 1, block + 2, block + 3]);
    }
    // The cell pointers, and the length and the key of each cell, which
    // is what says how far a cell reaches.
    {
        let page = Page::parse(&good, 2, 256).unwrap();
        for at in 0..page.cells() {
            edits.push(8 + at * 2);
            edits.push(9 + at * 2);
            let offset = page.cell_offset(at).unwrap();
            edits.extend([offset, offset + 1, offset + 2]);
        }
    }
    let small = leaf_cell(9, 8);
    let big = leaf_cell(9, 60);
    let mut refused = 0;
    let mut written = 0;
    for at in edits {
        for value in [0_u8, 1, 2, 4, 0x7f, 0x80, 0xfe, 0xff] {
            for step in 0..5 {
                let mut copy = good.clone();
                copy[at] = value;
                let Ok(mut writer) = Writer::open(&mut copy, 2, 256) else {
                    continue;
                };
                let answer = match step {
                    0 => writer.free().map(|_| true),
                    1 => writer.insert(0, &small),
                    2 => writer.insert(3, &big),
                    3 => writer.remove(0).map(|()| true),
                    _ => writer.remove(2).map(|()| true),
                };
                match answer {
                    Ok(_) => written += 1,
                    Err(error) => {
                        assert!(
                            matches!(error, Error::FreeBlock | Error::Overrun | Error::Varint),
                            "a broken page refused with {error:?}"
                        );
                        refused += 1;
                    }
                }
                // Whatever the page said, it is still a page: its cells
                // are read back or refused, and none of them is read
                // from outside it.
                let Ok(page) = Page::parse(&copy, 2, 256) else {
                    continue;
                };
                for index in 0..page.cells() {
                    let _ = page.cell(index);
                }
            }
        }
    }
    assert!(
        refused > 20,
        "only {refused} of the broken pages were refused"
    );
    assert!(
        written > 20,
        "only {written} of the broken pages were written"
    );
}

/// The steps of a write, each asked of its own copy of a page.
fn steps(good: &[u8], at: usize, value: u8, refused: &mut u32) {
    use crate::page::Writer;
    for step in 0..13 {
        let mut copy = good.to_vec();
        copy[at] = value;
        let Ok(mut writer) = Writer::open(&mut copy, 2, 256) else {
            return;
        };
        let answer = match step {
            0 => writer.free().map(|_| ()),
            1 => writer.slot(4).map(|_| ()),
            2 => writer.slot(20).map(|_| ()),
            3 => writer.slot(34).map(|_| ()),
            4 => writer.slot(36).map(|_| ()),
            5 => writer.release(200, 20),
            6 => writer.release(60, 20),
            7 => writer.release(20, 4),
            8 => writer.defragment(Some(4)),
            9 => writer.defragment(None),
            10 => writer.allocate(20).map(|_| ()),
            11 => writer.allocate(200).map(|_| ()),
            _ => writer.remove(0),
        };
        if let Err(error) = answer {
            assert!(
                matches!(error, Error::FreeBlock | Error::Overrun | Error::Varint),
                "a broken page refused with {error:?}"
            );
            *refused += 1;
        }
        // Whatever the step said, the page is still one: every cell of
        // it is read back or refused, and none from outside it.
        if let Ok(page) = Page::parse(&copy, 2, 256) {
            for index in 0..page.cells() {
                let _ = page.cell(index);
            }
        }
    }
}

#[test]
fn every_step_of_a_write_refuses_a_page_that_does_not_count_up() {
    use crate::page::{Writer, build};
    // The steps are asked one by one rather than through `insert`,
    // because `insert` counts the free space first and a page that does
    // not count up never reaches the steps after it. One page holds a
    // free list of two blocks and one holds none, because what a step
    // does with an empty list is its own path.
    let mut pieces = build(Kind::LeafTable, 2, 256, 256, &[], None).unwrap();
    {
        let mut writer = Writer::open(&mut pieces, 2, 256).unwrap();
        for at in 0..5 {
            let cell = leaf_cell(i64::try_from(at).unwrap() + 1, 30);
            assert!(writer.insert(at, &cell).unwrap());
        }
        writer.remove(3).unwrap();
        writer.remove(1).unwrap();
    }
    let mut whole = build(Kind::LeafTable, 2, 256, 256, &[], None).unwrap();
    {
        let mut writer = Writer::open(&mut whole, 2, 256).unwrap();
        for at in 0..4 {
            let cell = leaf_cell(i64::try_from(at).unwrap() + 1, 30);
            assert!(writer.insert(at, &cell).unwrap());
        }
    }
    let mut refused = 0;
    for good in [&pieces, &whole] {
        for at in 0..good.len() {
            for value in [
                0_u8, 1, 2, 3, 4, 8, 0x3f, 0x40, 0x7f, 0x80, 0xc0, 0xfe, 0xff,
            ] {
                steps(good, at, value, &mut refused);
            }
        }
    }
    assert!(refused > 50, "only {refused} steps refused a broken page");
}

#[test]
fn what_a_page_refuses_where_its_own_numbers_do_not_allow_the_step() {
    use crate::page::{Writer, build};
    // A page of five cells with the second and the fourth taken off, so
    // that it holds two freeblocks of a size this test knows.
    let mut bytes = build(Kind::LeafTable, 2, 256, 256, &[], None).unwrap();
    {
        let mut writer = Writer::open(&mut bytes, 2, 256).unwrap();
        for at in 0..5 {
            let cell = leaf_cell(i64::try_from(at).unwrap() + 1, 30);
            assert!(writer.insert(at, &cell).unwrap());
        }
        writer.remove(3).unwrap();
        writer.remove(1).unwrap();
    }
    let first = usize::from(u16::from_be_bytes([bytes[1], bytes[2]]));
    let size = usize::from(u16::from_be_bytes([bytes[first + 2], bytes[first + 3]]));
    assert_eq!(size, 32);

    // Bytes that end two short of a freeblock take those two into the
    // block, and a page that says it holds no fragmented byte cannot
    // give two back.
    let mut copy = bytes.clone();
    let mut writer = Writer::open(&mut copy, 2, 256).unwrap();
    assert_eq!(writer.release(first - 6, 4), Err(Error::FreeBlock));

    // A block between one and three bytes wider than what is asked for
    // is taken whole and the rest counted as fragments, which a page
    // already holding fifty-eight of them has no room for.
    let mut copy = bytes.clone();
    copy[7] = 58;
    let mut writer = Writer::open(&mut copy, 2, 256).unwrap();
    assert_eq!(writer.slot(size - 2), Ok(None));
    // With room for them the block goes, and the two bytes are counted.
    let mut copy = bytes.clone();
    copy[7] = 57;
    let mut writer = Writer::open(&mut copy, 2, 256).unwrap();
    assert_eq!(writer.slot(size - 2), Ok(Some(first)));
    assert_eq!(copy[7], 59);

    // A free list that begins inside the cell pointer array names space
    // the cells themselves are in.
    let mut copy = bytes.clone();
    copy[1] = 0;
    copy[2] = 10;
    copy[10] = 0;
    copy[11] = 0;
    copy[12] = 0;
    copy[13] = 20;
    let mut writer = Writer::open(&mut copy, 2, 256).unwrap();
    assert_eq!(writer.allocate(20), Err(Error::FreeBlock));

    // A cell whose length says it reaches past the page is refused by
    // the reader, so the page never gives it back.
    let page = Page::parse(&bytes, 2, 256).unwrap();
    let offset = page.cell_offset(0).unwrap();
    let mut copy = bytes.clone();
    copy[offset] = 100;
    let mut writer = Writer::open(&mut copy, 2, 256).unwrap();
    assert_eq!(writer.remove(0), Err(Error::Overrun));
}

#[test]
fn a_leaf_has_no_pointer_to_point() {
    use crate::page::{Writer, build};
    // The right-most pointer is the eight bytes only an interior page's
    // header is longer by.
    let mut bytes = build(Kind::LeafTable, 2, 256, 256, &[], None).unwrap();
    let mut writer = Writer::open(&mut bytes, 2, 256).unwrap();
    assert_eq!(writer.point(3), Err(Error::PageKind(13)));
    let mut bytes = build(Kind::InteriorTable, 2, 256, 256, &[], None).unwrap();
    let mut writer = Writer::open(&mut bytes, 2, 256).unwrap();
    assert_eq!(writer.point(3), Ok(()));
    assert_eq!(writer.page().right_most(), Some(3));
}

#[test]
fn a_leaf_has_no_cell_that_names_a_child() {
    use crate::page::{Writer, build};
    let mut bytes = build(Kind::LeafTable, 2, 256, 256, &[], None).unwrap();
    let mut writer = Writer::open(&mut bytes, 2, 256).unwrap();
    assert!(writer.insert(0, &leaf_cell(1, 10)).unwrap());
    assert_eq!(writer.point_child(0, 3), Err(Error::PageKind(13)));
}

#[test]
fn what_writing_a_page_again_refuses() {
    use crate::page::{Source, Writer, build};
    // Four cells on the page and a header that says three, so that the
    // page gives back more cells than it holds.
    let mut bytes = build(Kind::LeafTable, 2, 512, 512, &[], None).unwrap();
    {
        let mut writer = Writer::open(&mut bytes, 2, 512).unwrap();
        for at in 0..4 {
            assert!(
                writer
                    .insert(at, &leaf_cell(i64::try_from(at).unwrap() + 1, 30))
                    .unwrap()
            );
        }
    }
    let cells: Vec<Vec<u8>> = (0..4)
        .map(|at| {
            let page = Page::parse(&bytes, 2, 512).unwrap();
            let Cell::TableLeaf { .. } = page.cell(at).unwrap() else {
                panic!("a cell of another shape")
            };
            crate::page::write_cell(&page.cell(at).unwrap())
        })
        .collect();
    let sources: Vec<Source<'_>> = (0..4)
        .map(|at| {
            let page = Page::parse(&bytes, 2, 512).unwrap();
            Source {
                bytes: &cells[at],
                from: Some((2, page.cell_offset(at).unwrap())),
            }
        })
        .collect();
    let mut short = bytes.clone();
    short[4] = 3;
    let mut writer = Writer::open(&mut short, 2, 512).unwrap();
    assert_eq!(writer.edit(0, 4, 0, &sources, &[]), Err(Error::FreeBlock));

    // A page whose content area begins past the bytes the b-tree may
    // use is written again from the cells it is to hold.
    let mut past = bytes.clone();
    past[5] = 2;
    past[6] = 2;
    let mut writer = Writer::open(&mut past, 2, 512).unwrap();
    assert_eq!(writer.edit(0, 0, 4, &sources, &[]), Ok(()));
    assert_eq!(Page::parse(&past, 2, 512).unwrap().cells(), 4);

    // And where the cells are more than the page holds, the page
    // refuses them.
    let wide: Vec<Vec<u8>> = (0..6).map(|at| leaf_cell(i64::from(at) + 1, 100)).collect();
    let sources: Vec<Source<'_>> = wide
        .iter()
        .map(|cell| Source {
            bytes: cell,
            from: None,
        })
        .collect();
    let mut past = bytes.clone();
    past[5] = 2;
    past[6] = 2;
    let mut writer = Writer::open(&mut past, 2, 512).unwrap();
    assert_eq!(writer.edit(0, 0, 6, &sources, &[]), Err(Error::Balance));

    // A page whose free list says a block longer than the page holds is
    // one the cells cannot be written onto.
    let mut broken = build(Kind::LeafTable, 2, 512, 512, &[], None).unwrap();
    {
        let mut writer = Writer::open(&mut broken, 2, 512).unwrap();
        for at in 0..5 {
            assert!(
                writer
                    .insert(at, &leaf_cell(i64::try_from(at).unwrap() + 1, 30))
                    .unwrap()
            );
        }
        writer.remove(3).unwrap();
        writer.remove(1).unwrap();
    }
    let held: Vec<Vec<u8>> = (0..3)
        .map(|at| {
            let page = Page::parse(&broken, 2, 512).unwrap();
            crate::page::write_cell(&page.cell(at).unwrap())
        })
        .collect();
    let added = leaf_cell(9, 30);
    let mut sources: Vec<Source<'_>> = (0..3)
        .map(|at| {
            let page = Page::parse(&broken, 2, 512).unwrap();
            Source {
                bytes: &held[at],
                from: Some((2, page.cell_offset(at).unwrap())),
            }
        })
        .collect();
    sources.push(Source {
        bytes: &added,
        from: None,
    });
    let first = usize::from(u16::from_be_bytes([broken[1], broken[2]]));
    broken[first + 2] = 1;
    broken[first + 3] = 244;
    let mut writer = Writer::open(&mut broken, 2, 512).unwrap();
    assert_eq!(writer.edit(0, 0, 4, &sources, &[]), Err(Error::FreeBlock));
}
