// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every error says what went wrong in one line.

use crate::error::WireError;

#[test]
fn every_variant_writes_a_sentence_of_its_own() {
    let messages = [
        (
            WireError::OutOfBounds {
                needed: 4,
                available: 1,
            },
            "4 bytes were needed and 1 are left",
        ),
        (
            WireError::Address,
            "the text is not an address in canonical form",
        ),
        (
            WireError::PrefixLength(33),
            "an IPv4 prefix is at most 32 bits, not 33",
        ),
        (
            WireError::Length(70000),
            "70000 bytes do not fit in the length field",
        ),
        (
            WireError::MixedFamilies,
            "the two addresses are of different families",
        ),
    ];
    for (error, expected) in messages {
        assert_eq!(error.to_string(), expected, "{error:?}");
    }
}

#[test]
fn the_errors_compare_and_can_be_copied() {
    let error = WireError::PrefixLength(33);
    let copy = error;
    assert_eq!(error, copy);
    assert_ne!(error, WireError::PrefixLength(34));
    assert_ne!(WireError::Address, WireError::Length(0));
    assert!(format!("{error:?}").contains("PrefixLength"));
}
