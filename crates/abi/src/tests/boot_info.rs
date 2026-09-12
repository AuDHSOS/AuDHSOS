// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::boot_info`, covering the catalog items 6.6.10 for the
//! boot information structure.

#![allow(clippy::arithmetic_side_effects)]

use crate::boot_info::{
    BOOT_INFO_HEADER_LEN, BOOT_INFO_MAGIC, BOOT_INFO_PAGE_LEN, BOOT_INFO_VERSION, BOOT_REGION_LEN,
    BootInfoError, BootInfoHeader, BootInfoView, BootInfoWriter, BootRegion, BootRegionKind,
    FixedRange, Framebuffer, FramebufferFormat, WallClockSource,
};
use crate::layout::{MAX_BOOT_REGIONS, PAGE_SIZE};
use crate::strategies::{any_boot_info_bytes, any_boot_region, any_framebuffer};
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

/// The framebuffer every framebuffer test starts from, and the device
/// region that must enclose it.
const FRAMEBUFFER: Framebuffer = Framebuffer {
    phys_start: 0xC000_0000,
    len: 0x30_0000,
    width: 1024,
    height: 768,
    stride: 1024,
    format: FramebufferFormat::Rgbx8888,
};

fn with_framebuffer(framebuffer: Framebuffer) -> Raw {
    let mut header = valid_header();
    header.framebuffer_phys_start = framebuffer.phys_start;
    header.framebuffer_len = framebuffer.len;
    header.framebuffer_width = framebuffer.width;
    header.framebuffer_height = framebuffer.height;
    header.framebuffer_stride = framebuffer.stride;
    header.framebuffer_format = framebuffer.format.code();
    let mut regions = vec![BootRegion::new(0, 64 * MIB, BootRegionKind::Usable)];
    regions.push(BootRegion::new(
        FRAMEBUFFER.phys_start,
        FRAMEBUFFER.len,
        BootRegionKind::MmioReserved,
    ));
    Raw {
        header,
        regions,
        ..Raw::default()
    }
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
        framebuffer_phys_start: 0,
        framebuffer_len: 0,
        framebuffer_width: 0,
        framebuffer_height: 0,
        framebuffer_stride: 0,
        framebuffer_format: 0,
        region_count: 0,
        wall_clock: WallClockSource::FirmwareUtc.code(),
        boot_unix_seconds: BOOT_SECONDS,
    }
}

/// 2026-09-12T13:00:00Z, the moment the firmware clock stands at in every
/// test that does not care which moment it is.
const BOOT_SECONDS: i64 = 1_789_218_000;

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
            (96, header.framebuffer_phys_start),
            (104, header.framebuffer_len),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        for (offset, value) in [
            (112, header.framebuffer_width),
            (116, header.framebuffer_height),
            (120, header.framebuffer_stride),
            (124, header.framebuffer_format),
            (128, count),
            (132, header.wall_clock),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[136..144].copy_from_slice(&header.boot_unix_seconds.to_le_bytes());
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
    for version in [0, 1, 3, u32::MAX] {
        let raw = Raw {
            version,
            ..Raw::default()
        };
        assert_eq!(raw.parse(), Err(BootInfoError::UnsupportedVersion(version)));
    }
}

#[test]
fn size_below_the_fixed_part_above_a_page_or_not_matching_the_count_is_rejected() {
    for size in [0, 143] {
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
        size: Some(136),
        ..Raw::default()
    };
    assert_eq!(mismatch.parse(), Err(BootInfoError::Size(136)));
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
    let size = BootInfoWriter::write(&mut page, &raw.header, None, &raw.regions).unwrap();
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
        BootInfoWriter::write(&mut page, &valid_header(), None, &regions),
        Err(BootInfoError::RegionCount(
            u32::try_from(MAX_BOOT_REGIONS).unwrap() + 1
        ))
    );
}

#[test]
fn the_writer_clears_the_rest_of_the_page() {
    let raw = Raw::default();
    let mut page = [0xFFu8; BOOT_INFO_PAGE_LEN];
    let size = BootInfoWriter::write(&mut page, &raw.header, None, &raw.regions).unwrap();
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

#[test]
fn a_structure_without_a_framebuffer_reports_none() {
    let raw = Raw::default();
    let bytes = raw.bytes();
    let view = BootInfoView::parse(&bytes).unwrap();
    assert_eq!(view.framebuffer(), None);
    assert_eq!(view.header().framebuffer_format, 0);
}

#[test]
fn a_structure_with_a_framebuffer_reports_every_field() {
    let raw = with_framebuffer(FRAMEBUFFER);
    let bytes = raw.bytes();
    let view = BootInfoView::parse(&bytes).unwrap();
    assert_eq!(view.framebuffer(), Some(FRAMEBUFFER));
    assert_eq!(FRAMEBUFFER.visible_bytes(), Some(768 * 1024 * 4));
    assert_eq!(FRAMEBUFFER.end(), Some(0xC030_0000));
}

#[test]
fn a_framebuffer_field_set_without_a_format_is_rejected() {
    for change in [
        |header: &mut BootInfoHeader| header.framebuffer_phys_start = 0xC000_0000,
        |header: &mut BootInfoHeader| header.framebuffer_len = PAGE_SIZE,
        |header: &mut BootInfoHeader| header.framebuffer_width = 640,
        |header: &mut BootInfoHeader| header.framebuffer_height = 480,
        |header: &mut BootInfoHeader| header.framebuffer_stride = 640,
    ] {
        let mut header = valid_header();
        change(&mut header);
        let raw = Raw {
            header,
            ..Raw::default()
        };
        assert_eq!(
            raw.parse(),
            Err(BootInfoError::FramebufferAbsentFieldSet),
            "an absent framebuffer has every field zero"
        );
    }
}

#[test]
fn an_unknown_framebuffer_format_is_rejected() {
    for code in [3u32, 255, u32::MAX] {
        let mut raw = with_framebuffer(FRAMEBUFFER);
        raw.header.framebuffer_format = code;
        assert_eq!(raw.parse(), Err(BootInfoError::FramebufferFormat(code)));
    }
    for format in FramebufferFormat::ALL {
        assert_eq!(FramebufferFormat::from_code(format.code()), Some(*format));
        let mut raw = with_framebuffer(FRAMEBUFFER);
        raw.header.framebuffer_format = format.code();
        assert_eq!(raw.parse(), Ok(()));
    }
    assert_eq!(FramebufferFormat::from_code(0), None);
}

#[test]
fn a_framebuffer_base_that_is_zero_or_unaligned_is_rejected() {
    for base in [0u64, 0xC000_0001, 0xC000_0800] {
        let mut raw = with_framebuffer(FRAMEBUFFER);
        raw.header.framebuffer_phys_start = base;
        assert_eq!(raw.parse(), Err(BootInfoError::FramebufferBase(base)));
    }
}

#[test]
fn a_framebuffer_resolution_of_zero_or_a_short_stride_is_rejected() {
    for change in [
        |header: &mut BootInfoHeader| header.framebuffer_width = 0,
        |header: &mut BootInfoHeader| header.framebuffer_height = 0,
        |header: &mut BootInfoHeader| header.framebuffer_stride = 1023,
    ] {
        let mut raw = with_framebuffer(FRAMEBUFFER);
        change(&mut raw.header);
        assert_eq!(raw.parse(), Err(BootInfoError::FramebufferResolution));
    }
    let mut wider = with_framebuffer(FRAMEBUFFER);
    wider.header.framebuffer_width = 1000;
    assert_eq!(
        wider.parse(),
        Ok(()),
        "a stride above the width is padding, not an error"
    );
}

#[test]
fn a_framebuffer_length_that_is_short_or_unaligned_is_rejected() {
    let mut short = with_framebuffer(FRAMEBUFFER);
    short.header.framebuffer_len = FRAMEBUFFER.len - PAGE_SIZE;
    assert_eq!(
        short.parse(),
        Err(BootInfoError::FramebufferLength(
            FRAMEBUFFER.len - PAGE_SIZE
        ))
    );
    let mut unaligned = with_framebuffer(FRAMEBUFFER);
    unaligned.header.framebuffer_len = FRAMEBUFFER.len + 8;
    assert_eq!(
        unaligned.parse(),
        Err(BootInfoError::FramebufferLength(FRAMEBUFFER.len + 8))
    );
    let mut padded = with_framebuffer(FRAMEBUFFER);
    padded.header.framebuffer_len = FRAMEBUFFER.len + PAGE_SIZE;
    padded.regions = vec![
        BootRegion::new(0, 64 * MIB, BootRegionKind::Usable),
        BootRegion::new(
            FRAMEBUFFER.phys_start,
            FRAMEBUFFER.len + PAGE_SIZE,
            BootRegionKind::MmioReserved,
        ),
    ];
    assert_eq!(padded.parse(), Ok(()), "a longer framebuffer is allowed");
}

#[test]
fn a_framebuffer_whose_size_is_not_representable_is_rejected() {
    let mut overflowing = with_framebuffer(FRAMEBUFFER);
    overflowing.header.framebuffer_width = u32::MAX;
    overflowing.header.framebuffer_height = u32::MAX;
    overflowing.header.framebuffer_stride = u32::MAX;
    assert_eq!(overflowing.parse(), Err(BootInfoError::FramebufferOverflow));
    let huge = Framebuffer {
        height: u32::MAX,
        stride: u32::MAX,
        ..FRAMEBUFFER
    };
    assert_eq!(huge.visible_bytes(), None);

    let mut large = with_framebuffer(FRAMEBUFFER);
    large.header.framebuffer_width = 0x1_0000;
    large.header.framebuffer_height = 0x1_0000;
    large.header.framebuffer_stride = 0x1_0000;
    assert_eq!(
        large.parse(),
        Err(BootInfoError::FramebufferLength(FRAMEBUFFER.len)),
        "the visible size fits into u64, the length does not cover it"
    );
    let representable = Framebuffer {
        width: 0x1_0000,
        height: 0x1_0000,
        stride: 0x1_0000,
        ..FRAMEBUFFER
    };
    assert_eq!(representable.visible_bytes(), Some(0x4_0000_0000));

    let at_the_top = Framebuffer {
        phys_start: u64::MAX - PAGE_SIZE + 1,
        len: PAGE_SIZE,
        ..FRAMEBUFFER
    };
    assert_eq!(at_the_top.end(), None);
}

#[test]
fn a_framebuffer_overlapping_usable_memory_is_rejected() {
    let mut raw = with_framebuffer(FRAMEBUFFER);
    raw.regions = vec![
        BootRegion::new(0, 0xC010_0000, BootRegionKind::Usable),
        BootRegion::new(
            FRAMEBUFFER.phys_start,
            FRAMEBUFFER.len,
            BootRegionKind::MmioReserved,
        ),
    ];
    assert_eq!(raw.parse(), Err(BootInfoError::FramebufferOverlapsUsable));
}

#[test]
fn a_framebuffer_without_an_enclosing_device_region_is_rejected() {
    let mut missing = with_framebuffer(FRAMEBUFFER);
    missing.regions = vec![BootRegion::new(0, 64 * MIB, BootRegionKind::Usable)];
    assert_eq!(missing.parse(), Err(BootInfoError::FramebufferNotReserved));

    let mut too_small = with_framebuffer(FRAMEBUFFER);
    too_small.regions = vec![
        BootRegion::new(0, 64 * MIB, BootRegionKind::Usable),
        BootRegion::new(
            FRAMEBUFFER.phys_start,
            FRAMEBUFFER.len - PAGE_SIZE,
            BootRegionKind::MmioReserved,
        ),
    ];
    assert_eq!(
        too_small.parse(),
        Err(BootInfoError::FramebufferNotReserved)
    );

    let mut wrong_kind = with_framebuffer(FRAMEBUFFER);
    wrong_kind.regions = vec![
        BootRegion::new(0, 64 * MIB, BootRegionKind::Usable),
        BootRegion::new(
            FRAMEBUFFER.phys_start,
            FRAMEBUFFER.len,
            BootRegionKind::Reserved,
        ),
    ];
    assert_eq!(
        wrong_kind.parse(),
        Err(BootInfoError::FramebufferNotReserved),
        "the loader reports the framebuffer as device memory"
    );
}

#[test]
fn the_writer_takes_the_framebuffer_the_parser_reports() {
    let raw = with_framebuffer(FRAMEBUFFER);
    let mut page = [0u8; BOOT_INFO_PAGE_LEN];
    let size =
        BootInfoWriter::write(&mut page, &valid_header(), Some(FRAMEBUFFER), &raw.regions).unwrap();
    let view = BootInfoView::parse(&page).unwrap();
    assert_eq!(view.framebuffer(), Some(FRAMEBUFFER));
    assert_eq!(view.header().size, size);
    assert_eq!(
        usize::try_from(size).unwrap(),
        BOOT_INFO_HEADER_LEN + 2 * BOOT_REGION_LEN
    );

    let mut absent = [0u8; BOOT_INFO_PAGE_LEN];
    BootInfoWriter::write(&mut absent, &valid_header(), None, &raw.regions).unwrap();
    let view = BootInfoView::parse(&absent).unwrap();
    assert_eq!(
        view.framebuffer(),
        None,
        "the writer takes the framebuffer from its argument, not from the header"
    );
}

#[test]
fn the_fixed_part_is_one_hundred_and_forty_four_bytes() {
    assert_eq!(BOOT_INFO_HEADER_LEN, 144);
    assert_eq!(
        BOOT_INFO_HEADER_LEN + MAX_BOOT_REGIONS * BOOT_REGION_LEN,
        3216
    );
    const { assert!(BOOT_INFO_HEADER_LEN + MAX_BOOT_REGIONS * BOOT_REGION_LEN <= BOOT_INFO_PAGE_LEN) };
}

#[test]
fn property_the_writer_produces_what_the_parser_accepts() {
    check("boot_info_round_trip", &any_framebuffer(), |framebuffer| {
        let mut regions = vec![BootRegion::new(0, 64 * MIB, BootRegionKind::Usable)];
        if let Some(buffer) = framebuffer {
            regions.push(BootRegion::new(
                buffer.phys_start,
                buffer.len,
                BootRegionKind::MmioReserved,
            ));
        }
        let mut page = [0u8; BOOT_INFO_PAGE_LEN];
        BootInfoWriter::write(&mut page, &valid_header(), *framebuffer, &regions)
            .map_err(|error| format!("the writer refused: {error}"))?;
        let view = BootInfoView::parse(&page).map_err(|error| format!("rejected: {error}"))?;
        if view.framebuffer() != *framebuffer {
            return Err(format!("{:?} did not round-trip", view.framebuffer()));
        }
        if view.regions().count() != regions.len() {
            return Err("the regions did not round-trip".to_owned());
        }
        Ok(())
    });
}

#[test]
fn every_wall_clock_source_round_trips_its_code() {
    let mut codes = Vec::new();
    for source in WallClockSource::ALL {
        assert_eq!(WallClockSource::from_code(source.code()), Some(*source));
        codes.push(source.code());
    }
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), WallClockSource::ALL.len());
    assert_eq!(WallClockSource::from_code(0), None);
    assert_eq!(WallClockSource::from_code(3), None);
}

#[test]
fn the_wall_clock_is_reported_with_the_source_it_came_from() {
    for source in WallClockSource::ALL {
        let mut header = valid_header();
        header.wall_clock = source.code();
        let raw = Raw {
            header,
            ..Raw::default()
        };
        let bytes = raw.bytes();
        let view = BootInfoView::parse(&bytes).unwrap();
        assert_eq!(view.wall_clock(), Some((BOOT_SECONDS, *source)));
    }
}

#[test]
fn a_machine_that_reported_no_wall_clock_parses_and_has_none() {
    let mut header = valid_header();
    header.wall_clock = 0;
    header.boot_unix_seconds = 0;
    let raw = Raw {
        header,
        ..Raw::default()
    };
    let bytes = raw.bytes();
    let view = BootInfoView::parse(&bytes).unwrap();
    assert_eq!(view.wall_clock(), None);
}

#[test]
fn an_unknown_wall_clock_source_is_rejected() {
    for code in [3, 4, u32::MAX] {
        let mut header = valid_header();
        header.wall_clock = code;
        let raw = Raw {
            header,
            ..Raw::default()
        };
        assert_eq!(raw.parse(), Err(BootInfoError::WallClockSource(code)));
    }
}

#[test]
fn a_source_whose_seconds_are_not_after_the_epoch_is_rejected() {
    for seconds in [0, -1, i64::MIN] {
        let mut header = valid_header();
        header.boot_unix_seconds = seconds;
        let raw = Raw {
            header,
            ..Raw::default()
        };
        assert_eq!(raw.parse(), Err(BootInfoError::WallClockTime(seconds)));
    }
}

#[test]
fn seconds_without_a_source_are_rejected() {
    for seconds in [1, BOOT_SECONDS, -1] {
        let mut header = valid_header();
        header.wall_clock = 0;
        header.boot_unix_seconds = seconds;
        let raw = Raw {
            header,
            ..Raw::default()
        };
        assert_eq!(raw.parse(), Err(BootInfoError::WallClockTime(seconds)));
    }
}

#[test]
fn the_writer_carries_the_wall_clock_of_its_header() {
    let mut header = valid_header();
    header.wall_clock = WallClockSource::FirmwareUnspecifiedZone.code();
    let regions = [BootRegion::new(0, 64 * MIB, BootRegionKind::Usable)];
    let mut page = [0u8; BOOT_INFO_PAGE_LEN];
    BootInfoWriter::write(&mut page, &header, None, &regions).unwrap();
    let view = BootInfoView::parse(&page).unwrap();
    assert_eq!(
        view.wall_clock(),
        Some((BOOT_SECONDS, WallClockSource::FirmwareUnspecifiedZone))
    );
}
