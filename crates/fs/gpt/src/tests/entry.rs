// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::entry`.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use crate::entry::{ENTRY_LEN, ESP_TYPE_GUID, Entry, NAME_UNITS, UNUSED_TYPE_GUID};
use crate::error::Error;

use super::support::PARTITION_GUID;

fn esp(first: u64, last: u64) -> Entry {
    Entry::new(ESP_TYPE_GUID, PARTITION_GUID, first, last, "EFI").expect("an entry")
}

#[test]
fn an_entry_written_and_read_back_is_the_entry() {
    let entry = esp(2048, 4095);
    let mut bytes = [0u8; ENTRY_LEN];
    entry.write(&mut bytes);
    assert_eq!(Entry::parse(&bytes), entry);
    assert_eq!(&bytes[..16], &ESP_TYPE_GUID);
    assert_eq!(u64::from_le_bytes(bytes[32..40].try_into().unwrap()), 2048);
    assert_eq!(u64::from_le_bytes(bytes[40..48].try_into().unwrap()), 4095);
    assert_eq!(
        u16::from_le_bytes(bytes[56..58].try_into().unwrap()),
        0x0045
    );
}

#[test]
fn a_name_of_every_unit_the_entry_holds_fits_and_one_more_does_not() {
    let full = "x".repeat(NAME_UNITS);
    let entry = Entry::new(ESP_TYPE_GUID, PARTITION_GUID, 34, 35, &full).expect("an entry");
    assert_eq!(entry.name[NAME_UNITS - 1], u16::from(b'x'));
    let over = "x".repeat(NAME_UNITS + 1);
    assert_eq!(
        Entry::new(ESP_TYPE_GUID, PARTITION_GUID, 34, 35, &over),
        Err(Error::Name)
    );
}

#[test]
fn a_name_outside_the_basic_plane_costs_the_two_units_it_takes() {
    // One code point, two UTF-16 units, so seventeen of them are one
    // unit too many for an entry.
    let long = "\u{1F600}".repeat(NAME_UNITS / 2);
    assert!(Entry::new(ESP_TYPE_GUID, PARTITION_GUID, 34, 35, &long).is_ok());
    let over = "\u{1F600}".repeat(NAME_UNITS / 2 + 1);
    assert_eq!(
        Entry::new(ESP_TYPE_GUID, PARTITION_GUID, 34, 35, &over),
        Err(Error::Name)
    );
}

#[test]
fn a_partition_that_ends_before_it_begins_is_refused() {
    assert_eq!(
        Entry::new(ESP_TYPE_GUID, PARTITION_GUID, 4096, 4095, "EFI"),
        Err(Error::Partition(4096))
    );
    assert!(Entry::new(ESP_TYPE_GUID, PARTITION_GUID, 4096, 4096, "EFI").is_ok());
}

#[test]
fn an_entry_of_no_type_is_one_nothing_uses() {
    let unused = Entry::parse(&[0u8; ENTRY_LEN]);
    assert!(!unused.is_used());
    assert_eq!(unused.type_guid, UNUSED_TYPE_GUID);
    assert!(esp(2048, 4095).is_used());
}

#[test]
fn bytes_an_entry_does_not_reach_read_as_zero_and_take_no_field() {
    assert!(!Entry::parse(&[]).is_used());
    assert_eq!(Entry::parse(&[0xFFu8; 8]).first_lba, 0);
    // A field is written whole or not at all, so a slice too short for
    // the first one takes nothing rather than half of it.
    let mut short = [0u8; 8];
    esp(2048, 4095).write(&mut short);
    assert_eq!(short, [0u8; 8]);
    // One long enough for the type and no more takes the type.
    let mut head = [0u8; 16];
    esp(2048, 4095).write(&mut head);
    assert_eq!(head, ESP_TYPE_GUID);
}

#[test]
fn the_blocks_of_a_partition_count_both_ends() {
    assert_eq!(esp(2048, 2048).blocks(), 1);
    assert_eq!(esp(2048, 4095).blocks(), 2048);
}
