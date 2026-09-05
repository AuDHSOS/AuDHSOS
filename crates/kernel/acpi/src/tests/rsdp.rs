// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::rsdp`.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use kernel_types::PhysAddr;
use kernel_types::phys::MAX_PHYS_ADDR;

use crate::error::AcpiError;
use crate::rsdp::{REVISION_V1, REVISION_V2, RSDP_LEN, parse_rsdp};
use crate::tests::build::{fix_rsdp, rsdp};

#[test]
fn a_revision_zero_pointer_names_the_rsdt_and_no_xsdt() {
    let bytes = rsdp(REVISION_V1, 0x000E_0000, 0);
    let parsed = parse_rsdp(&bytes).expect("the pointer is well formed");
    assert_eq!(parsed.revision, REVISION_V1);
    assert_eq!(parsed.rsdt, PhysAddr::new(0x000E_0000).unwrap());
    assert_eq!(parsed.xsdt, None);
    assert_eq!(parsed.root(), parsed.rsdt);
}

#[test]
fn a_revision_two_pointer_names_the_xsdt_and_the_kernel_reads_that_one() {
    let bytes = rsdp(REVISION_V2, 0x000E_0000, 0x000F_0000);
    let parsed = parse_rsdp(&bytes).expect("the pointer is well formed");
    assert_eq!(parsed.revision, REVISION_V2);
    assert_eq!(parsed.xsdt, Some(PhysAddr::new(0x000F_0000).unwrap()));
    assert_eq!(parsed.root(), PhysAddr::new(0x000F_0000).unwrap());
}

#[test]
fn a_pointer_with_another_signature_is_refused() {
    let mut bytes = rsdp(REVISION_V2, 1, 2);
    bytes[0] = b'X';
    fix_rsdp(&mut bytes);
    assert_eq!(parse_rsdp(&bytes), Err(AcpiError::RootPointerSignature));
}

#[test]
fn a_pointer_whose_first_checksum_is_wrong_is_refused() {
    let mut bytes = rsdp(REVISION_V2, 1, 2);
    bytes[8] = bytes[8].wrapping_add(1);
    assert_eq!(parse_rsdp(&bytes), Err(AcpiError::RootPointerChecksum));
}

#[test]
fn a_pointer_whose_extended_checksum_is_wrong_is_refused() {
    let mut bytes = rsdp(REVISION_V2, 1, 2);
    bytes[32] = bytes[32].wrapping_add(1);
    assert_eq!(parse_rsdp(&bytes), Err(AcpiError::ExtendedChecksum));
}

#[test]
fn a_revision_one_pointer_is_refused_because_the_specification_defines_none() {
    let bytes = rsdp(1, 1, 2);
    assert_eq!(parse_rsdp(&bytes), Err(AcpiError::Revision(1)));
}

#[test]
fn a_length_that_does_not_reach_the_extended_checksum_is_refused() {
    for length in [0u32, 32] {
        let mut bytes = rsdp(REVISION_V2, 1, 2);
        bytes[20..24].copy_from_slice(&length.to_le_bytes());
        fix_rsdp(&mut bytes);
        assert_eq!(
            parse_rsdp(&bytes),
            Err(AcpiError::RootPointerLength(length)),
            "a length of {length} bytes"
        );
    }
}

#[test]
fn a_length_beyond_the_structure_is_refused() {
    let mut bytes = rsdp(REVISION_V2, 1, 2);
    bytes[20..24].copy_from_slice(&(RSDP_LEN as u32 + 1).to_le_bytes());
    fix_rsdp(&mut bytes);
    assert_eq!(
        parse_rsdp(&bytes),
        Err(AcpiError::RootPointerLength(RSDP_LEN as u32 + 1))
    );
}

#[test]
fn a_length_of_thirty_three_bytes_is_checked_over_those_bytes() {
    let mut bytes = rsdp(REVISION_V2, 1, 2);
    bytes[20..24].copy_from_slice(&33u32.to_le_bytes());
    fix_rsdp(&mut bytes);
    assert!(parse_rsdp(&bytes).is_ok());
    bytes[35] = 0xFF;
    assert!(
        parse_rsdp(&bytes).is_ok(),
        "a byte the announced length leaves out changes nothing"
    );
}

#[test]
fn an_xsdt_address_beyond_the_physical_width_is_refused() {
    let raw = MAX_PHYS_ADDR + 1;
    let bytes = rsdp(REVISION_V2, 1, raw);
    assert_eq!(parse_rsdp(&bytes), Err(AcpiError::Address(raw)));
}
