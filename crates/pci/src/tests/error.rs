// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::error`.

use crate::error::PciError;

/// Every variant, so that the report of one is never empty.
const EVERY: [PciError; 16] = [
    PciError::Device(32),
    PciError::Function(8),
    PciError::BusRange {
        first_bus: 16,
        last_bus: 15,
    },
    PciError::BusOutside {
        bus: 8,
        first_bus: 4,
        last_bus: 7,
    },
    PciError::Segment {
        wanted: 1,
        window: 0,
    },
    PciError::Offset(0x1000),
    PciError::Unaligned(0x35),
    PciError::Unreadable(0x10),
    PciError::HeaderType(1),
    PciError::BarIndex(6),
    PciError::BarTruncated(5),
    PciError::CapabilityPointer(0x3C),
    PciError::CapabilityLoop,
    PciError::CapabilityLength { id: 9, length: 16 },
    PciError::CapabilityBar(6),
    PciError::NotMsix(9),
];

#[test]
fn every_error_reports_itself() {
    for error in EVERY {
        assert!(!format!("{error}").is_empty(), "{error:?}");
    }
    assert!(!format!("{}", PciError::EntryTooShort(15)).is_empty());
}
