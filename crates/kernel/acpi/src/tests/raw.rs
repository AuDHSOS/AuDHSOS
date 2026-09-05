// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::raw`.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

use crate::raw::{array_at, sum_of, u8_at, u16_at, u32_at, u64_at};

/// Eight bytes with a value at every width.
const BYTES: [u8; 8] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];

#[test]
fn every_width_is_read_little_endian_at_its_offset() {
    assert_eq!(u8_at(&BYTES, 1), Some(0x02));
    assert_eq!(u16_at(&BYTES, 1), Some(0x0302));
    assert_eq!(u32_at(&BYTES, 1), Some(0x0504_0302));
    assert_eq!(u64_at(&BYTES, 0), Some(0x0807_0605_0403_0201));
    assert_eq!(array_at::<3>(&BYTES, 5), Some([0x06, 0x07, 0x08]));
}

#[test]
fn a_read_that_leaves_the_bytes_hands_nothing_out() {
    assert_eq!(u8_at(&BYTES, 8), None);
    assert_eq!(u16_at(&BYTES, 7), None);
    assert_eq!(u32_at(&BYTES, 5), None);
    assert_eq!(u64_at(&BYTES, 1), None);
    assert_eq!(array_at::<9>(&BYTES, 0), None);
}

#[test]
fn the_sum_covers_the_first_bytes_and_wraps() {
    assert_eq!(sum_of(&BYTES, 0), 0);
    assert_eq!(sum_of(&BYTES, 3), 6);
    assert_eq!(sum_of(&BYTES, 8), 36);
    assert_eq!(sum_of(&[0xFF, 0x01], 2), 0);
}

#[test]
fn a_length_beyond_the_bytes_sums_what_there_is() {
    assert_eq!(sum_of(&BYTES, 100), 36);
}
