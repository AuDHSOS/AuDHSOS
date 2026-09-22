// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::ecam`.

use crate::Ecam;
use crate::ecam::BYTES_PER_BUS;

#[test]
fn absent_and_reversed_windows_cover_no_bytes() {
    for (base, first_bus, last_bus) in [(0, 0, 0), (0, 0, 255), (0xB000_0000, 8, 7)] {
        let window = Ecam {
            base,
            segment: 0,
            first_bus,
            last_bus,
        };
        assert!(window.is_empty());
        assert_eq!(window.buses(), 0);
        assert_eq!(window.len(), 0);
    }
}

#[test]
fn valid_windows_include_both_boundary_buses() {
    for (first_bus, last_bus, buses) in [(0, 0, 1u16), (7, 7, 1), (16, 31, 16), (0, 255, 256)] {
        let window = Ecam {
            base: 0xB000_0000,
            segment: 1,
            first_bus,
            last_bus,
        };
        assert!(!window.is_empty());
        assert_eq!(window.buses(), buses);
        assert_eq!(window.len(), u64::from(buses).saturating_mul(BYTES_PER_BUS));
    }
}
