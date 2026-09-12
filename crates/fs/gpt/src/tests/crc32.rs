// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::crc32`.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use crate::crc32::{Crc32, POLYNOMIAL, crc32};

#[test]
fn the_check_value_of_the_algorithm_is_the_published_one() {
    // The check value every description of CRC-32/ISO-HDLC carries.
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(&[0u8]), 0xD202_EF8D);
    assert_eq!(POLYNOMIAL, 0xEDB8_8320);
}

#[test]
fn a_checksum_over_nothing_is_zero_and_the_default_is_that_checksum() {
    assert_eq!(crc32(&[]), 0);
    assert_eq!(Crc32::default(), Crc32::new());
    assert_eq!(Crc32::default().finish(), 0);
}

#[test]
fn feeding_the_bytes_in_pieces_is_feeding_them_at_once() {
    let bytes: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
    let whole = crc32(&bytes);
    for split in [0usize, 1, 511, 512, 999, 1000] {
        let mut running = Crc32::new();
        running.update(&bytes[..split]);
        running.update(&bytes[split..]);
        assert_eq!(running.finish(), whole, "split at {split}");
    }
}

#[test]
fn a_single_bit_changes_the_checksum() {
    let mut bytes = [0u8; 512];
    let zeroed = crc32(&bytes);
    bytes[300] = 1;
    assert_ne!(crc32(&bytes), zeroed);
}
