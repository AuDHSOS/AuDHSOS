// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::image`, covering the ELF items of the catalog 6.6.13.

#![allow(clippy::arithmetic_side_effects)]

use crate::error::ElfError;
use crate::image::{Constraints, EHDR_LEN, MAX_SEGMENTS, PHDR_LEN, Segment, parse};
use crate::strategies::{DEFAULT_VADDR, ElfBuilder, ProgramHeader, any_elf_bytes, any_elf_image};
use test_support::property::check;

/// What the loader allows for the kernel: the whole kernel half.
const KERNEL: Constraints = Constraints {
    lowest_vaddr: 0xFFFF_8000_0000_0000,
    highest_vaddr: u64::MAX,
};

/// What the userland loader allows: above the forbidden low 64 KiB and
/// below the canonical hole.
const USER: Constraints = Constraints {
    lowest_vaddr: 0x1_0000,
    highest_vaddr: 0x0000_7FFF_FFFF_FFFF,
};

fn parse_kernel(builder: &ElfBuilder) -> Result<usize, ElfError> {
    parse(&builder.build(), KERNEL).map(|image| image.segment_count())
}

#[test]
fn a_well_formed_image_reports_its_entry_and_segments() {
    let builder = ElfBuilder::new();
    let bytes = builder.build();
    let image = parse(&bytes, KERNEL).unwrap();
    assert_eq!(image.entry, DEFAULT_VADDR);
    assert_eq!(image.segment_count(), 1);
    assert_eq!(image.bytes.len(), bytes.len());
    let segment = image.segments().next().unwrap();
    assert_eq!(segment.vaddr, DEFAULT_VADDR);
    assert_eq!(segment.file_offset, 0x1000);
    assert_eq!(segment.file_size, 0x1000);
    assert_eq!(segment.mem_size, 0x1000);
    assert!(segment.execute && !segment.write);
    assert_eq!(image.segment_bytes(segment).len(), 0x1000);
    assert_eq!(image.highest_address(), Some(DEFAULT_VADDR + 0x1000 - 1));
}

#[test]
fn a_file_shorter_than_the_header_is_rejected() {
    let bytes = ElfBuilder::new().build();
    assert_eq!(
        parse(&bytes[..EHDR_LEN - 1], KERNEL).map(|_| ()),
        Err(ElfError::TooShort)
    );
    assert_eq!(parse(&[], KERNEL).map(|_| ()), Err(ElfError::TooShort));
}

#[test]
fn the_header_fields_are_checked_in_order() {
    let cases: [(ElfBuilder, ElfError); 7] = [
        (
            ElfBuilder {
                magic: [0x7F, b'E', b'L', b'G'],
                ..ElfBuilder::new()
            },
            ElfError::BadMagic,
        ),
        (
            ElfBuilder {
                class: 1,
                ..ElfBuilder::new()
            },
            ElfError::NotClass64,
        ),
        (
            ElfBuilder {
                data: 2,
                ..ElfBuilder::new()
            },
            ElfError::NotLittleEndian,
        ),
        (
            ElfBuilder {
                kind: 3,
                ..ElfBuilder::new()
            },
            ElfError::NotExecutable,
        ),
        (
            ElfBuilder {
                machine: 0x28,
                ..ElfBuilder::new()
            },
            ElfError::WrongMachine,
        ),
        (
            ElfBuilder {
                header_size: 63,
                ..ElfBuilder::new()
            },
            ElfError::HeaderSize,
        ),
        (
            ElfBuilder {
                entry_size: 55,
                ..ElfBuilder::new()
            },
            ElfError::ProgramHeaderSize,
        ),
    ];
    for (builder, expected) in cases {
        assert_eq!(parse_kernel(&builder), Err(expected));
    }
}

#[test]
fn a_program_header_table_beyond_the_file_is_rejected() {
    let beyond = ElfBuilder {
        table_offset: Some(0x1_0000),
        file_len: Some(0x2000),
        ..ElfBuilder::new()
    };
    assert_eq!(parse_kernel(&beyond), Err(ElfError::ProgramHeaderTable));
    let too_many = ElfBuilder {
        table_count: Some(1000),
        file_len: Some(0x2000),
        ..ElfBuilder::new()
    };
    assert_eq!(parse_kernel(&too_many), Err(ElfError::ProgramHeaderTable));
    let overflowing = ElfBuilder {
        table_offset: Some(u64::MAX),
        table_count: Some(2),
        file_len: Some(0x2000),
        ..ElfBuilder::new()
    };
    assert_eq!(parse_kernel(&overflowing), Err(ElfError::Overflow));
    let multiplying = ElfBuilder {
        entry_size: u16::MAX,
        table_count: Some(u16::MAX),
        file_len: Some(0x2000),
        ..ElfBuilder::new()
    };
    assert_eq!(
        parse_kernel(&multiplying),
        Err(ElfError::ProgramHeaderTable)
    );
}

#[test]
fn a_file_without_a_loadable_segment_is_rejected() {
    let none = ElfBuilder {
        table_count: Some(0),
        ..ElfBuilder::new()
    };
    assert_eq!(parse_kernel(&none), Err(ElfError::NoSegments));
    let only_other = ElfBuilder::new().with_headers(vec![ProgramHeader {
        kind: 4,
        ..ProgramHeader::code(0x1000, DEFAULT_VADDR, 0x1000)
    }]);
    assert_eq!(
        parse_kernel(&only_other),
        Err(ElfError::NoSegments),
        "entries that are not PT_LOAD are skipped"
    );
}

#[test]
fn a_segment_with_less_memory_than_file_content_is_rejected() {
    let builder = ElfBuilder::new().with_headers(vec![ProgramHeader {
        mem_size: 0x800,
        ..ProgramHeader::code(0x1000, DEFAULT_VADDR, 0x1000)
    }]);
    assert_eq!(
        parse_kernel(&builder),
        Err(ElfError::SegmentMemorySize { index: 0 })
    );
}

#[test]
fn a_segment_reaching_beyond_the_file_is_rejected() {
    let builder = ElfBuilder {
        file_len: Some(0x1800),
        ..ElfBuilder::new()
    };
    assert_eq!(
        parse_kernel(&builder),
        Err(ElfError::SegmentFileRange { index: 0 })
    );
    let overflowing = ElfBuilder {
        file_len: Some(0x2000),
        ..ElfBuilder::new()
    }
    .with_headers(vec![ProgramHeader {
        offset: u64::MAX,
        file_size: 2,
        mem_size: 2,
        align: 0,
        ..ProgramHeader::code(0, DEFAULT_VADDR, 0)
    }]);
    assert_eq!(parse_kernel(&overflowing), Err(ElfError::Overflow));
}

#[test]
fn a_segment_that_is_both_writable_and_executable_is_rejected() {
    let builder = ElfBuilder::new().with_headers(vec![ProgramHeader {
        flags: 7,
        ..ProgramHeader::code(0x1000, DEFAULT_VADDR, 0x1000)
    }]);
    assert_eq!(
        parse_kernel(&builder),
        Err(ElfError::WritableAndExecutable { index: 0 })
    );
}

#[test]
fn an_alignment_that_is_not_a_power_of_two_or_disagrees_is_rejected() {
    let not_a_power = ElfBuilder::new().with_headers(vec![ProgramHeader {
        align: 3000,
        ..ProgramHeader::code(0x1000, DEFAULT_VADDR, 0x1000)
    }]);
    assert_eq!(
        parse_kernel(&not_a_power),
        Err(ElfError::SegmentAlignment { index: 0 })
    );
    let disagreeing = ElfBuilder::new().with_headers(vec![ProgramHeader::code(
        0x1000,
        DEFAULT_VADDR + 0x10,
        0x1000,
    )]);
    assert_eq!(
        parse_kernel(&disagreeing),
        Err(ElfError::SegmentAlignment { index: 0 })
    );
    let unaligned_but_consistent = ElfBuilder {
        entry: DEFAULT_VADDR + 0x10,
        ..ElfBuilder::new()
    }
    .with_headers(vec![ProgramHeader {
        offset: 0x1010,
        vaddr: DEFAULT_VADDR + 0x10,
        ..ProgramHeader::code(0x1000, DEFAULT_VADDR, 0x1000)
    }]);
    assert_eq!(parse_kernel(&unaligned_but_consistent), Ok(1));
    let no_alignment = ElfBuilder {
        entry: DEFAULT_VADDR + 1,
        ..ElfBuilder::new()
    }
    .with_headers(vec![ProgramHeader {
        align: 0,
        ..ProgramHeader::code(0x1000, DEFAULT_VADDR + 1, 0x1000)
    }]);
    assert_eq!(
        parse(&no_alignment.build(), KERNEL).map(|image| image.entry),
        Ok(DEFAULT_VADDR + 1),
        "an alignment of zero switches the check off"
    );
}

#[test]
fn a_segment_outside_the_bounds_is_rejected() {
    let builder = ElfBuilder::new();
    assert_eq!(
        parse(&builder.build(), USER).map(|image| image.segment_count()),
        Err(ElfError::SegmentOutsideBounds { index: 0 }),
        "a kernel image is not a user image"
    );
    let low = ElfBuilder {
        entry: 0x1_0000,
        ..ElfBuilder::new()
    }
    .with_headers(vec![ProgramHeader::code(0x1000, 0x1_0000, 0x1000)]);
    assert_eq!(
        parse(&low.build(), USER).map(|image| image.segment_count()),
        Ok(1)
    );
    let too_low = ElfBuilder {
        entry: 0xF000,
        ..ElfBuilder::new()
    }
    .with_headers(vec![ProgramHeader::code(0x1000, 0xF000, 0x1000)]);
    assert_eq!(
        parse(&too_low.build(), USER).map(|image| image.segment_count()),
        Err(ElfError::SegmentOutsideBounds { index: 0 }),
        "the forbidden low 64 KiB"
    );
    let over_the_top = ElfBuilder::new().with_headers(vec![ProgramHeader {
        mem_size: 0x2000,
        vaddr: u64::MAX - 0x1000 + 1,
        ..ProgramHeader::code(0x1000, DEFAULT_VADDR, 0x1000)
    }]);
    assert_eq!(
        parse_kernel(&over_the_top),
        Err(ElfError::SegmentOutsideBounds { index: 0 })
    );
}

#[test]
fn segments_that_share_memory_are_rejected() {
    let overlapping = ElfBuilder::new().with_headers(vec![
        ProgramHeader::code(0x1000, DEFAULT_VADDR, 0x1000),
        ProgramHeader::data(0x2800, DEFAULT_VADDR + 0x800, 0x1000, 0x1000),
    ]);
    assert_eq!(parse_kernel(&overlapping), Err(ElfError::SegmentsOverlap));
    let touching = ElfBuilder::new().with_headers(vec![
        ProgramHeader::code(0x1000, DEFAULT_VADDR, 0x1000),
        ProgramHeader::data(0x2000, DEFAULT_VADDR + 0x1000, 0x1000, 0x1000),
    ]);
    assert_eq!(
        parse_kernel(&touching),
        Ok(2),
        "touching is not overlapping"
    );
    let empty = ElfBuilder::new().with_headers(vec![
        ProgramHeader::code(0x1000, DEFAULT_VADDR, 0x1000),
        ProgramHeader::data(0x2800, DEFAULT_VADDR + 0x800, 0, 0),
    ]);
    assert_eq!(
        parse_kernel(&empty),
        Ok(2),
        "an empty segment shares nothing"
    );
}

#[test]
fn segments_come_back_sorted_by_virtual_address() {
    let builder = ElfBuilder::new().with_headers(vec![
        ProgramHeader::data(0x3000, DEFAULT_VADDR + 0x2000, 0x1000, 0x1000),
        ProgramHeader::code(0x1000, DEFAULT_VADDR, 0x1000),
        ProgramHeader::data(0x2000, DEFAULT_VADDR + 0x1000, 0x1000, 0x1000),
    ]);
    let bytes = builder.build();
    let image = parse(&bytes, KERNEL).unwrap();
    let addresses: Vec<u64> = image.segments().map(|segment| segment.vaddr).collect();
    assert_eq!(
        addresses,
        vec![
            DEFAULT_VADDR,
            DEFAULT_VADDR + 0x1000,
            DEFAULT_VADDR + 0x2000
        ]
    );
    assert_eq!(image.highest_address(), Some(DEFAULT_VADDR + 0x3000 - 1));
}

#[test]
fn more_loadable_segments_than_the_capacity_are_rejected() {
    let headers: Vec<ProgramHeader> = (0..=MAX_SEGMENTS)
        .map(|index| {
            let offset = 0x1000 + u64::try_from(index).unwrap() * 0x1000;
            ProgramHeader::data(offset, DEFAULT_VADDR + offset, 0x1000, 0x1000)
        })
        .collect();
    let mut builder = ElfBuilder::new().with_headers(headers);
    builder.entry = DEFAULT_VADDR + 0x1000;
    assert_eq!(parse_kernel(&builder), Err(ElfError::TooManySegments));

    let at_limit: Vec<ProgramHeader> = (0..MAX_SEGMENTS)
        .map(|index| {
            let offset = 0x1000 + u64::try_from(index).unwrap() * 0x1000;
            ProgramHeader::code(offset, DEFAULT_VADDR + offset, 0x1000)
        })
        .collect();
    let mut fitting = ElfBuilder::new().with_headers(at_limit);
    fitting.entry = DEFAULT_VADDR + 0x1000;
    assert_eq!(parse_kernel(&fitting), Ok(MAX_SEGMENTS));
}

#[test]
fn an_entry_point_outside_every_executable_segment_is_rejected() {
    let outside = ElfBuilder {
        entry: DEFAULT_VADDR + 0x4000,
        ..ElfBuilder::new()
    };
    assert_eq!(parse_kernel(&outside), Err(ElfError::EntryNotExecutable));
    let in_data = ElfBuilder {
        entry: DEFAULT_VADDR + 0x1000,
        ..ElfBuilder::new()
    }
    .with_headers(vec![
        ProgramHeader::code(0x1000, DEFAULT_VADDR, 0x1000),
        ProgramHeader::data(0x2000, DEFAULT_VADDR + 0x1000, 0x1000, 0x1000),
    ]);
    assert_eq!(
        parse_kernel(&in_data),
        Err(ElfError::EntryNotExecutable),
        "a writable segment does not carry the entry point"
    );
    let at_the_last_byte = ElfBuilder {
        entry: DEFAULT_VADDR + 0xFFF,
        ..ElfBuilder::new()
    };
    assert_eq!(parse_kernel(&at_the_last_byte), Ok(1));
}

#[test]
fn segment_helpers_report_ends_containment_and_overlap() {
    let segment = Segment {
        file_offset: 0,
        file_size: 0x10,
        vaddr: 0x1000,
        mem_size: 0x10,
        align: 0x1000,
        write: false,
        execute: true,
    };
    assert_eq!(segment.end(), Some(0x1010));
    assert!(segment.contains(0x1000) && segment.contains(0x100F));
    assert!(!segment.contains(0x1010) && !segment.contains(0xFFF));
    let at_the_top = Segment {
        vaddr: u64::MAX,
        mem_size: 2,
        ..segment
    };
    assert_eq!(at_the_top.end(), None);
    assert!(!at_the_top.contains(u64::MAX));
    assert!(!at_the_top.overlaps(segment) && !segment.overlaps(at_the_top));
}

#[test]
fn the_constraints_accept_only_what_lies_inside_them() {
    assert!(USER.allows(0x1_0000, 0x1000));
    assert!(!USER.allows(0xFFFF, 1));
    assert!(USER.allows(USER.highest_vaddr, 1));
    assert!(!USER.allows(USER.highest_vaddr, 2));
    assert!(!USER.allows(USER.highest_vaddr + 1, 0));
    assert!(
        USER.allows(0x1_0000, 0),
        "an empty segment only needs a start"
    );
    assert!(KERNEL.allows(u64::MAX, 1));
    assert!(!KERNEL.allows(u64::MAX, 2));
}

#[test]
fn segment_bytes_of_a_foreign_segment_are_empty() {
    let bytes = ElfBuilder::new().build();
    let image = parse(&bytes, KERNEL).unwrap();
    let foreign = Segment {
        file_offset: u64::MAX,
        file_size: 1,
        vaddr: 0,
        mem_size: 1,
        align: 0,
        write: false,
        execute: false,
    };
    assert!(image.segment_bytes(foreign).is_empty());
    let beyond = Segment {
        file_offset: 0,
        file_size: u64::MAX,
        ..foreign
    };
    assert!(image.segment_bytes(beyond).is_empty());
}

#[test]
fn every_error_renders_a_message() {
    let errors = [
        ElfError::TooShort,
        ElfError::BadMagic,
        ElfError::NotClass64,
        ElfError::NotLittleEndian,
        ElfError::NotExecutable,
        ElfError::WrongMachine,
        ElfError::HeaderSize,
        ElfError::ProgramHeaderSize,
        ElfError::ProgramHeaderTable,
        ElfError::NoSegments,
        ElfError::TooManySegments,
        ElfError::SegmentFileRange { index: 0 },
        ElfError::SegmentMemorySize { index: 1 },
        ElfError::SegmentAlignment { index: 2 },
        ElfError::SegmentOutsideBounds { index: 3 },
        ElfError::SegmentsOverlap,
        ElfError::WritableAndExecutable { index: 4 },
        ElfError::EntryNotExecutable,
        ElfError::Overflow,
    ];
    for error in errors {
        assert!(!format!("{error}").is_empty());
    }
    assert_eq!(PHDR_LEN, 56);
}

#[test]
fn property_generated_images_parse_and_keep_their_invariants() {
    check("elf_valid_images", &any_elf_image(), |bytes| {
        let image = parse(bytes, KERNEL).map_err(|error| format!("rejected: {error}"))?;
        let mut previous: Option<Segment> = None;
        for segment in image.segments() {
            if segment.write && segment.execute {
                return Err("writable and executable".to_owned());
            }
            if let Some(previous) = previous
                && previous.vaddr > segment.vaddr
            {
                return Err("not sorted".to_owned());
            }
            previous = Some(segment);
        }
        Ok(())
    });
}

#[test]
fn property_parsing_near_valid_bytes_never_panics() {
    check("elf_mutated", &any_elf_bytes(), |bytes| {
        let _ = parse(bytes, KERNEL);
        let _ = parse(bytes, USER);
        Ok(())
    });
}
