// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::registers`.

use crate::registers::{Registers, Structure, Width};

use super::support::device;

#[test]
fn a_width_moves_the_bytes_it_names() {
    assert_eq!(Width::B8.bytes(), 1);
    assert_eq!(Width::B16.bytes(), 2);
    assert_eq!(Width::B32.bytes(), 4);
    assert_eq!(Width::B64.bytes(), 8);
}

#[test]
fn a_width_keeps_only_the_bits_its_register_holds() {
    assert_eq!(Width::B8.mask(u64::MAX), 0xFF);
    assert_eq!(Width::B16.mask(u64::MAX), 0xFFFF);
    assert_eq!(Width::B32.mask(u64::MAX), 0xFFFF_FFFF);
    assert_eq!(Width::B64.mask(u64::MAX), u64::MAX);
    assert_eq!(Width::B32.mask(7), 7);
}

#[test]
fn what_is_written_to_a_structure_is_what_is_read_back() {
    let mut device = device();
    for structure in [Structure::Common, Structure::Notify, Structure::Device] {
        device.write(structure, 8, Width::B32, 0xDEAD_BEEF);
        assert_eq!(
            device.read(structure, 8, Width::B32),
            0xDEAD_BEEF,
            "{structure:?}"
        );
    }
}

#[test]
fn the_interrupt_status_clears_as_it_is_read() {
    let device = device();
    device.set_interrupt_status(0b11);
    assert_eq!(device.read(Structure::Isr, 0, Width::B8), 0b11);
    assert_eq!(device.read(Structure::Isr, 0, Width::B8), 0);
}
