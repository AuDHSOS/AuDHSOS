// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::mbr`.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use crate::SECTOR;
use crate::mbr::{PROTECTIVE_TYPE, is_protective, protective};

#[test]
fn the_record_claims_everything_above_itself() {
    let sector = protective(2048);
    assert_eq!(sector[446], 0x00, "the partition is not bootable");
    assert_eq!(&sector[447..450], &[0x00, 0x02, 0x00]);
    assert_eq!(sector[450], PROTECTIVE_TYPE);
    assert_eq!(&sector[451..454], &[0xFF, 0xFF, 0xFF]);
    assert_eq!(u32::from_le_bytes(sector[454..458].try_into().unwrap()), 1);
    assert_eq!(
        u32::from_le_bytes(sector[458..462].try_into().unwrap()),
        2047
    );
    assert_eq!(&sector[510..], &[0x55, 0xAA]);
    assert!(sector[462..510].iter().all(|byte| *byte == 0));
}

#[test]
fn a_device_beyond_a_thirty_two_bit_count_gets_the_largest_count_there_is() {
    let sector = protective(1 << 33);
    assert_eq!(
        u32::from_le_bytes(sector[458..462].try_into().unwrap()),
        u32::MAX
    );
}

#[test]
fn a_record_this_module_wrote_is_one_it_reads() {
    assert!(is_protective(&protective(2048)));
}

#[test]
fn a_block_without_the_signature_or_without_the_type_is_no_record() {
    assert!(!is_protective(&[0u8; SECTOR]));
    let mut no_signature = protective(2048);
    no_signature[511] = 0;
    assert!(!is_protective(&no_signature));
    let mut legacy = [0u8; SECTOR];
    legacy[510] = 0x55;
    legacy[511] = 0xAA;
    legacy[450] = 0x0C;
    assert!(!is_protective(&legacy), "a legacy table is not a record");
}

#[test]
fn the_type_is_looked_for_in_every_one_of_the_four_records() {
    let mut sector = [0u8; SECTOR];
    sector[510] = 0x55;
    sector[511] = 0xAA;
    sector[446 + 3 * 16 + 4] = PROTECTIVE_TYPE;
    assert!(is_protective(&sector));
}
