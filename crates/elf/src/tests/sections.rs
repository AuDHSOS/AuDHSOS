// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::sections`.

#![allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
#![allow(clippy::as_conversions, clippy::cast_possible_truncation)]

use crate::error::ElfError;
use crate::sections::{SHDR_LEN, SHT_NOBITS, SHT_STRTAB, sections, string_at};

/// Number of bytes of the ELF file header.
const EHDR_LEN: usize = 64;

/// A file with three sections: two of content and the name table.
fn file() -> Vec<u8> {
    build(&[
        ("\u{0}.text", 1, vec![0x90; 8]),
        ("\u{0}.bss", SHT_NOBITS, vec![]),
    ])
}

/// An ELF file whose sections are the given ones plus the name table.
fn build(parts: &[(&str, u32, Vec<u8>)]) -> Vec<u8> {
    let mut names = vec![0u8];
    let mut offsets = Vec::new();
    for (name, _, _) in parts {
        offsets.push(names.len() as u32);
        names.extend_from_slice(name.trim_start_matches('\u{0}').as_bytes());
        names.push(0);
    }
    let shstrtab_name = names.len() as u32;
    names.extend_from_slice(b".shstrtab\0");

    let mut file = vec![0u8; EHDR_LEN];
    let mut content = Vec::new();
    for (_, _, bytes) in parts {
        content.push((file.len(), bytes.len()));
        file.extend_from_slice(bytes);
    }
    let names_at = file.len();
    file.extend_from_slice(&names);
    let table = file.len();

    file.extend_from_slice(&[0u8; SHDR_LEN]);
    for (index, (_, kind, bytes)) in parts.iter().enumerate() {
        let size = if *kind == SHT_NOBITS { 32 } else { bytes.len() };
        file.extend_from_slice(&entry(offsets[index], *kind, content[index].0, size));
    }
    file.extend_from_slice(&entry(shstrtab_name, SHT_STRTAB, names_at, names.len()));

    let count = parts.len() + 2;
    file[..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    file[4] = 2;
    file[5] = 1;
    file[0x28..0x30].copy_from_slice(&(table as u64).to_le_bytes());
    file[0x3A..0x3C].copy_from_slice(&(SHDR_LEN as u16).to_le_bytes());
    file[0x3C..0x3E].copy_from_slice(&(count as u16).to_le_bytes());
    file[0x3E..0x40].copy_from_slice(&((count - 1) as u16).to_le_bytes());
    file
}

/// One section header entry.
fn entry(name: u32, kind: u32, offset: usize, size: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(SHDR_LEN);
    bytes.extend_from_slice(&name.to_le_bytes());
    bytes.extend_from_slice(&kind.to_le_bytes());
    bytes.extend_from_slice(&2u64.to_le_bytes()); // flags
    bytes.extend_from_slice(&0x1000u64.to_le_bytes()); // addr
    bytes.extend_from_slice(&(offset as u64).to_le_bytes());
    bytes.extend_from_slice(&(size as u64).to_le_bytes());
    bytes.extend_from_slice(&3u32.to_le_bytes()); // link
    bytes.extend_from_slice(&4u32.to_le_bytes()); // info
    bytes.extend_from_slice(&8u64.to_le_bytes()); // align
    bytes.extend_from_slice(&0u64.to_le_bytes()); // entry size
    bytes
}

#[test]
fn every_section_is_listed_with_its_name_and_its_content() {
    let bytes = file();
    let table = sections(&bytes).unwrap();
    assert_eq!(table.len(), 4);
    assert!(!table.is_empty());
    assert_eq!(table.bytes().len(), bytes.len());
    let text = table.by_name(".text").expect(".text");
    assert_eq!(table.name_of(text), Some(".text"));
    assert_eq!(table.content(text), Some(&[0x90u8; 8][..]));
    assert_eq!(table.content_by_name(".text"), Some(&[0x90u8; 8][..]));
    assert_eq!(
        (text.flags, text.addr, text.link, text.info),
        (2, 0x1000, 3, 4)
    );
    assert_eq!(table.iter().count(), 4);
}

#[test]
fn a_section_without_content_hands_none_out() {
    let bytes = file();
    let table = sections(&bytes).unwrap();
    let bss = table.by_name(".bss").expect(".bss");
    assert_eq!(bss.size, 32, "the size says what it occupies in memory");
    assert_eq!(table.content(bss), None);
    assert_eq!(table.content_by_name(".bss"), None);
}

#[test]
fn a_name_that_is_in_no_section_finds_nothing() {
    let bytes = file();
    let table = sections(&bytes).unwrap();
    assert_eq!(table.by_name(".debug_line"), None);
    assert_eq!(table.content_by_name(".debug_line"), None);
    assert_eq!(table.get(4), None);
    assert_eq!(table.get(usize::MAX), None);
}

#[test]
fn strings_come_out_of_a_table_without_their_terminator() {
    let table = b"\0first\0second\0";
    assert_eq!(string_at(table, 0), Some(""));
    assert_eq!(string_at(table, 1), Some("first"));
    assert_eq!(string_at(table, 7), Some("second"));
    assert_eq!(string_at(table, 99), None, "an offset beyond the table");
    assert_eq!(string_at(b"open", 0), None, "no terminator");
    assert_eq!(string_at(&[0xFF, 0xFE, 0], 0), None, "not text");
}

#[test]
fn a_file_that_is_not_an_elf_object_is_refused() {
    assert_eq!(sections(&[]).unwrap_err(), ElfError::TooShort);
    assert_eq!(sections(&[0u8; 63]).unwrap_err(), ElfError::TooShort);
    assert_eq!(sections(&[0u8; 64]).unwrap_err(), ElfError::BadMagic);
    let mut bytes = file();
    bytes[4] = 1;
    assert_eq!(sections(&bytes).unwrap_err(), ElfError::NotClass64);
    let mut bytes = file();
    bytes[5] = 2;
    assert_eq!(sections(&bytes).unwrap_err(), ElfError::NotLittleEndian);
}

#[test]
fn a_file_with_no_section_table_has_no_sections() {
    let mut bytes = file();
    bytes[0x3C..0x3E].copy_from_slice(&0u16.to_le_bytes());
    let table = sections(&bytes).unwrap();
    assert!(table.is_empty());
    assert_eq!(table.len(), 0);
    assert_eq!(table.get(0), None);
    assert_eq!(table.by_name(".text"), None);
}

#[test]
fn an_entry_shorter_than_the_structure_is_refused() {
    let mut bytes = file();
    bytes[0x3A..0x3C].copy_from_slice(&32u16.to_le_bytes());
    assert_eq!(sections(&bytes).unwrap_err(), ElfError::SectionHeaderSize);
}

#[test]
fn a_table_that_reaches_beyond_the_file_is_refused() {
    let mut bytes = file();
    let len = bytes.len() as u64;
    bytes[0x28..0x30].copy_from_slice(&len.to_le_bytes());
    assert_eq!(sections(&bytes).unwrap_err(), ElfError::SectionHeaderTable);
    let mut bytes = file();
    bytes[0x28..0x30].copy_from_slice(&u64::MAX.to_le_bytes());
    assert_eq!(sections(&bytes).unwrap_err(), ElfError::SectionHeaderTable);
    let mut bytes = file();
    bytes[0x3C..0x3E].copy_from_slice(&u16::MAX.to_le_bytes());
    assert_eq!(sections(&bytes).unwrap_err(), ElfError::SectionHeaderTable);
}

#[test]
fn a_name_table_index_that_is_not_a_section_names_nothing() {
    let mut bytes = file();
    bytes[0x3E..0x40].copy_from_slice(&99u16.to_le_bytes());
    let table = sections(&bytes).unwrap();
    let first = table.get(1).expect("a section");
    assert_eq!(table.name_of(first), None);
    assert_eq!(table.by_name(".text"), None);
}

#[test]
fn a_section_whose_content_reaches_beyond_the_file_hands_none_out() {
    let bytes = file();
    let table = sections(&bytes).unwrap();
    let mut section = table.by_name(".text").expect(".text");
    section.size = u64::MAX;
    assert_eq!(table.content(section), None);
    section.size = 8;
    section.offset = u64::MAX;
    assert_eq!(table.content(section), None);
    assert!(!format!("{section:?}").is_empty());
}
