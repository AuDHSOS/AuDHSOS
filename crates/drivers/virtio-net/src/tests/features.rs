// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The feature table: what is wanted, what is refused, and that every bit
//! the driver can read has a name.

#![allow(clippy::arithmetic_side_effects)]

use virtio_queue::{F_VERSION_1, UNSUPPORTED};

use crate::features::{F_MAC, F_MQ, F_MRG_RXBUF, WANTED, name, refused};

#[test]
fn the_driver_wants_the_version_and_the_address_and_nothing_else() {
    assert_eq!(WANTED, F_VERSION_1 | F_MAC);
}

#[test]
fn the_driver_wants_nothing_the_queue_crate_refuses() {
    assert_eq!(WANTED & UNSUPPORTED, 0);
}

#[test]
fn what_is_offered_beside_the_wanted_bits_is_refused() {
    let offered = WANTED | F_MRG_RXBUF | F_MQ;
    assert_eq!(refused(offered), F_MRG_RXBUF | F_MQ);
}

#[test]
fn a_refusal_of_the_wanted_set_alone_is_empty() {
    assert_eq!(refused(WANTED), 0);
}

#[test]
fn every_bit_of_the_device_the_driver_reads_has_a_name() {
    // The bits of the device's own range, which is everything below the
    // transport's range of 24 to 40 and everything above it up to the
    // sixty-three the two windows reach.
    let defined: [u64; 35] = [
        0, 1, 2, 3, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 50, 51, 52,
        53, 54, 55, 56, 57, 59, 60, 61, 62, 63,
    ];
    for bit in defined {
        let value = 1u64 << bit;
        assert!(name(value).is_some(), "bit {bit} has no name");
    }
}

#[test]
fn a_bit_of_the_transport_has_no_name_of_this_device() {
    assert!(name(F_VERSION_1).is_none());
    assert!(name(1 << 24).is_none());
}

#[test]
fn no_two_bits_carry_the_same_name() {
    let mut seen = std::vec::Vec::new();
    for bit in 0..64u32 {
        if let Some(found) = name(1u64 << bit) {
            assert!(!seen.contains(&found), "{found} names two bits");
            seen.push(found);
        }
    }
    assert_eq!(seen.len(), 35);
}
