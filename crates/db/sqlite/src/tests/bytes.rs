// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::bytes`.

#![allow(clippy::arithmetic_side_effects)]

use crate::bytes::{signed, size, u8_at, u16_at, u32_at, varint};
use crate::error::Error;
use crate::tests::varint as write_varint;

#[test]
fn a_number_that_does_not_fit_the_bytes_is_not_read() {
    let bytes = [0x01, 0x02, 0x03];
    assert_eq!(u8_at(&bytes, 2), Some(3));
    assert_eq!(u8_at(&bytes, 3), None);
    assert_eq!(u16_at(&bytes, 0), Some(0x0102));
    assert_eq!(u16_at(&bytes, 2), None);
    assert_eq!(u32_at(&bytes, 0), None);
    assert_eq!(u32_at(&[1, 2, 3, 4], 0), Some(0x0102_0304));
    assert_eq!(u16_at(&bytes, usize::MAX), None);
    assert_eq!(u32_at(&bytes, usize::MAX), None);
}

#[test]
fn a_varint_reads_back_what_was_written() {
    for value in [
        0,
        1,
        127,
        128,
        0x3fff,
        0x4000,
        0x001f_ffff,
        0x0fff_ffff,
        0x00ff_ffff_ffff_ffff,
        0x0100_0000_0000_0000,
        u64::MAX,
        u64::MAX / 3,
    ] {
        let (bytes, len) = write_varint(value);
        assert_eq!(varint(&bytes), Ok((value, len)), "{value:#x}");
    }
}

#[test]
fn a_varint_of_nine_bytes_is_the_longest_there_is() {
    let (bytes, len) = write_varint(u64::MAX);
    assert_eq!(len, 9);
    assert_eq!(varint(&bytes), Ok((u64::MAX, 9)));
    // Every byte of the first eight carries its continuation bit.
    assert!(bytes.iter().take(8).all(|byte| byte & 0x80 != 0));
}

#[test]
fn a_varint_that_does_not_end_inside_its_bytes_is_refused() {
    assert_eq!(varint(&[]), Err(Error::Varint));
    assert_eq!(varint(&[0x81]), Err(Error::Varint));
    assert_eq!(varint(&[0x81; 8]), Err(Error::Varint));
    assert_eq!(varint(&[0x81; 9]).map(|(_, len)| len), Ok(9));
}

#[test]
fn a_rowid_is_the_same_bits_read_as_a_signed_number() {
    assert_eq!(signed(0), 0);
    assert_eq!(signed(1), 1);
    assert_eq!(signed(u64::MAX), -1);
    assert_eq!(signed(0x8000_0000_0000_0000), i64::MIN);
}

#[test]
fn a_length_wider_than_this_machine_saturates_rather_than_wrapping() {
    assert_eq!(size(7), 7);
    assert_eq!(size(u64::from(u32::MAX)), 4_294_967_295);
    assert!(size(u64::MAX) >= usize::MAX / 2);
}

#[test]
fn every_varint_written_reads_back_as_the_value_it_was_written_from() {
    use crate::bytes::{put_varint, varint_len};
    // The two sides of every group boundary, and the nine-byte form,
    // whose ninth byte carries all eight of its bits.
    let values = [
        0,
        1,
        127,
        128,
        16_383,
        16_384,
        2_097_151,
        2_097_152,
        268_435_455,
        268_435_456,
        34_359_738_367,
        34_359_738_368,
        4_398_046_511_103,
        4_398_046_511_104,
        562_949_953_421_311,
        562_949_953_421_312,
        72_057_594_037_927_935,
        72_057_594_037_927_936,
        u64::MAX,
    ];
    for value in values {
        let mut written = Vec::new();
        put_varint(&mut written, value);
        assert_eq!(varint(&written), Ok((value, written.len())), "{value}");
        // The length the writer takes is the length the counter says,
        // except past what eight groups of seven bits hold, where the
        // counter answers ten and the writer takes nine.
        assert_eq!(
            written.len(),
            varint_len(value).min(9),
            "the length of {value}"
        );
    }
}
