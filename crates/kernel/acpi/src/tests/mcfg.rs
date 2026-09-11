// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::mcfg`.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use kernel_types::PhysAddr;
use kernel_types::phys::MAX_PHYS_ADDR;

use crate::error::AcpiError;
use crate::mcfg::{
    ALLOCATION_LEN, BYTES_PER_BUS, Ecam, MAX_ECAM_ALLOCATIONS, MCFG_HEADER_LEN, parse,
};
use crate::tests::build::{allocation, fix_table, mcfg, table};

/// The base address QEMU's `q35` machine programs the window at.
const BASE: u64 = 0xB000_0000;

#[test]
fn a_table_of_one_allocation_is_read_in_full() {
    let bytes = mcfg(&[allocation(BASE, 0, 0, 255)]);
    let parsed = parse(&bytes).expect("the table is well formed");
    assert_eq!(parsed.count(), 1);
    assert_eq!(
        parsed.first(),
        Some(Ecam {
            base: PhysAddr::new(BASE).unwrap(),
            segment: 0,
            first_bus: 0,
            last_bus: 255,
        })
    );
}

#[test]
fn a_table_of_several_allocations_keeps_them_in_order() {
    let bytes = mcfg(&[
        allocation(BASE, 0, 0, 15),
        allocation(BASE + BYTES_PER_BUS * 16, 1, 16, 31),
    ]);
    let parsed = parse(&bytes).expect("the table is well formed");
    assert_eq!(parsed.count(), 2);
    assert_eq!(parsed.first().unwrap().segment, 0);
    assert_eq!(parsed.allocations[1].unwrap().segment, 1);
    assert_eq!(parsed.allocations[1].unwrap().first_bus, 16);
}

#[test]
fn the_window_covers_a_mebibyte_of_every_bus_it_names() {
    let window = Ecam {
        base: PhysAddr::new(BASE).unwrap(),
        segment: 0,
        first_bus: 0,
        last_bus: 255,
    };
    assert_eq!(window.buses(), 256);
    assert_eq!(window.len(), 256 * BYTES_PER_BUS);
    assert!(!window.is_empty());
}

#[test]
fn a_window_of_one_bus_covers_that_one_bus() {
    let window = Ecam {
        base: PhysAddr::new(BASE).unwrap(),
        segment: 0,
        first_bus: 7,
        last_bus: 7,
    };
    assert_eq!(window.buses(), 1);
    assert_eq!(window.len(), BYTES_PER_BUS);
}

#[test]
fn a_table_with_no_allocation_names_no_window() {
    let bytes = mcfg(&[]);
    let parsed = parse(&bytes).expect("a table may name no window");
    assert_eq!(parsed.count(), 0);
    assert_eq!(parsed.first(), None);
}

#[test]
fn a_table_of_another_signature_is_refused() {
    let mut body = vec![0u8; 8];
    body.extend_from_slice(&allocation(BASE, 0, 0, 255));
    let bytes = table(*b"APIC", 1, &body);
    assert_eq!(parse(&bytes), Err(AcpiError::Signature(*b"APIC")));
}

#[test]
fn a_length_below_the_header_is_refused_before_a_field_is_read() {
    let mut bytes = mcfg(&[allocation(BASE, 0, 0, 255)]);
    bytes[4..8].copy_from_slice(&8u32.to_le_bytes());
    bytes[9] = 0;
    let sum = bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    bytes[9] = 0u8.wrapping_sub(sum);
    assert_eq!(parse(&bytes), Err(AcpiError::Length(8)));
}

#[test]
fn a_length_beyond_the_bytes_is_refused_before_a_field_is_read() {
    let mut bytes = mcfg(&[allocation(BASE, 0, 0, 255)]);
    let announced = (bytes.len() + ALLOCATION_LEN) as u32;
    bytes[4..8].copy_from_slice(&announced.to_le_bytes());
    assert_eq!(parse(&bytes), Err(AcpiError::Length(announced)));
}

#[test]
fn a_bad_checksum_is_refused_before_a_field_is_read() {
    let mut bytes = mcfg(&[allocation(BASE, 0, 0, 255)]);
    bytes[9] = bytes[9].wrapping_add(1);
    assert_eq!(parse(&bytes), Err(AcpiError::Checksum));
}

#[test]
fn an_allocation_whose_last_bus_is_below_its_first_is_refused() {
    let bytes = mcfg(&[allocation(BASE, 0, 16, 15)]);
    assert_eq!(
        parse(&bytes),
        Err(AcpiError::BusRange {
            first_bus: 16,
            last_bus: 15,
        })
    );
}

#[test]
fn an_allocation_whose_base_is_not_page_aligned_is_refused() {
    let bytes = mcfg(&[allocation(BASE + 1, 0, 0, 255)]);
    assert_eq!(parse(&bytes), Err(AcpiError::Unaligned(BASE + 1)));
}

#[test]
fn an_allocation_beyond_the_physical_width_is_refused() {
    let base = (MAX_PHYS_ADDR + 1) & !0xFFF;
    let bytes = mcfg(&[allocation(base, 0, 0, 255)]);
    assert_eq!(parse(&bytes), Err(AcpiError::Address(base)));
}

#[test]
fn more_allocations_than_the_array_holds_are_refused_rather_than_truncated() {
    let mut allocations = Vec::new();
    for index in 0..=MAX_ECAM_ALLOCATIONS {
        allocations.push(allocation(
            BASE + (index as u64) * BYTES_PER_BUS,
            index as u16,
            0,
            0,
        ));
    }
    let bytes = mcfg(&allocations);
    assert_eq!(parse(&bytes), Err(AcpiError::TooManyAllocations));
}

#[test]
fn a_table_that_ends_inside_an_allocation_is_refused() {
    let mut bytes = mcfg(&[allocation(BASE, 0, 0, 255)]);
    bytes.truncate(MCFG_HEADER_LEN + ALLOCATION_LEN - 1);
    fix_table(&mut bytes);
    assert_eq!(parse(&bytes), Err(AcpiError::TooShort(MCFG_HEADER_LEN)));
}

#[test]
fn a_table_that_ends_before_the_reserved_bytes_is_refused_as_a_short_table() {
    let bytes = table(*b"MCFG", 1, &[0u8; 4]);
    let parsed = parse(&bytes).expect("a table shorter than one allocation names no window");
    assert_eq!(parsed.count(), 0);
}
