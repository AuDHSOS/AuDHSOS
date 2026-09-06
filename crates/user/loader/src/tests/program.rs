// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::program`.

use audhsos_abi::layout::{PAGE_SIZE, USER_SPACE_START};
use audhsos_elf::ElfError;

use crate::program::{ProgramError, STACK_PAGES, USER_CONSTRAINTS, plan};

/// One program header the tests build an image out of.
#[derive(Clone, Copy, Debug)]
struct Header {
    vaddr: u64,
    file_size: u64,
    mem_size: u64,
    write: bool,
    execute: bool,
}

/// An executable with these segments, with its entry at the first byte of
/// the first one.
fn image(headers: &[Header]) -> Vec<u8> {
    let entry = headers.first().map_or(0, |header| header.vaddr);
    image_with_entry(headers, entry)
}

/// The same, with the entry point chosen by the caller.
///
/// The content of a segment is placed at a file offset congruent to its
/// virtual address modulo the page size, which is what the ELF reader
/// insists on: a loader maps whole pages, so the bytes have to sit at the
/// same place inside a page in the file as they will in memory.
fn image_with_entry(headers: &[Header], entry: u64) -> Vec<u8> {
    const EHDR: usize = 64;
    const PHDR: usize = 56;
    let table = EHDR;
    let table_end = table.wrapping_add(PHDR.wrapping_mul(headers.len()));
    let mut cursor = table_end;
    let mut offsets = Vec::new();
    for header in headers {
        let page = cursor.next_multiple_of(usize::try_from(PAGE_SIZE).unwrap());
        let inside = usize::try_from(header.vaddr % PAGE_SIZE).unwrap();
        let offset = page.wrapping_add(inside);
        offsets.push(offset);
        cursor = offset.wrapping_add(usize::try_from(header.file_size).unwrap());
    }
    let mut bytes = vec![0u8; cursor.max(table_end)];

    put(&mut bytes, 0, &[0x7F, b'E', b'L', b'F', 2, 1, 1, 0]);
    put(&mut bytes, 16, &2u16.to_le_bytes()); // executable
    put(&mut bytes, 18, &62u16.to_le_bytes()); // x86-64
    put(&mut bytes, 20, &1u32.to_le_bytes()); // version
    put(&mut bytes, 24, &entry.to_le_bytes());
    put(&mut bytes, 32, &u64::try_from(table).unwrap().to_le_bytes()); // phoff
    put(&mut bytes, 52, &u16::try_from(EHDR).unwrap().to_le_bytes()); // ehsize
    put(&mut bytes, 54, &u16::try_from(PHDR).unwrap().to_le_bytes()); // phentsize
    put(
        &mut bytes,
        56,
        &u16::try_from(headers.len()).unwrap().to_le_bytes(),
    ); // phnum

    for (index, header) in headers.iter().enumerate() {
        let at = table.wrapping_add(index.wrapping_mul(PHDR));
        let offset = *offsets.get(index).unwrap();
        let mut flags = 4u32; // read
        if header.write {
            flags |= 2;
        }
        if header.execute {
            flags |= 1;
        }
        put(&mut bytes, at, &1u32.to_le_bytes()); // PT_LOAD
        put(&mut bytes, at.wrapping_add(4), &flags.to_le_bytes());
        put(
            &mut bytes,
            at.wrapping_add(8),
            &u64::try_from(offset).unwrap().to_le_bytes(),
        );
        put(&mut bytes, at.wrapping_add(16), &header.vaddr.to_le_bytes());
        put(&mut bytes, at.wrapping_add(24), &header.vaddr.to_le_bytes()); // paddr
        put(
            &mut bytes,
            at.wrapping_add(32),
            &header.file_size.to_le_bytes(),
        );
        put(
            &mut bytes,
            at.wrapping_add(40),
            &header.mem_size.to_le_bytes(),
        );
        put(&mut bytes, at.wrapping_add(48), &PAGE_SIZE.to_le_bytes()); // align
    }
    bytes
}

/// Copies `value` into `bytes` at `at`.
fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    let end = at.wrapping_add(value.len());
    bytes.get_mut(at..end).unwrap().copy_from_slice(value);
}

/// A read-and-execute segment at `vaddr`.
fn text(vaddr: u64, len: u64) -> Header {
    Header {
        vaddr,
        file_size: len,
        mem_size: len,
        write: false,
        execute: true,
    }
}

/// A read-and-write segment at `vaddr`, with `mem_size - file_size` bytes
/// of `.bss` behind what the file holds.
fn data(vaddr: u64, file_size: u64, mem_size: u64) -> Header {
    Header {
        vaddr,
        file_size,
        mem_size,
        write: true,
        execute: false,
    }
}

/// Where the programs of these tests are linked.
const BASE: u64 = 0x0100_0000;

#[test]
fn the_constraints_are_the_user_half_below_the_buffers() {
    assert_eq!(USER_CONSTRAINTS.lowest_vaddr, USER_SPACE_START);
    const { assert!(USER_CONSTRAINTS.highest_vaddr < audhsos_abi::layout::USER_SPACE_END) };
}

#[test]
fn a_program_of_one_segment_becomes_one_region() {
    let bytes = image(&[text(BASE, 100)]);
    let plan = plan(&bytes).unwrap();
    assert_eq!(plan.len(), 1);
    assert!(!plan.is_empty());
    let region = plan.regions().next().unwrap();
    assert_eq!(region.vaddr, BASE);
    assert_eq!(region.len, PAGE_SIZE);
    assert_eq!(region.pages(), 1);
    assert_eq!(region.end(), BASE + PAGE_SIZE);
    assert_eq!(region.file_size, 100);
    assert_eq!(region.leading, 0);
    assert!(region.execute);
    assert!(!region.write);
    assert_eq!(plan.entry, BASE);
}

#[test]
fn a_segment_that_does_not_start_on_a_page_keeps_what_is_in_front_of_it() {
    let bytes = image(&[text(BASE + 0x100, 100)]);
    let plan = plan(&bytes).unwrap();
    let region = plan.regions().next().unwrap();
    assert_eq!(region.vaddr, BASE);
    assert_eq!(region.leading, 0x100);
    assert_eq!(region.len, PAGE_SIZE);
}

#[test]
fn a_segment_with_bss_behind_its_bytes_covers_the_whole_of_it() {
    let bytes = image(&[
        text(BASE, 100),
        data(BASE + PAGE_SIZE, 8, PAGE_SIZE * 2 + 8),
    ]);
    let plan = plan(&bytes).unwrap();
    assert_eq!(plan.len(), 2);
    let region = plan.regions().nth(1).unwrap();
    assert_eq!(region.vaddr, BASE + PAGE_SIZE);
    assert_eq!(region.len, PAGE_SIZE * 3);
    assert_eq!(region.file_size, 8);
    assert!(region.write);
    assert!(!region.execute);
}

#[test]
fn the_stack_lies_below_the_program_with_a_page_between_them() {
    let bytes = image(&[text(BASE, 100)]);
    let plan = plan(&bytes).unwrap();
    assert_eq!(plan.stack_top, BASE - PAGE_SIZE);
    assert_eq!(plan.stack_base, plan.stack_top - STACK_PAGES * PAGE_SIZE);
    assert_eq!(plan.bytes(), PAGE_SIZE + STACK_PAGES * PAGE_SIZE);
}

#[test]
fn a_program_linked_too_low_for_a_stack_is_refused() {
    // Nothing at all below the program.
    let bytes = image(&[text(USER_SPACE_START, 100)]);
    assert_eq!(plan(&bytes).unwrap_err(), ProgramError::NoRoomForStack);

    // Room below it, but not enough: the stack would reach under the
    // lowest address this system maps.
    let bytes = image(&[text(USER_SPACE_START + 16 * PAGE_SIZE, 100)]);
    assert_eq!(plan(&bytes).unwrap_err(), ProgramError::NoRoomForStack);

    // One page more, and it fits exactly.
    let bytes = image(&[text(USER_SPACE_START + 17 * PAGE_SIZE, 100)]);
    let plan = plan(&bytes).unwrap();
    assert_eq!(plan.stack_base, USER_SPACE_START);
}

#[test]
fn an_entry_point_outside_every_executable_region_is_refused() {
    let bytes = image_with_entry(&[text(BASE, 100)], BASE + PAGE_SIZE);
    assert_eq!(
        plan(&bytes).unwrap_err(),
        ProgramError::Elf(ElfError::EntryNotExecutable)
    );
}

#[test]
fn an_entry_point_in_a_writable_region_is_refused() {
    let bytes = image_with_entry(
        &[text(BASE, 100), data(BASE + PAGE_SIZE, 8, 8)],
        BASE + PAGE_SIZE,
    );
    assert_eq!(
        plan(&bytes).unwrap_err(),
        ProgramError::Elf(ElfError::EntryNotExecutable)
    );
}

#[test]
fn two_segments_whose_pages_overlap_once_rounded_are_refused() {
    // Both segments live in the same page, which the ELF reader allows and
    // a plan cannot map twice.
    let bytes = image(&[text(BASE, 100), data(BASE + 200, 8, 8)]);
    assert_eq!(
        plan(&bytes).unwrap_err(),
        ProgramError::Overlap { vaddr: BASE }
    );
}

#[test]
fn a_segment_below_the_lowest_mappable_address_is_refused_by_the_elf_reader() {
    let bytes = image(&[text(0, 100)]);
    assert!(matches!(plan(&bytes).unwrap_err(), ProgramError::Elf(_)));
}

#[test]
fn what_the_elf_reader_refuses_is_passed_on() {
    let mut bytes = image(&[text(BASE, 100)]);
    put(&mut bytes, 0, &[0, 0, 0, 0]);
    assert_eq!(
        plan(&bytes).unwrap_err(),
        ProgramError::Elf(ElfError::BadMagic)
    );
}

#[test]
fn every_error_renders_a_message() {
    let cases = [
        ProgramError::Elf(ElfError::BadMagic),
        ProgramError::Overlap { vaddr: 1 },
        ProgramError::NoRoomForStack,
    ];
    for case in cases {
        assert!(!format!("{case}").is_empty(), "{case:?} has no message");
    }
}
