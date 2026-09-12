// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::capability`.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use crate::capability::{Capability, ID_MSIX, ID_VENDOR, find, walk};
use crate::doubles::RecordedConfigSpace;
use crate::error::PciError;
use crate::header::read;
use crate::tests::build::{Builder, one};

/// Walks the list of the one function of `space`.
fn capabilities(
    space: &RecordedConfigSpace,
) -> Result<[Option<Capability>; crate::capability::MAX_CAPABILITIES], PciError> {
    let address = space.address(0, 0).expect("the function exists");
    let header = read(space, address).unwrap().unwrap();
    walk(space, address, &header)
}

#[test]
fn a_function_without_a_list_has_no_capability() {
    let builder = Builder::new(0x8086, 0x1234);
    let space = one(&builder);
    let found = capabilities(&space).unwrap();
    assert_eq!(found.iter().flatten().count(), 0);
    assert_eq!(find(&found, ID_MSIX), None);
}

#[test]
fn a_list_of_one_capability_ends_at_it() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.capabilities(0x40);
    builder.capability(0x40, ID_MSIX, 0x00, &[0, 0]);
    let space = one(&builder);
    let found = capabilities(&space).unwrap();
    assert_eq!(
        found[0],
        Some(Capability {
            id: ID_MSIX,
            offset: 0x40,
        })
    );
    assert_eq!(found[1], None);
}

#[test]
fn a_list_of_several_capabilities_keeps_the_order_it_had() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.capabilities(0x60);
    builder.capability(0x60, ID_VENDOR, 0x50, &[0; 4]);
    builder.capability(0x50, ID_MSIX, 0x40, &[0; 4]);
    builder.capability(0x40, ID_VENDOR, 0x00, &[0; 4]);
    let space = one(&builder);
    let found = capabilities(&space).unwrap();
    let offsets: Vec<u16> = found.iter().flatten().map(|entry| entry.offset).collect();
    assert_eq!(offsets, vec![0x60, 0x50, 0x40]);
    assert_eq!(find(&found, ID_MSIX).unwrap().offset, 0x50);
    assert_eq!(
        find(&found, ID_VENDOR).unwrap().offset,
        0x60,
        "the first of a kind is the one the device prefers"
    );
}

#[test]
fn a_capability_that_points_at_itself_is_refused_by_the_bound() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.capabilities(0x40);
    builder.capability(0x40, ID_VENDOR, 0x40, &[0; 4]);
    let space = one(&builder);
    assert_eq!(capabilities(&space), Err(PciError::CapabilityLoop));
}

#[test]
fn a_chain_that_returns_to_an_earlier_entry_is_refused_by_the_bound() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.capabilities(0x40);
    builder.capability(0x40, ID_VENDOR, 0x50, &[0; 4]);
    builder.capability(0x50, ID_VENDOR, 0x40, &[0; 4]);
    let space = one(&builder);
    assert_eq!(capabilities(&space), Err(PciError::CapabilityLoop));
}

#[test]
fn a_pointer_into_the_header_is_refused() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.capabilities(0x3C);
    let space = one(&builder);
    assert_eq!(capabilities(&space), Err(PciError::CapabilityPointer(0x3C)));
}

#[test]
fn the_low_two_bits_of_a_pointer_are_not_part_of_the_offset() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.capabilities(0x43);
    builder.capability(0x40, ID_MSIX, 0x00, &[0; 4]);
    let space = one(&builder);
    let found = capabilities(&space).unwrap();
    assert_eq!(found[0].unwrap().offset, 0x40);
}

#[test]
fn a_list_that_fills_the_space_it_lies_in_is_walked_to_its_end() {
    let mut builder = Builder::new(0x8086, 0x1234);
    builder.capabilities(0x40);
    let mut offset = 0x40u8;
    while offset < 0xFC {
        builder.capability(offset, ID_VENDOR, offset + 4, &[0, 0]);
        offset += 4;
    }
    builder.capability(0xFC, ID_VENDOR, 0x00, &[0, 0]);
    let space = one(&builder);
    let found = capabilities(&space).unwrap();
    assert_eq!(found.iter().flatten().count(), 48);
}
