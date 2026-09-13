// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::image`.

#![allow(clippy::arithmetic_side_effects)]

use crate::error::Error;
use crate::image::Image;
use crate::record::Value;
use crate::tests::{INDEXED, OVERFLOW, PAGE512, SMALL, UTF16};

/// The root page of the table named `name`. The name is given in the
/// file's own encoding, because the schema table is stored in it like
/// every other table.
fn root_of(image: &Image<'_>, name: &[u8]) -> u32 {
    for row in image.schema() {
        let row = row.unwrap();
        let record = row.record().unwrap();
        if record.value(2).unwrap() == Some(Value::Text(name))
            && let Some(Value::Int(root)) = record.value(3).unwrap()
        {
            return u32::try_from(root).unwrap();
        }
    }
    panic!("no table named {name:?}");
}

#[test]
fn a_file_is_as_many_pages_as_it_is_long() {
    let image = Image::open(SMALL).unwrap();
    assert_eq!(image.pages(), 2);
    assert_eq!(image.header().page_size, 4096);
    assert_eq!(Image::open(PAGE512).unwrap().pages(), 15);
}

#[test]
fn page_zero_and_a_page_past_the_end_are_both_refused() {
    let image = Image::open(SMALL).unwrap();
    assert_eq!(image.page_bytes(0), Err(Error::Page(0)));
    assert_eq!(image.page_bytes(3), Err(Error::Page(3)));
    assert_eq!(image.page_bytes(u32::MAX), Err(Error::Page(u32::MAX)));
    assert_eq!(image.page_bytes(1).map(<[u8]>::len), Ok(4096));
}

#[test]
fn the_schema_names_every_table_of_the_file() {
    let image = Image::open(SMALL).unwrap();
    let mut names = Vec::new();
    for row in image.schema() {
        let record = row.unwrap().record().unwrap();
        let Some(Value::Text(kind)) = record.value(0).unwrap() else {
            panic!("the first column of the schema is its type");
        };
        let Some(Value::Text(name)) = record.value(1).unwrap() else {
            panic!("the second column of the schema is a name");
        };
        names.push((kind.to_vec(), name.to_vec()));
    }
    assert_eq!(names, [(b"table".to_vec(), b"t".to_vec())]);
}

#[test]
fn the_rows_of_a_table_come_back_as_the_shell_wrote_them() {
    let image = Image::open(SMALL).unwrap();
    let root = root_of(&image, b"t");
    let mut rows = Vec::new();
    for row in image.rows(root) {
        let row = row.unwrap();
        let record = row.record().unwrap();
        let values: Vec<_> = record.values().collect::<Result<_, _>>().unwrap();
        rows.push((row.rowid, values));
    }
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].0, 1);
    assert_eq!(
        rows[0].1,
        [
            Value::Int(1),
            Value::Text(b"one"),
            Value::Real(1.5),
            Value::Blob(&[1, 2]),
        ]
    );
    assert_eq!(
        rows[1].1,
        [
            Value::Int(2),
            Value::Text(b"two"),
            Value::Real(-2.5),
            Value::Null,
        ]
    );
    assert_eq!(
        rows[2].1,
        [
            Value::Int(-3),
            Value::Text(b""),
            // A real with no fractional part is stored as an integer;
            // section 2.1 calls it an internal optimization, and reading
            // it back as a real needs the column's affinity, which this
            // layer does not have.
            Value::Int(0),
            Value::Blob(&[0xff]),
        ]
    );
}

#[test]
fn a_tree_with_interior_pages_is_walked_in_rowid_order_and_whole() {
    let image = Image::open(PAGE512).unwrap();
    let root = root_of(&image, b"wide");
    let mut seen = 0i64;
    for row in image.rows(root) {
        let row = row.unwrap();
        seen += 1;
        assert_eq!(row.rowid, seen);
        let record = row.record().unwrap();
        assert_eq!(record.value(0).unwrap(), Some(Value::Int(seen)));
    }
    assert_eq!(seen, 400);
}

#[test]
fn text_comes_back_in_the_encoding_the_header_names() {
    let image = Image::open(UTF16).unwrap();
    let root = root_of(&image, b"u\0");
    let mut texts = Vec::new();
    for row in image.rows(root) {
        let record = row.unwrap().record().unwrap();
        let Some(Value::Text(text)) = record.value(0).unwrap() else {
            panic!("the column holds text");
        };
        texts.push(text.to_vec());
    }
    // 'abc' as UTF-16le, and the three letters with an umlaut after it.
    assert_eq!(texts[0], b"a\0b\0c\0");
    assert_eq!(texts[1], [0xe4, 0x00, 0xf6, 0x00, 0xfc, 0x00]);
}

#[test]
fn a_payload_that_does_not_fit_its_page_is_read_off_the_chain_that_follows() {
    let image = Image::open(OVERFLOW).unwrap();
    let root = root_of(&image, b"big");
    let mut rows = image.rows(root);
    let row = rows.next().unwrap().unwrap();
    let payload = row.payload;
    assert!(!payload.is_whole());
    assert!(payload.local.len() < payload.total);
    assert_eq!(row.record(), Err(Error::Overrun));

    let mut whole = vec![0u8; payload.total];
    assert_eq!(image.read_payload(&payload, &mut whole), Ok(payload.total));
    let record = crate::record::Record::parse(&whole).unwrap();
    let Some(Value::Text(text)) = record.value(0).unwrap() else {
        panic!("the column holds text");
    };
    assert_eq!(text.len(), 18000);
    assert!(text.iter().all(|byte| *byte == b'x'));
    assert!(rows.next().is_none());
}

#[test]
fn a_buffer_shorter_than_the_payload_is_refused_rather_than_filled() {
    let image = Image::open(OVERFLOW).unwrap();
    let root = root_of(&image, b"big");
    let payload = image.rows(root).next().unwrap().unwrap().payload;
    let mut too_small = vec![0u8; payload.total - 1];
    assert_eq!(
        image.read_payload(&payload, &mut too_small),
        Err(Error::Overrun)
    );
}

#[test]
fn a_whole_payload_needs_no_chain_to_be_read() {
    let image = Image::open(SMALL).unwrap();
    let root = root_of(&image, b"t");
    let payload = image.rows(root).next().unwrap().unwrap().payload;
    assert!(payload.is_whole());
    let mut into = vec![0u8; payload.total];
    assert_eq!(image.read_payload(&payload, &mut into), Ok(payload.total));
    assert_eq!(into, payload.local);
}

#[test]
fn a_root_that_names_an_index_tree_is_refused_by_a_walk_of_rows() {
    let image = Image::open(INDEXED).unwrap();
    let mut index_root = 0;
    for row in image.schema() {
        let record = row.unwrap().record().unwrap();
        if record.value(0).unwrap() == Some(Value::Text(b"index"))
            && let Some(Value::Int(root)) = record.value(3).unwrap()
        {
            index_root = u32::try_from(root).unwrap();
        }
    }
    assert!(index_root > 0);
    let mut rows = image.rows(index_root);
    // The refusal names the page type that was found where a table tree
    // has none: ten is a leaf of an index tree.
    assert_eq!(rows.next(), Some(Err(Error::PageKind(10))));
    assert_eq!(rows.next(), None);
}

#[test]
fn a_walk_that_starts_at_a_page_the_file_does_not_have_ends_with_that() {
    let image = Image::open(SMALL).unwrap();
    let mut rows = image.rows(99);
    assert_eq!(rows.next(), Some(Err(Error::Page(99))));
    assert_eq!(rows.next(), None);
}

#[test]
fn a_hundred_rows_of_a_table_with_indexes_are_all_there() {
    let image = Image::open(INDEXED).unwrap();
    let root = root_of(&image, b"k");
    assert_eq!(image.rows(root).count(), 100);
}

#[test]
fn a_chain_that_ends_before_the_payload_does_is_refused() {
    // Three thousand bytes, four hundred and sixty of them on the page —
    // which is what the threshold rule leaves there — and one overflow
    // page that says the chain ends after it.
    let page = crate::tests::leaf_with_overflow(512, 3000, 460, 3);
    let bytes = crate::tests::Builder::new(512)
        .page(&page)
        .page(&[0, 0, 0, 0])
        .finish();
    let image = Image::open(&bytes).unwrap();
    let row = image.rows(2).next().unwrap().unwrap();
    let mut into = vec![0u8; row.payload.total];
    assert_eq!(
        image.read_payload(&row.payload, &mut into),
        Err(Error::Overflow(3))
    );
}

#[test]
fn a_chain_that_turns_back_on_itself_is_refused_rather_than_followed() {
    let page = crate::tests::leaf_with_overflow(512, 3000, 460, 3);
    // Page 3 points at itself, so the walk would never end.
    let mut overflow = vec![0u8; 512];
    overflow[..4].copy_from_slice(&3u32.to_be_bytes());
    let bytes = crate::tests::Builder::new(512)
        .page(&page)
        .page(&overflow)
        .finish();
    let image = Image::open(&bytes).unwrap();
    let row = image.rows(2).next().unwrap().unwrap();
    let mut into = vec![0u8; row.payload.total];
    assert_eq!(
        image.read_payload(&row.payload, &mut into),
        Err(Error::Overflow(3))
    );
}

#[test]
fn a_chain_that_leaves_the_file_is_refused_by_the_page_it_names() {
    let page = crate::tests::leaf_with_overflow(512, 3000, 460, 9);
    let bytes = crate::tests::Builder::new(512).page(&page).finish();
    let image = Image::open(&bytes).unwrap();
    let row = image.rows(2).next().unwrap().unwrap();
    let mut into = vec![0u8; row.payload.total];
    assert_eq!(
        image.read_payload(&row.payload, &mut into),
        Err(Error::Page(9))
    );
}

#[test]
fn a_tree_deeper_than_the_walk_is_refused_at_the_depth_it_stops_at() {
    // Thirty-three interior pages, each pointing at the next: one more
    // than the walk keeps frames for.
    let mut builder = crate::tests::Builder::new(512);
    for page in 0..33u32 {
        builder = builder.page(&crate::tests::interior_to(512, page + 3));
    }
    let bytes = builder.finish();
    let image = Image::open(&bytes).unwrap();
    let mut rows = image.rows(2);
    assert_eq!(rows.next(), Some(Err(Error::Depth)));
    assert_eq!(rows.next(), None);
}

#[test]
fn a_payload_that_says_it_continues_at_page_zero_is_refused() {
    let page = crate::tests::leaf_with_overflow(512, 600, 92, 0);
    let bytes = crate::tests::Builder::new(512).page(&page).finish();
    let image = Image::open(&bytes).unwrap();
    let mut rows = image.rows(2);
    assert_eq!(rows.next(), Some(Err(Error::Overflow(0))));
}

#[test]
fn a_file_that_is_not_a_database_is_refused_by_opening_it() {
    assert_eq!(Image::open(&[]), Err(Error::Truncated));
    assert_eq!(Image::open(&[0u8; 200]), Err(Error::Magic));
}
