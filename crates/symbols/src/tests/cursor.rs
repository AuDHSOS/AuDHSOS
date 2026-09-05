// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::cursor`.

use crate::cursor::Cursor;
use crate::error::SymbolError;

use super::build::{sleb, uleb};

#[test]
fn numbers_come_out_little_endian_and_in_order() {
    let bytes = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0xFF];
    let mut cursor = Cursor::new(&bytes);
    assert_eq!(cursor.u8().unwrap(), 1);
    assert_eq!(cursor.u16().unwrap(), 0x0302);
    assert_eq!(cursor.u32().unwrap(), 0x0706_0504);
    assert_eq!(cursor.i8().unwrap(), -1);
    assert!(cursor.is_empty());
    assert_eq!(cursor.remaining(), 0);
    let mut whole = Cursor::new(&bytes);
    assert_eq!(whole.u64().unwrap(), 0xFF07_0605_0403_0201);
}

#[test]
fn a_read_beyond_the_end_is_an_error_and_not_a_panic() {
    let bytes = [0u8; 3];
    let mut cursor = Cursor::new(&bytes);
    assert_eq!(cursor.u32(), Err(SymbolError::Truncated));
    assert_eq!(cursor.u64(), Err(SymbolError::Truncated));
    assert_eq!(cursor.take(4), Err(SymbolError::Truncated));
    assert_eq!(cursor.seek(4), Err(SymbolError::Truncated));
    assert_eq!(Cursor::new(&[]).u8(), Err(SymbolError::Truncated));
}

#[test]
fn seeking_moves_the_cursor_and_the_end_is_a_position() {
    let bytes = [1u8, 2, 3];
    let mut cursor = Cursor::new(&bytes);
    cursor.seek(2).unwrap();
    assert_eq!(cursor.position(), 2);
    assert_eq!(cursor.u8().unwrap(), 3);
    cursor.seek(3).unwrap();
    assert!(cursor.is_empty());
    cursor.seek(0).unwrap();
    cursor.skip(1).unwrap();
    assert_eq!(cursor.position(), 1);
}

#[test]
fn a_number_of_a_chosen_width_reads_the_low_bytes() {
    let bytes = [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22];
    let mut cursor = Cursor::new(&bytes);
    assert_eq!(cursor.unsigned(3).unwrap(), 0x00CC_BBAA);
    let mut whole = Cursor::new(&bytes);
    assert_eq!(whole.unsigned(8).unwrap(), 0x2211_FFEE_DDCC_BBAA);
    assert_eq!(Cursor::new(&bytes).unsigned(9), Err(SymbolError::Overflow));
    assert_eq!(Cursor::new(&bytes).unsigned(0).unwrap(), 0);
}

#[test]
fn unsigned_variable_length_numbers_round_trip() {
    for value in [0u64, 1, 63, 64, 127, 128, 300, 0x3FFF, 0x4000, u64::MAX] {
        let bytes = uleb(value);
        assert_eq!(Cursor::new(&bytes).uleb().unwrap(), value, "{value}");
    }
}

#[test]
fn signed_variable_length_numbers_round_trip() {
    for value in [
        0i64,
        1,
        -1,
        63,
        -64,
        64,
        -65,
        127,
        -128,
        1000,
        -1000,
        i32::MIN.into(),
    ] {
        let bytes = sleb(value);
        assert_eq!(Cursor::new(&bytes).sleb().unwrap(), value, "{value}");
    }
}

#[test]
fn a_variable_length_number_that_never_ends_is_an_error() {
    let bytes = [0x80u8; 4];
    assert_eq!(Cursor::new(&bytes).uleb(), Err(SymbolError::Truncated));
    assert_eq!(Cursor::new(&bytes).sleb(), Err(SymbolError::Truncated));
}

#[test]
fn a_variable_length_number_wider_than_sixty_four_bits_is_an_error() {
    let mut bytes = vec![0xFFu8; 10];
    bytes.push(0x7F);
    assert_eq!(Cursor::new(&bytes).uleb(), Err(SymbolError::Overflow));
}

#[test]
fn a_variable_length_number_padded_with_zero_bytes_still_reads() {
    let bytes = [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00];
    assert_eq!(Cursor::new(&bytes).uleb().unwrap(), 0);
}

#[test]
fn a_number_that_fits_an_offset_comes_out_as_one() {
    assert_eq!(Cursor::new(&uleb(7)).uleb_usize().unwrap(), 7);
    // A number too large for an offset is refused, which on a machine
    // whose offsets are 64 bits wide no `u64` is.
    let widest = Cursor::new(&uleb(u64::MAX)).uleb_usize();
    match usize::try_from(u64::MAX) {
        Ok(value) => assert_eq!(widest.unwrap(), value),
        Err(_) => assert_eq!(widest, Err(SymbolError::Overflow)),
    }
}

#[test]
fn strings_end_at_the_terminator_and_the_cursor_moves_past_it() {
    let bytes = b"first\0second\0";
    let mut cursor = Cursor::new(bytes);
    assert_eq!(cursor.string().unwrap(), "first");
    assert_eq!(cursor.string().unwrap(), "second");
    assert!(cursor.is_empty());
}

#[test]
fn a_string_without_a_terminator_is_an_error() {
    assert_eq!(Cursor::new(b"open").string(), Err(SymbolError::Truncated));
    assert_eq!(
        Cursor::new(&[0xFF, 0xFE, 0x00]).string(),
        Err(SymbolError::Truncated),
        "bytes that are not text are refused"
    );
}
