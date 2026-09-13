// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The widths a register access carries.

use crate::registers::{Width, low_u8, low_u16};

#[test]
fn a_width_names_the_bytes_it_moves() {
    assert_eq!(Width::B8.bytes(), 1);
    assert_eq!(Width::B16.bytes(), 2);
    assert_eq!(Width::B32.bytes(), 4);
    assert_eq!(Width::B64.bytes(), 8);
}

#[test]
fn a_width_masks_what_does_not_fit_it() {
    assert_eq!(Width::B8.mask(0x1234), 0x34);
    assert_eq!(Width::B16.mask(0x1_2345), 0x2345);
    assert_eq!(Width::B32.mask(0x1_2345_6789), 0x2345_6789);
    assert_eq!(Width::B64.mask(u64::MAX), u64::MAX);
}

#[test]
fn the_low_halves_are_the_registers_of_their_width() {
    assert_eq!(low_u8(0xFF12), 0x12);
    assert_eq!(low_u16(0xFFFF_1234), 0x1234);
}
