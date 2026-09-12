// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::msix`.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use crate::capability::ID_VENDOR;
use crate::error::PciError;
use crate::msix::{
    ENTRY_LEN, Entry, Location, read, read_entry, set_enabled, set_function_mask, write_entry,
};
use crate::tests::build::{Builder, one};

#[test]
fn the_table_size_is_the_encoded_field_plus_one() {
    let mut builder = Builder::new(0x1AF4, 0x1041);
    builder.capabilities(0x40);
    builder.msix(0x40, 0x00, 4, (1, 0x0000), (1, 0x0800));
    let space = one(&builder);
    let address = space.address(0, 0).unwrap();
    let capability = read(&space, address, 0x40).expect("the capability is there");
    assert_eq!(capability.vectors, 4);
    assert!(!capability.enabled);
    assert!(!capability.function_masked);
}

#[test]
fn the_table_and_the_pending_bits_may_name_different_registers() {
    let mut builder = Builder::new(0x1AF4, 0x1041);
    builder.capabilities(0x40);
    builder.msix(0x40, 0x00, 8, (1, 0x0000), (3, 0x2000));
    let space = one(&builder);
    let address = space.address(0, 0).unwrap();
    let capability = read(&space, address, 0x40).unwrap();
    assert_eq!(
        capability.table,
        Location {
            bar: 1,
            offset: 0x0000,
        }
    );
    assert_eq!(
        capability.pending,
        Location {
            bar: 3,
            offset: 0x2000,
        }
    );
}

#[test]
fn the_enable_and_the_function_mask_are_written_where_the_specification_puts_them() {
    let mut builder = Builder::new(0x1AF4, 0x1041);
    builder.capabilities(0x40);
    builder.msix(0x40, 0x00, 4, (1, 0), (1, 0x800));
    let mut space = one(&builder);
    let address = space.address(0, 0).unwrap();
    let capability = read(&space, address, 0x40).unwrap();

    set_enabled(&mut space, address, &capability, true).unwrap();
    assert!(read(&space, address, 0x40).unwrap().enabled);
    assert_eq!(
        space.peek(0, 0, 0x40).unwrap() >> 16 & 0x8000,
        0x8000,
        "bit 15 of the message control word"
    );

    set_function_mask(&mut space, address, &capability, true).unwrap();
    let after = read(&space, address, 0x40).unwrap();
    assert!(after.function_masked);
    assert!(after.enabled, "the other bit stayed as it was");
    assert_eq!(after.vectors, 4, "the table size stayed as it was");

    set_enabled(&mut space, address, &capability, false).unwrap();
    set_function_mask(&mut space, address, &capability, false).unwrap();
    let back = read(&space, address, 0x40).unwrap();
    assert!(!back.enabled);
    assert!(!back.function_masked);
}

#[test]
fn a_capability_of_another_identifier_is_not_read_as_msi_x() {
    let mut builder = Builder::new(0x1AF4, 0x1041);
    builder.capabilities(0x40);
    builder.capability(0x40, ID_VENDOR, 0x00, &[0; 8]);
    let space = one(&builder);
    let address = space.address(0, 0).unwrap();
    assert_eq!(
        read(&space, address, 0x40),
        Err(PciError::NotMsix(ID_VENDOR))
    );
}

#[test]
fn a_capability_that_would_leave_the_list_is_refused() {
    let builder = Builder::new(0x1AF4, 0x1041);
    let space = one(&builder);
    let address = space.address(0, 0).unwrap();
    assert_eq!(read(&space, address, 0xF8), Err(PciError::Offset(0xF8)));
}

#[test]
fn a_table_entry_carries_the_address_the_data_and_a_clear_mask_bit() {
    let mut bytes = [0xAAu8; ENTRY_LEN];
    let entry = Entry {
        address: 0x0000_0000_FEE0_0000,
        data: 0x0000_0031,
        masked: false,
    };
    write_entry(&mut bytes, &entry).expect("the slice is long enough");
    assert_eq!(&bytes[0..4], &0xFEE0_0000u32.to_le_bytes());
    assert_eq!(&bytes[4..8], &0u32.to_le_bytes());
    assert_eq!(&bytes[8..12], &0x31u32.to_le_bytes());
    assert_eq!(&bytes[12..16], &0u32.to_le_bytes());
    assert_eq!(read_entry(&bytes), Ok(entry));
}

#[test]
fn a_masked_entry_carries_the_bit_the_specification_gives_it() {
    let mut bytes = [0u8; ENTRY_LEN];
    let entry = Entry {
        address: 0xFEE0_1000_0000_0000,
        data: 0x42,
        masked: true,
    };
    write_entry(&mut bytes, &entry).unwrap();
    assert_eq!(&bytes[4..8], &0xFEE0_1000u32.to_le_bytes());
    assert_eq!(bytes[12] & 1, 1);
    assert_eq!(read_entry(&bytes), Ok(entry));
}

#[test]
fn a_slice_too_short_for_an_entry_is_refused_and_written_to_by_nothing() {
    let mut bytes = [0u8; ENTRY_LEN - 1];
    let entry = Entry {
        address: 0,
        data: 0,
        masked: false,
    };
    assert_eq!(
        write_entry(&mut bytes, &entry),
        Err(PciError::EntryTooShort(ENTRY_LEN - 1))
    );
    assert_eq!(
        read_entry(&bytes),
        Err(PciError::EntryTooShort(ENTRY_LEN - 1))
    );
}
