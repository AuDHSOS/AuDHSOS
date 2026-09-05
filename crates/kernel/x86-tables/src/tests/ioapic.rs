// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::ioapic`.

use crate::ioapic::{
    ACTIVE_LOW, ID, IOREGSEL, IOWIN, LEVEL_TRIGGERED, MASKED, REDIRECTION_BASE, VERSION,
    entry_active_low, entry_destination, entry_from_words, entry_level, entry_masked, entry_vector,
    entry_words, id_of, redirection_entry, redirection_index, version_line_count, with_mask,
};

#[test]
fn every_register_sits_where_the_specification_says() {
    assert_eq!([IOREGSEL, IOWIN], [0x00, 0x10]);
    assert_eq!([ID, VERSION, REDIRECTION_BASE], [0, 1, 0x10]);
}

#[test]
fn the_two_words_of_a_line_follow_the_base_in_pairs() {
    assert_eq!(redirection_index(0), (0x10, 0x11));
    assert_eq!(redirection_index(1), (0x12, 0x13));
    assert_eq!(redirection_index(23), (0x3E, 0x3F));
}

#[test]
fn an_edge_triggered_entry_asserted_high_carries_only_its_vector_and_destination() {
    let entry = redirection_entry(0x41, false, false, false, 0);
    assert_eq!(entry, 0x41);
    assert_eq!(entry_vector(entry), 0x41);
    assert!(!entry_masked(entry));
    assert!(!entry_active_low(entry));
    assert!(!entry_level(entry));
    assert_eq!(entry_destination(entry), 0);
}

#[test]
fn a_level_triggered_entry_asserted_low_carries_both_bits() {
    let entry = redirection_entry(0x44, true, true, true, 3);
    assert_eq!(entry_vector(entry), 0x44);
    assert!(entry_active_low(entry));
    assert!(entry_level(entry));
    assert!(entry_masked(entry));
    assert_eq!(entry_destination(entry), 3);
    assert_eq!(
        entry,
        0x44 | ACTIVE_LOW | LEVEL_TRIGGERED | MASKED | (3 << 56)
    );
}

#[test]
fn the_mask_bit_is_set_and_cleared_without_touching_the_rest() {
    let entry = redirection_entry(0x42, true, true, false, 1);
    let masked = with_mask(entry, true);
    assert!(entry_masked(masked));
    assert_eq!(with_mask(masked, false), entry);
    assert_eq!(with_mask(entry, false), entry);
}

#[test]
fn an_entry_survives_being_written_as_two_words_and_read_back() {
    let entry = redirection_entry(0x50, true, false, true, 0x0F);
    let (low, high) = entry_words(entry);
    assert_eq!(low, 0x50 | 0x2000 | 0x1_0000);
    assert_eq!(high, 0x0F00_0000);
    assert_eq!(entry_from_words(low, high), entry);
}

#[test]
fn the_version_register_reports_one_less_than_the_number_of_lines() {
    assert_eq!(version_line_count(0x0017_0011), 24);
    assert_eq!(version_line_count(0x0000_0011), 1);
}

#[test]
fn the_identifier_register_carries_the_identifier_in_four_bits() {
    assert_eq!(id_of(0x0F00_0000), 0x0F);
    assert_eq!(id_of(0x0000_0000), 0);
    assert_eq!(id_of(0xF000_0000), 0);
}
