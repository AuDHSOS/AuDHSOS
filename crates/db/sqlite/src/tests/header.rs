// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::header`.

#![allow(clippy::arithmetic_side_effects)]

use crate::error::Error;
use crate::header::{Encoding, HEADER_LEN, Header, MAGIC};
use crate::tests::{PAGE512, SMALL, UTF16};

/// The header of the small fixture, with `edit` applied to its bytes.
fn edited(edit: impl FnOnce(&mut [u8; HEADER_LEN])) -> Result<Header, Error> {
    let mut bytes = [0u8; HEADER_LEN];
    bytes.copy_from_slice(&SMALL[..HEADER_LEN]);
    edit(&mut bytes);
    Header::parse(&bytes)
}

#[test]
fn the_header_of_a_file_the_shell_wrote_reads_as_the_shell_wrote_it() {
    let header = Header::parse(SMALL).unwrap();
    assert_eq!(header.page_size, 4096);
    assert_eq!(header.reserved, 0);
    assert_eq!(header.usable(), 4096);
    assert_eq!(header.write_version, 1);
    assert_eq!(header.read_version, 1);
    assert_eq!(header.encoding, Encoding::Utf8);
    assert_eq!(header.schema_format, 4);
    assert_eq!(header.freelist, 0);
    assert_eq!(header.freelist_pages, 0);
    assert_eq!(header.pages, 2);
    assert_eq!(header.version_valid_for, header.change_counter);
    assert!(header.library_version >= 3_053_004);
}

#[test]
fn a_page_size_of_five_hundred_and_twelve_is_read_as_one() {
    assert_eq!(Header::parse(PAGE512).unwrap().page_size, 512);
}

#[test]
fn the_encoding_travels_in_the_header_and_not_in_the_rows() {
    assert_eq!(Header::parse(UTF16).unwrap().encoding, Encoding::Utf16Le);
    assert_eq!(Encoding::Utf8.code(), 1);
    assert_eq!(Encoding::Utf16Le.code(), 2);
    assert_eq!(Encoding::Utf16Be.code(), 3);
    assert_eq!(Encoding::from_code(3), Ok(Encoding::Utf16Be));
    assert_eq!(Encoding::from_code(0), Err(Error::Encoding(0)));
    assert_eq!(Encoding::from_code(4), Err(Error::Encoding(4)));
}

#[test]
fn a_file_that_does_not_begin_with_the_header_string_is_not_a_database() {
    assert_eq!(edited(|bytes| bytes[0] = b'X'), Err(Error::Magic));
    assert_eq!(Header::parse(MAGIC.as_slice()), Err(Error::Truncated));
    assert_eq!(Header::parse(&[]), Err(Error::Truncated));
    assert_eq!(
        Header::parse(&SMALL[..HEADER_LEN - 1]),
        Err(Error::Truncated)
    );
}

#[test]
fn the_page_size_field_holds_one_for_the_largest_page_and_refuses_the_rest() {
    let with_size = |high: u8, low: u8| {
        edited(|bytes| {
            bytes[16] = high;
            bytes[17] = low;
        })
    };
    assert_eq!(with_size(0, 1).unwrap().page_size, 65536);
    assert_eq!(with_size(2, 0).unwrap().page_size, 512);
    assert_eq!(with_size(0, 0), Err(Error::PageSize(0)));
    assert_eq!(with_size(0, 2), Err(Error::PageSize(2)));
    assert_eq!(with_size(1, 0), Err(Error::PageSize(256)));
    assert_eq!(with_size(0x30, 0), Err(Error::PageSize(12288)));
}

#[test]
fn reserved_space_that_leaves_too_little_of_a_page_is_refused() {
    assert_eq!(edited(|bytes| bytes[20] = 16).unwrap().usable(), 4080);
    // A 512-byte page with 33 bytes reserved leaves 479, one short of the
    // 480 the format asks for.
    let small_page = |reserved: u8| {
        edited(|bytes| {
            bytes[16] = 2;
            bytes[17] = 0;
            bytes[20] = reserved;
        })
    };
    assert_eq!(small_page(32).unwrap().usable(), 480);
    assert_eq!(small_page(33), Err(Error::Reserved(33)));
}

#[test]
fn the_three_fixed_fractions_are_checked_and_not_assumed() {
    assert_eq!(edited(|bytes| bytes[21] = 63), Err(Error::Fractions));
    assert_eq!(edited(|bytes| bytes[22] = 31), Err(Error::Fractions));
    assert_eq!(edited(|bytes| bytes[23] = 33), Err(Error::Fractions));
}

#[test]
fn an_encoding_the_format_does_not_have_is_refused_with_its_code() {
    assert_eq!(edited(|bytes| bytes[59] = 9), Err(Error::Encoding(9)));
}
