// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::sdt`.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use kernel_types::PhysAddr;
use kernel_types::phys::MAX_PHYS_ADDR;

use crate::error::AcpiError;
use crate::sdt::{RootTable, SDT_HEADER_LEN, SdtHeader, announced_length};
use crate::tests::build::{fix_table, rsdt, table, xsdt};

#[test]
fn a_header_reports_its_signature_its_length_and_its_revision() {
    let bytes = table(*b"FACP", 6, &[1, 2, 3, 4]);
    let header = SdtHeader::parse(&bytes).expect("the table is well formed");
    assert_eq!(header.signature, *b"FACP");
    assert_eq!(header.length, bytes.len() as u32);
    assert_eq!(header.revision, 6);
    assert_eq!(header.body_len(), 4);
}

#[test]
fn bytes_shorter_than_a_header_are_refused() {
    let bytes = [0u8; SDT_HEADER_LEN - 1];
    assert_eq!(
        SdtHeader::parse(&bytes),
        Err(AcpiError::TooShort(SDT_HEADER_LEN - 1))
    );
}

#[test]
fn a_length_below_the_header_is_refused() {
    let mut bytes = table(*b"FACP", 1, &[]);
    bytes[4..8].copy_from_slice(&(SDT_HEADER_LEN as u32 - 1).to_le_bytes());
    assert_eq!(
        SdtHeader::parse(&bytes),
        Err(AcpiError::Length(SDT_HEADER_LEN as u32 - 1))
    );
}

#[test]
fn a_length_beyond_the_bytes_is_refused() {
    let mut bytes = table(*b"FACP", 1, &[]);
    let announced = bytes.len() as u32 + 1;
    bytes[4..8].copy_from_slice(&announced.to_le_bytes());
    assert_eq!(SdtHeader::parse(&bytes), Err(AcpiError::Length(announced)));
}

#[test]
fn a_wrong_checksum_is_refused() {
    let mut bytes = table(*b"FACP", 1, &[]);
    bytes[9] = bytes[9].wrapping_add(1);
    assert_eq!(SdtHeader::parse(&bytes), Err(AcpiError::Checksum));
}

#[test]
fn bytes_after_the_announced_length_are_not_part_of_the_table() {
    let mut bytes = table(*b"FACP", 1, &[7, 7]);
    bytes.push(0xFF);
    let header = SdtHeader::parse(&bytes).expect("the trailing byte is outside the table");
    assert_eq!(header.length as usize, bytes.len() - 1);
}

#[test]
fn a_header_of_the_wrong_signature_is_reported_with_the_one_it_carries() {
    let bytes = table(*b"FACP", 1, &[]);
    let header = SdtHeader::parse(&bytes).expect("the table is well formed");
    assert_eq!(header.require(*b"FACP"), Ok(header));
    assert_eq!(
        header.require(*b"APIC"),
        Err(AcpiError::Signature(*b"FACP"))
    );
}

#[test]
fn an_rsdt_names_four_byte_addresses() {
    let bytes = rsdt(&[0x1000, 0x2000, 0x3000]);
    let root = RootTable::parse(&bytes).expect("the table is well formed");
    assert_eq!(root.len(), 3);
    assert!(!root.is_empty());
    assert_eq!(root.header().signature, *b"RSDT");
    let addresses: Vec<PhysAddr> = root.addresses().collect();
    assert_eq!(
        addresses,
        vec![
            PhysAddr::new(0x1000).unwrap(),
            PhysAddr::new(0x2000).unwrap(),
            PhysAddr::new(0x3000).unwrap(),
        ]
    );
    assert_eq!(root.get(3), None);
}

#[test]
fn an_xsdt_names_eight_byte_addresses() {
    let bytes = xsdt(&[0x1_0000_0000, 0x2000]);
    let root = RootTable::parse(&bytes).expect("the table is well formed");
    assert_eq!(root.len(), 2);
    assert_eq!(root.get(0), Some(PhysAddr::new(0x1_0000_0000).unwrap()));
    assert_eq!(root.get(1), Some(PhysAddr::new(0x2000).unwrap()));
}

#[test]
fn a_root_table_without_entries_names_nothing() {
    let bytes = xsdt(&[]);
    let root = RootTable::parse(&bytes).expect("the table is well formed");
    assert!(root.is_empty());
    assert_eq!(root.addresses().count(), 0);
}

#[test]
fn a_trailing_partial_entry_is_not_an_address() {
    let mut bytes = xsdt(&[0x1000]);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    fix_table(&mut bytes);
    let root = RootTable::parse(&bytes).expect("the table is well formed");
    assert_eq!(root.len(), 1);
}

#[test]
fn an_address_beyond_the_physical_width_is_left_out() {
    let bytes = xsdt(&[MAX_PHYS_ADDR + 1, 0x2000]);
    let root = RootTable::parse(&bytes).expect("the table is well formed");
    assert_eq!(root.len(), 2);
    assert_eq!(root.get(0), None);
    assert_eq!(
        root.addresses().collect::<Vec<_>>(),
        vec![PhysAddr::new(0x2000).unwrap()]
    );
}

#[test]
fn a_table_that_is_neither_an_rsdt_nor_an_xsdt_is_refused() {
    let bytes = table(*b"FACP", 1, &[]);
    assert_eq!(
        RootTable::parse(&bytes).err(),
        Some(AcpiError::Signature(*b"FACP"))
    );
}

#[test]
fn the_announced_length_is_read_without_a_check_of_any_kind() {
    let bytes = table(*b"FACP", 1, &[1, 2, 3, 4]);
    let header = <[u8; SDT_HEADER_LEN]>::try_from(&bytes[..SDT_HEADER_LEN]).unwrap();
    assert_eq!(announced_length(&header), bytes.len() as u32);
    let mut broken = header;
    broken[4..8].copy_from_slice(&0xDEAD_BEEF_u32.to_le_bytes());
    assert_eq!(announced_length(&broken), 0xDEAD_BEEF);
}
