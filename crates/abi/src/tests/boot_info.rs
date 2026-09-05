// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::boot_info`, covering the catalog items 6.6.10 for the
//! boot information structure.

#![allow(clippy::arithmetic_side_effects)]

use crate::boot_info::{
    BOOT_INFO_HEADER_LEN, BOOT_INFO_MAGIC, BOOT_INFO_PAGE_LEN, BOOT_INFO_VERSION, BOOT_REGION_LEN,
    BootInfoError, BootInfoHeader, BootInfoView, BootInfoWriter, BootRegion, BootRegionKind,
    FixedRange,
};
use crate::layout::{MAX_BOOT_REGIONS, PAGE_SIZE};
use crate::strategies::{any_boot_info_bytes, any_boot_region};
use test_support::generators::vec;
use test_support::property::check;

const MIB: u64 = 1024 * 1024;

/// A structure written field by field, so that tests can produce values the
/// writer never writes.
#[derive(Clone, Debug)]
struct Raw {
    magic: [u8; 8],
    version: u32,
    size: Option<u32>,
    region_count: Option<u32>,
    header: BootInfoHeader,
    regions: Vec<BootRegion>,
}

fn valid_header() -> BootInfoHeader {
    BootInfoHeader {
        magic: BOOT_INFO_MAGIC,
        version: BOOT_INFO_VERSION,
        size: 0,
        phys_window_base: 0xFFFF_8000_0000_0000,
        kernel_phys_start: MIB,
        kernel_phys_len: MIB,
        boot_image_phys_start: 2 * MIB,
        boot_image_phys_len: MIB,
        page_tables_phys_start: 3 * MIB,
        page_tables_phys_len: 64 * PAGE_SIZE,
        boot_stack_phys_start: 4 * MIB,
        boot_stack_phys_len: 16 * PAGE_SIZE,
        acpi_rsdp: 0,
        region_count: 0,
        reserved: 0,
    }
}

impl Default for Raw {
    fn default() -> Self {
        Raw {
            magic: BOOT_INFO_MAGIC,
            version: BOOT_INFO_VERSION,
            size: None,
            region_count: None,
            header: valid_header(),
            regions: vec![BootRegion::new(0, 64 * MIB, BootRegionKind::Usable)],
        }
    }
}

impl Raw {
    fn bytes(&self) -> Vec<u8> {
        let count = self
            .region_count
            .unwrap_or_else(|| u32::try_from(self.regions.len()).unwrap());
        let size = self.size.unwrap_or_else(|| {
            u32::try_from(BOOT_INFO_HEADER_LEN + self.regions.len() * BOOT_REGION_LEN).unwrap()
        });
        let mut bytes = vec![0u8; BOOT_INFO_HEADER_LEN + self.regions.len() * BOOT_REGION_LEN];
        let header = &self.header;
        bytes[..8].copy_from_slice(&self.magic);
        bytes[8..12].copy_from_slice(&self.version.to_le_bytes());
        bytes[12..16].copy_from_slice(&size.to_le_bytes());
        for (offset, value) in [
            (16, header.phys_window_base),
            (24, header.kernel_phys_start),
            (32, header.kernel_phys_len),
            (40, header.boot_image_phys_start),
            (48, header.boot_image_phys_len),
            (56, header.page_tables_phys_start),
            (64, header.page_tables_phys_len),
            (72, header.boot_stack_phys_start),
            (80, header.boot_stack_phys_len),
            (88, header.acpi_rsdp),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[96..100].copy_from_slice(&count.to_le_bytes());
        bytes[100..104].copy_from_slice(&header.reserved.to_le_bytes());
        for (index, region) in self.regions.iter().enumerate() {
            let base = BOOT_INFO_HEADER_LEN + index * BOOT_REGION_LEN;
            bytes[base..base + 8].copy_from_slice(&region.start.to_le_bytes());
            bytes[base + 8..base + 16].copy_from_slice(&region.len.to_le_bytes());
            bytes[base + 16..base + 20].copy_from_slice(&region.kind.to_le_bytes());
            bytes[base + 20..base + 24].copy_from_slice(&region.reserved.to_le_bytes());
        }
        bytes
    }

    fn parse(&self) -> Result<(), BootInfoError> {
        BootInfoView::parse(&self.bytes()).map(|_| ())
    }
}

#[test]
fn a_valid_structure_parses_and_reports_its_content() {
    let raw = Raw::default();
    let bytes = raw.bytes();
    let view = BootInfoView::parse(&bytes).unwrap();
    assert_eq!(view.region_count(), 1);
    assert_eq!(view.phys_window_base(), 0xFFFF_8000_0000_0000);
    assert_eq!(view.acpi_rsdp(), None);
    assert_eq!(view.header().version, BOOT_INFO_VERSION);
    assert_eq!(view.header().magic, BOOT_INFO_MAGIC);
    let regions: Vec<BootRegion> = view.regions().collect();
    assert_eq!(regions, raw.regions);
    assert_eq!(view.fixed_range(FixedRange::Kernel), (MIB, MIB));
    assert_eq!(view.fixed_range(FixedRange::BootImage), (2 * MIB, MIB));
    assert_eq!(
        view.fixed_range(FixedRange::PageTables),
        (3 * MIB, 64 * PAGE_SIZE)
    );
    assert_eq!(
        view.fixed_range(FixedRange::BootStack),
        (4 * MIB, 16 * PAGE_SIZE)
    );
}

#[test]
fn fewer_bytes_than_the_structure_needs_are_too_short() {
    let bytes = Raw::default().bytes();
    assert_eq!(
        BootInfoView::parse(&bytes[..BOOT_INFO_HEADER_LEN - 1]).map(|_| ()),
        Err(BootInfoError::TooShort)
    );
    assert_eq!(
        BootInfoView::parse(&bytes[..bytes.len() - 1]).map(|_| ()),
        Err(BootInfoError::TooShort)
    );
    assert_eq!(
        BootInfoView::parse(&[]).map(|_| ()),
        Err(BootInfoError::TooShort)
    );
}

#[test]
fn wrong_magic_is_rejected() {
    let raw = Raw {
        magic: *b"AUDHBOO0",
        ..Raw::default()
    };
    assert_eq!(raw.parse(), Err(BootInfoError::BadMagic));
}

#[test]
fn wrong_version_is_rejected() {
    for version in [0, 2, u32::MAX] {
        let raw = Raw {
            version,
            ..Raw::default()
        };
        assert_eq!(raw.parse(), Err(BootInfoError::UnsupportedVersion(version)));
    }
}

#[test]
fn size_below_the_fixed_part_above_a_page_or_not_matching_the_count_is_rejected() {
    for size in [0, 103] {
        let raw = Raw {
            size: Some(size),
            ..Raw::default()
        };
        assert_eq!(raw.parse(), Err(BootInfoError::Size(size)));
    }
    let too_large = Raw {
        size: Some(4097),
        ..Raw::default()
    };
    assert_eq!(too_large.parse(), Err(BootInfoError::Size(4097)));
    let mismatch = Raw {
        size: Some(104),
        ..Raw::default()
    };
    assert_eq!(mismatch.parse(), Err(BootInfoError::Size(104)));
}

#[test]
fn region_count_zero_leaves_the_fixed_ranges_without_a_region() {
    let raw = Raw {
        regions: Vec::new(),
        ..Raw::default()
    };
    assert_eq!(
        raw.parse(),
        Err(BootInfoError::RangeOutsideRegions(FixedRange::Kernel))
    );
}

#[test]
fn region_count_at_the_limit_is_accepted_and_one_above_is_rejected() {
    let mut regions = vec![BootRegion::new(0, 64 * MIB, BootRegionKind::Usable)];
    for index in 1..MAX_BOOT_REGIONS {
        let start = 128 * MIB + u64::try_from(index).unwrap() * MIB;
        regions.push(BootRegion::new(start, MIB, BootRegionKind::Reserved));
    }
    assert_eq!(regions.len(), MAX_BOOT_REGIONS);
    let at_limit = Raw {
        regions: regions.clone(),
        ..Raw::default()
    };
    let bytes = at_limit.bytes();
    let view = BootInfoView::parse(&bytes).unwrap();
    assert_eq!(view.region_count(), MAX_BOOT_REGIONS);
    assert_eq!(
        u32::try_from(bytes.len()).unwrap(),
        view.header().size,
        "the size covers the whole structure"
    );

    let above = Raw {
        region_count: Some(u32::try_from(MAX_BOOT_REGIONS).unwrap() + 1),
        regions,
        ..Raw::default()
    };
    assert_eq!(
        above.parse(),
        Err(BootInfoError::RegionCount(
            u32::try_from(MAX_BOOT_REGIONS).unwrap() + 1
        ))
    );
}

#[test]
fn an_unknown_region_kind_is_rejected() {
    for kind in [0u32, 6, u32::MAX] {
        let raw = Raw {
            regions: vec![BootRegion {
                start: 0,
                len: 64 * MIB,
                kind,
                reserved: 0,
            }],
            ..Raw::default()
        };
        assert_eq!(
            raw.parse(),
            Err(BootInfoError::RegionKind { index: 0, kind })
        );
    }
}

#[test]
fn a_region_of_zero_length_or_beyond_the_address_space_is_rejected() {
    let empty = Raw {
        regions: vec![
            BootRegion::new(0, 64 * MIB, BootRegionKind::Usable),
            BootRegion::new(128 * MIB, 0, BootRegionKind::Reserved),
        ],
        ..Raw::default()
    };
    assert_eq!(empty.parse(), Err(BootInfoError::RegionLength(1)));
    let overflow = Raw {
        regions: vec![
            BootRegion::new(0, 64 * MIB, BootRegionKind::Usable),
            BootRegion::new(u64::MAX, 2, BootRegionKind::Reserved),
        ],
        ..Raw::default()
    };
    assert_eq!(overflow.parse(), Err(BootInfoError::RegionOverflow(1)));
}

#[test]
fn an_empty_or_overflowing_fixed_range_is_rejected() {
    let empty = Raw {
        header: BootInfoHeader {
            page_tables_phys_len: 0,
            ..valid_header()
        },
        ..Raw::default()
    };
    assert_eq!(
        empty.parse(),
        Err(BootInfoError::RangeEmpty(FixedRange::PageTables))
    );
    let overflow = Raw {
        header: BootInfoHeader {
            boot_stack_phys_start: u64::MAX,
            boot_stack_phys_len: 2,
            ..valid_header()
        },
        ..Raw::default()
    };
    assert_eq!(
        overflow.parse(),
        Err(BootInfoError::RangeOverflow(FixedRange::BootStack))
    );
}

#[test]
fn fixed_ranges_outside_every_region_are_rejected() {
    for range in FixedRange::ALL {
        let mut header = valid_header();
        let outside = 1024 * MIB;
        match range {
            FixedRange::Kernel => header.kernel_phys_start = outside,
            FixedRange::BootImage => header.boot_image_phys_start = outside,
            FixedRange::PageTables => header.page_tables_phys_start = outside,
            FixedRange::BootStack => header.boot_stack_phys_start = outside,
        }
        let raw = Raw {
            header,
            ..Raw::default()
        };
        assert_eq!(raw.parse(), Err(BootInfoError::RangeOutsideRegions(*range)));
    }
}

#[test]
fn overlapping_fixed_ranges_are_rejected_for_every_pair() {
    let pairs = [
        (FixedRange::Kernel, FixedRange::BootImage),
        (FixedRange::Kernel, FixedRange::PageTables),
        (FixedRange::Kernel, FixedRange::BootStack),
        (FixedRange::BootImage, FixedRange::PageTables),
        (FixedRange::BootImage, FixedRange::BootStack),
        (FixedRange::PageTables, FixedRange::BootStack),
    ];
    for (first, second) in pairs {
        let mut header = valid_header();
        let (start, _len) = match first {
            FixedRange::Kernel => (header.kernel_phys_start, header.kernel_phys_len),
            FixedRange::BootImage => (header.boot_image_phys_start, header.boot_image_phys_len),
            FixedRange::PageTables => (header.page_tables_phys_start, header.page_tables_phys_len),
            FixedRange::BootStack => (header.boot_stack_phys_start, header.boot_stack_phys_len),
        };
        match second {
            FixedRange::Kernel => header.kernel_phys_start = start,
            FixedRange::BootImage => header.boot_image_phys_start = start,
            FixedRange::PageTables => header.page_tables_phys_start = start,
            FixedRange::BootStack => header.boot_stack_phys_start = start,
        }
        let raw = Raw {
            header,
            ..Raw::default()
        };
        assert_eq!(raw.parse(), Err(BootInfoError::RangeOverlap(first, second)));
    }
}

#[test]
fn an_acpi_pointer_of_zero_is_absent_and_one_outside_every_region_is_rejected() {
    let absent = Raw::default();
    let bytes = absent.bytes();
    assert_eq!(BootInfoView::parse(&bytes).unwrap().acpi_rsdp(), None);

    let present = Raw {
        header: BootInfoHeader {
            acpi_rsdp: 8 * MIB,
            ..valid_header()
        },
        ..Raw::default()
    };
    let bytes = present.bytes();
    assert_eq!(
        BootInfoView::parse(&bytes).unwrap().acpi_rsdp(),
        Some(8 * MIB)
    );

    let beyond = Raw {
        header: BootInfoHeader {
            acpi_rsdp: 1024 * MIB,
            ..valid_header()
        },
        ..Raw::default()
    };
    assert_eq!(beyond.parse(), Err(BootInfoError::AcpiPointer(1024 * MIB)));
}

#[test]
fn the_writer_produces_what_the_parser_accepts() {
    let raw = Raw::default();
    let mut page = [0u8; BOOT_INFO_PAGE_LEN];
    let size = BootInfoWriter::write(&mut page, &raw.header, &raw.regions).unwrap();
    assert_eq!(
        usize::try_from(size).unwrap(),
        BOOT_INFO_HEADER_LEN + BOOT_REGION_LEN
    );
    let view = BootInfoView::parse(&page).unwrap();
    assert_eq!(view.header().size, size);
    assert_eq!(view.regions().collect::<Vec<_>>(), raw.regions);
    assert_eq!(view.header(), {
        let mut expected = raw.header;
        expected.size = size;
        expected.region_count = 1;
        expected
    });
}

#[test]
fn the_writer_rejects_more_regions_than_the_limit() {
    let regions =
        vec![BootRegion::new(PAGE_SIZE, PAGE_SIZE, BootRegionKind::Usable); MAX_BOOT_REGIONS + 1];
    let mut page = [0u8; BOOT_INFO_PAGE_LEN];
    assert_eq!(
        BootInfoWriter::write(&mut page, &valid_header(), &regions),
        Err(BootInfoError::RegionCount(
            u32::try_from(MAX_BOOT_REGIONS).unwrap() + 1
        ))
    );
}

#[test]
fn the_writer_clears_the_rest_of_the_page() {
    let raw = Raw::default();
    let mut page = [0xFFu8; BOOT_INFO_PAGE_LEN];
    let size = BootInfoWriter::write(&mut page, &raw.header, &raw.regions).unwrap();
    let tail = &page[usize::try_from(size).unwrap()..];
    assert!(tail.iter().all(|byte| *byte == 0));
}

#[test]
fn region_kinds_round_trip_through_their_codes() {
    for kind in BootRegionKind::ALL {
        assert_eq!(BootRegionKind::from_code(kind.code()), Some(*kind));
    }
    assert_eq!(BootRegionKind::from_code(0), None);
    assert_eq!(BootRegionKind::from_code(6), None);
    let codes: Vec<u32> = BootRegionKind::ALL.iter().map(|k| k.code()).collect();
    assert_eq!(codes, vec![1, 2, 3, 4, 5]);
}

#[test]
fn regions_report_their_end_and_containment() {
    let region = BootRegion::new(MIB, MIB, BootRegionKind::Usable);
    assert_eq!(region.end(), Some(2 * MIB));
    assert!(region.contains_range(MIB, MIB));
    assert!(region.contains_range(MIB + 8, 8));
    assert!(!region.contains_range(MIB - 1, 2));
    assert!(!region.contains_range(MIB, MIB + 1));
    assert!(!region.contains_range(u64::MAX, 2));
    let at_the_top = BootRegion::new(u64::MAX - 1, 1, BootRegionKind::Usable);
    assert_eq!(at_the_top.end(), Some(u64::MAX));
    let beyond = BootRegion::new(u64::MAX, 2, BootRegionKind::Usable);
    assert_eq!(beyond.end(), None);
    assert!(!beyond.contains_range(u64::MAX, 1));
}

#[test]
fn parsing_arbitrary_bytes_never_panics() {
    check("boot_info_bytes", &any_boot_info_bytes(), |bytes| {
        let _ = BootInfoView::parse(bytes);
        Ok(())
    });
}

#[test]
fn generated_regions_report_a_known_kind() {
    check(
        "boot_regions_known",
        &vec(any_boot_region(), 0..=8),
        |regions| {
            for region in regions {
                if BootRegionKind::from_code(region.kind).is_none() {
                    return Err(format!("unknown kind {}", region.kind));
                }
                if region.len == 0 {
                    return Err("empty region".to_owned());
                }
            }
            Ok(())
        },
    );
}

#[test]
fn errors_and_range_names_render_a_message() {
    let errors = [
        BootInfoError::TooShort,
        BootInfoError::BadMagic,
        BootInfoError::UnsupportedVersion(7),
        BootInfoError::Size(3),
        BootInfoError::RegionCount(999),
        BootInfoError::RegionKind { index: 1, kind: 9 },
        BootInfoError::RegionLength(2),
        BootInfoError::RegionOverflow(3),
        BootInfoError::RangeEmpty(FixedRange::Kernel),
        BootInfoError::RangeOverflow(FixedRange::BootImage),
        BootInfoError::RangeOverlap(FixedRange::PageTables, FixedRange::BootStack),
        BootInfoError::RangeOutsideRegions(FixedRange::BootStack),
        BootInfoError::AcpiPointer(5),
    ];
    for error in errors {
        assert!(!format!("{error}").is_empty());
    }
    for range in FixedRange::ALL {
        assert!(!range.name().is_empty());
    }
}
