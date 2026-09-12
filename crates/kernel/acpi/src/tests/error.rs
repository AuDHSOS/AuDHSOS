// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::error`.

use crate::error::AcpiError;

/// Every variant, so that the report of one is never empty.
const EVERY: [AcpiError; 17] = [
    AcpiError::RootPointerSignature,
    AcpiError::RootPointerChecksum,
    AcpiError::ExtendedChecksum,
    AcpiError::Revision(3),
    AcpiError::RootPointerLength(7),
    AcpiError::TooShort(12),
    AcpiError::Length(9),
    AcpiError::Checksum,
    AcpiError::Signature(*b"APIC"),
    AcpiError::EntryLengthZero(44),
    AcpiError::EntryTruncated {
        kind: 1,
        length: 40,
    },
    AcpiError::EntryLength {
        kind: 2,
        length: 11,
    },
    AcpiError::TooManyIoApics,
    AcpiError::TooManyOverrides,
    AcpiError::TooManyAllocations,
    AcpiError::BusRange {
        first_bus: 16,
        last_bus: 15,
    },
    AcpiError::Unaligned(0xB000_0001),
];

#[test]
fn every_error_reports_itself() {
    for error in EVERY {
        assert!(!format!("{error}").is_empty(), "{error:?}");
    }
    assert!(!format!("{}", AcpiError::Address(1 << 60)).is_empty());
}

#[test]
fn a_signature_is_reported_as_text_with_a_dot_for_what_is_not_printable() {
    assert_eq!(
        format!("{}", AcpiError::Signature(*b"APIC")),
        "the table signature is APIC"
    );
    assert_eq!(
        format!("{}", AcpiError::Signature([0, 0x7F, b'A', b' '])),
        "the table signature is ..A."
    );
}
