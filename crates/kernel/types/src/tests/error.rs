// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::error`.

use crate::error::Error;

#[test]
fn every_variant_has_a_message() {
    let variants = [
        Error::NonCanonical(1),
        Error::ExceedsPhysicalLimit(2),
        Error::Unaligned {
            address: 3,
            alignment: 4,
        },
        Error::InvalidAlignment(5),
        Error::Overflow,
        Error::CrossesCanonicalHole,
    ];
    for variant in variants {
        assert!(!variant.to_string().is_empty());
    }
    assert!(
        Error::Unaligned {
            address: 0x1001,
            alignment: 4096
        }
        .to_string()
        .contains("0x1001")
    );
}
