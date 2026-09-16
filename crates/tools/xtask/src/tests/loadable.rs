// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::image::loadable`.

use crate::image::loadable::trim;

/// How many bytes of the one segment lie in the file.
const SEGMENT: u64 = 0x200;

/// Where the segment stands: the address the programs of this system are
/// linked at, which leaves the room below it a stack needs.
const BASE: u64 = crate::commands::USER_TEST_BASE;

/// A program of one readable and executable segment that covers the
/// headers, with `tail` bytes behind it that no loader reads.
fn program(tail: usize) -> Vec<u8> {
    let vaddr = BASE;
    let mut bytes = vec![0u8; 64];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2; // 64-bit
    bytes[5] = 1; // little-endian
    bytes[6] = 1; // version
    bytes[16..18].copy_from_slice(&2u16.to_le_bytes()); // executable
    bytes[18..20].copy_from_slice(&0x3Eu16.to_le_bytes()); // x86-64
    bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
    bytes[24..32].copy_from_slice(&vaddr.to_le_bytes()); // entry
    bytes[32..40].copy_from_slice(&64u64.to_le_bytes()); // program headers
    bytes[40..48].copy_from_slice(&0u64.to_le_bytes()); // no section headers
    bytes[52..54].copy_from_slice(&64u16.to_le_bytes()); // header size
    bytes[54..56].copy_from_slice(&56u16.to_le_bytes()); // one entry is 56
    bytes[56..58].copy_from_slice(&1u16.to_le_bytes()); // one entry

    let mut header = vec![0u8; 56];
    header[..4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
    header[4..8].copy_from_slice(&0b101u32.to_le_bytes()); // read and execute
    header[8..16].copy_from_slice(&0u64.to_le_bytes()); // from the first byte
    header[16..24].copy_from_slice(&vaddr.to_le_bytes());
    header[24..32].copy_from_slice(&vaddr.to_le_bytes());
    header[32..40].copy_from_slice(&SEGMENT.to_le_bytes());
    header[40..48].copy_from_slice(&SEGMENT.to_le_bytes());
    header[48..56].copy_from_slice(&0x1000u64.to_le_bytes()); // page-aligned
    bytes.extend_from_slice(&header);

    bytes.resize(usize::try_from(SEGMENT).unwrap(), 0);
    bytes.extend(std::iter::repeat_n(0xAB, tail));
    bytes
}

#[test]
fn what_no_loader_reads_is_cut_away() {
    let whole = program(4096);
    let kept = trim("test", &whole).unwrap();
    let segment = usize::try_from(SEGMENT).unwrap();
    assert_eq!(kept.len(), segment);
    assert_eq!(kept, whole[..segment]);
}

#[test]
fn a_program_of_nothing_but_what_is_read_is_kept_whole() {
    let whole = program(0);
    assert_eq!(trim("test", &whole).unwrap(), whole);
}

#[test]
fn what_was_kept_reads_back_as_the_same_program() {
    let whole = program(1024);
    let kept = trim("test", &whole).unwrap();
    let before = user_loader::plan(&whole).unwrap();
    let after = user_loader::plan(&kept).unwrap();
    assert_eq!(after.entry, before.entry);
    assert_eq!(
        after.regions().collect::<Vec<_>>(),
        before.regions().collect::<Vec<_>>()
    );
}

#[test]
fn a_file_that_is_no_program_of_this_system_is_refused() {
    let error = trim("test", b"not an elf at all").unwrap_err();
    assert!(
        error
            .to_string()
            .contains("is no program this system loads"),
        "{error}"
    );
}
