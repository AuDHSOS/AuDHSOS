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
    // The schema names the two indexes; their roots are index pages.
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
    assert_eq!(roots.len(), 2);
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
