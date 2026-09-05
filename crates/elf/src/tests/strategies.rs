// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::strategies`.

#![allow(clippy::arithmetic_side_effects)]

use crate::image::{EHDR_LEN, PHDR_LEN};
use crate::strategies::{DEFAULT_VADDR, ElfBuilder, ProgramHeader, any_elf_bytes, any_elf_image};
use test_support::property::check;

#[test]
fn the_builder_writes_the_fields_at_the_offsets_the_specification_names() {
    let bytes = ElfBuilder::new().build();
    assert_eq!(&bytes[..4], &[0x7F, b'E', b'L', b'F']);
    assert_eq!(bytes[4], 2, "EI_CLASS");
    assert_eq!(bytes[5], 1, "EI_DATA");
    assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 2, "e_type");
    assert_eq!(
        u16::from_le_bytes([bytes[18], bytes[19]]),
        0x3E,
        "e_machine"
    );
    assert_eq!(
        u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
        DEFAULT_VADDR,
        "e_entry"
    );
    assert_eq!(
        u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
        u64::try_from(EHDR_LEN).unwrap(),
        "e_phoff"
    );
    assert_eq!(u16::from_le_bytes([bytes[52], bytes[53]]), 64, "e_ehsize");
    assert_eq!(
        u16::from_le_bytes([bytes[54], bytes[55]]),
        56,
        "e_phentsize"
    );
    assert_eq!(u16::from_le_bytes([bytes[56], bytes[57]]), 1, "e_phnum");
    assert_eq!(bytes.len(), 0x2000);
}

#[test]
fn the_builder_writes_every_program_header_field() {
    let header = ProgramHeader::data(0x2000, DEFAULT_VADDR + 0x2000, 0x800, 0x1000);
    let bytes = ElfBuilder::new().with_headers(vec![header]).build();
    let table = &bytes[EHDR_LEN..EHDR_LEN + PHDR_LEN];
    assert_eq!(u32::from_le_bytes(table[0..4].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(table[4..8].try_into().unwrap()), 6);
    assert_eq!(u64::from_le_bytes(table[8..16].try_into().unwrap()), 0x2000);
    assert_eq!(
        u64::from_le_bytes(table[16..24].try_into().unwrap()),
        DEFAULT_VADDR + 0x2000
    );
    assert_eq!(u64::from_le_bytes(table[32..40].try_into().unwrap()), 0x800);
    assert_eq!(
        u64::from_le_bytes(table[40..48].try_into().unwrap()),
        0x1000
    );
    assert_eq!(
        u64::from_le_bytes(table[48..56].try_into().unwrap()),
        0x1000
    );
    assert_eq!(
        ElfBuilder::new().offset_of_table(),
        u64::try_from(EHDR_LEN).unwrap()
    );
}

#[test]
fn a_field_written_beyond_the_file_is_dropped_instead_of_panicking() {
    let short = ElfBuilder {
        file_len: Some(8),
        ..ElfBuilder::new()
    };
    assert_eq!(short.build().len(), 8);
    let empty = ElfBuilder {
        file_len: Some(0),
        headers: Vec::new(),
        ..ElfBuilder::new()
    };
    assert!(empty.build().is_empty());
}

#[test]
fn generated_images_and_mutations_stay_in_their_shapes() {
    check("elf_images_length", &any_elf_image(), |bytes| {
        if bytes.len() < EHDR_LEN {
            return Err(format!("length {}", bytes.len()));
        }
        Ok(())
    });
    check("elf_mutations_length", &any_elf_bytes(), |bytes| {
        if bytes.len() > 0x8000 {
            return Err(format!("length {}", bytes.len()));
        }
        Ok(())
    });
}
