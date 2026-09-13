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
