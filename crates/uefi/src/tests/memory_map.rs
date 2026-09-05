// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::memory_map`, covering the memory map items of the catalog
//! 6.6.14.

#![allow(clippy::arithmetic_side_effects)]

use audhsos_abi::boot_info::{BootRegion, BootRegionKind};
use audhsos_abi::layout::{MAX_BOOT_REGIONS, PAGE_SIZE};
use test_support::generators::{range, vec};
use test_support::property::check;

use crate::memory_map::{
    ConversionError, DESCRIPTOR_LEN, MemoryDescriptor, MemoryType, descriptors, to_boot_regions,
};

/// A buffer of descriptors written with the given stride, as the firmware
/// reports them.
fn buffer(entries: &[MemoryDescriptor], stride: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; entries.len() * stride];
    for (index, entry) in entries.iter().enumerate() {
        let base = index * stride;
        bytes[base..base + 4].copy_from_slice(&entry.type_.to_le_bytes());
        bytes[base + 8..base + 16].copy_from_slice(&entry.physical_start.to_le_bytes());
        bytes[base + 16..base + 24].copy_from_slice(&entry.virtual_start.to_le_bytes());
        bytes[base + 24..base + 32].copy_from_slice(&entry.pages.to_le_bytes());
        bytes[base + 32..base + 40].copy_from_slice(&entry.attribute.to_le_bytes());
    }
    bytes
}

fn descriptor(kind: MemoryType, start: u64, pages: u64) -> MemoryDescriptor {
    MemoryDescriptor {
        type_: kind.code(),
        physical_start: start,
        virtual_start: 0,
        pages,
        attribute: 0xF,
    }
}

fn empty_regions() -> [BootRegion; MAX_BOOT_REGIONS] {
    [BootRegion::new(0, 0, BootRegionKind::Reserved); MAX_BOOT_REGIONS]
}

fn convert(entries: &[MemoryDescriptor]) -> Result<Vec<BootRegion>, ConversionError> {
    let mut out = empty_regions();
    let count = to_boot_regions(entries.iter().copied(), &mut out)?;
    Ok(out[..count].to_vec())
}

#[test]
fn every_memory_type_maps_to_exactly_one_region_kind() {
    let expected = [
        (MemoryType::Reserved, BootRegionKind::Reserved),
        (MemoryType::LoaderCode, BootRegionKind::Usable),
        (MemoryType::LoaderData, BootRegionKind::Usable),
        (MemoryType::BootServicesCode, BootRegionKind::Usable),
        (MemoryType::BootServicesData, BootRegionKind::Usable),
        (MemoryType::RuntimeServicesCode, BootRegionKind::Reserved),
        (MemoryType::RuntimeServicesData, BootRegionKind::Reserved),
        (MemoryType::Conventional, BootRegionKind::Usable),
        (MemoryType::Unusable, BootRegionKind::Reserved),
        (MemoryType::AcpiReclaim, BootRegionKind::AcpiReclaimable),
        (MemoryType::AcpiNvs, BootRegionKind::AcpiNvs),
        (MemoryType::MappedIo, BootRegionKind::MmioReserved),
        (MemoryType::MappedIoPortSpace, BootRegionKind::MmioReserved),
        (MemoryType::PalCode, BootRegionKind::Reserved),
        (MemoryType::Persistent, BootRegionKind::Reserved),
        (MemoryType::Unaccepted, BootRegionKind::Reserved),
    ];
    assert_eq!(expected.len(), MemoryType::ALL.len());
    for (kind, region) in expected {
        assert_eq!(kind.region_kind(), region, "{kind:?}");
        assert_eq!(MemoryType::from_u32(kind.code()), Some(kind));
    }
    assert_eq!(MemoryType::from_u32(16), None);
    assert_eq!(MemoryType::from_u32(u32::MAX), None);
    assert_eq!(
        descriptor(MemoryType::Conventional, 0, 1).region_kind(),
        BootRegionKind::Usable
    );
    let unknown = MemoryDescriptor {
        type_: 99,
        ..descriptor(MemoryType::Conventional, 0, 1)
    };
    assert_eq!(
        unknown.region_kind(),
        BootRegionKind::Reserved,
        "an unknown type is reserved, not an error"
    );
}

#[test]
fn the_stride_the_firmware_reports_is_honored() {
    let entries = [
        descriptor(MemoryType::Conventional, 0, 1),
        descriptor(MemoryType::Reserved, 0x1_0000, 2),
    ];
    for stride in [DESCRIPTOR_LEN, 48, 64, 128] {
        let bytes = buffer(&entries, stride);
        let read: Vec<MemoryDescriptor> = descriptors(&bytes, stride).collect();
        assert_eq!(read, entries.to_vec(), "stride {stride}");
    }
}

#[test]
fn a_stride_below_the_structure_yields_nothing() {
    let bytes = buffer(
        &[descriptor(MemoryType::Conventional, 0, 1)],
        DESCRIPTOR_LEN,
    );
    assert_eq!(descriptors(&bytes, DESCRIPTOR_LEN - 1).count(), 0);
    assert_eq!(descriptors(&bytes, 0).count(), 0);
    assert_eq!(descriptors(&[], DESCRIPTOR_LEN).count(), 0);
}

#[test]
fn a_truncated_last_descriptor_is_dropped() {
    let bytes = buffer(
        &[descriptor(MemoryType::Conventional, 0, 1)],
        DESCRIPTOR_LEN,
    );
    let short = &bytes[..DESCRIPTOR_LEN - 1];
    assert_eq!(descriptors(short, DESCRIPTOR_LEN).count(), 0);
    assert_eq!(MemoryDescriptor::decode(short), None);
    assert!(MemoryDescriptor::decode(&bytes).is_some());
}

#[test]
fn an_empty_map_produces_no_region() {
    assert_eq!(convert(&[]), Ok(Vec::new()));
    assert_eq!(
        convert(&[descriptor(MemoryType::Conventional, 0x1000, 0)]),
        Ok(Vec::new()),
        "a descriptor of zero pages is dropped"
    );
}

#[test]
fn descriptors_out_of_order_come_back_sorted() {
    let entries = [
        descriptor(MemoryType::Conventional, 0x8000, 1),
        descriptor(MemoryType::Reserved, 0x1000, 1),
        descriptor(MemoryType::AcpiNvs, 0x4000, 1),
    ];
    let regions = convert(&entries).unwrap();
    let starts: Vec<u64> = regions.iter().map(|region| region.start).collect();
    assert_eq!(starts, vec![0x1000, 0x4000, 0x8000]);
    assert_eq!(
        regions.iter().map(|region| region.kind).collect::<Vec<_>>(),
        vec![
            BootRegionKind::Reserved.code(),
            BootRegionKind::AcpiNvs.code(),
            BootRegionKind::Usable.code(),
        ]
    );
}

#[test]
fn adjacent_and_overlapping_regions_of_one_kind_are_merged() {
    let adjacent = [
        descriptor(MemoryType::Conventional, 0x1000, 1),
        descriptor(MemoryType::LoaderData, 0x2000, 1),
        descriptor(MemoryType::BootServicesCode, 0x3000, 2),
    ];
    let regions = convert(&adjacent).unwrap();
    assert_eq!(
        regions.len(),
        1,
        "loader and boot services memory is usable"
    );
    assert_eq!(
        regions.first().map(|region| (region.start, region.len)),
        Some((0x1000, 4 * PAGE_SIZE))
    );

    let overlapping = [
        descriptor(MemoryType::Conventional, 0x1000, 4),
        descriptor(MemoryType::Conventional, 0x2000, 1),
    ];
    let merged = convert(&overlapping).unwrap();
    assert_eq!(merged.len(), 1);
    assert_eq!(merged.first().map(|region| region.len), Some(4 * PAGE_SIZE));
}

#[test]
fn a_gap_keeps_the_regions_apart() {
    let entries = [
        descriptor(MemoryType::Conventional, 0x1000, 1),
        descriptor(MemoryType::Conventional, 0x3000, 1),
    ];
    let regions = convert(&entries).unwrap();
    assert_eq!(regions.len(), 2);
}

#[test]
fn overlapping_regions_of_different_kinds_are_rejected() {
    let entries = [
        descriptor(MemoryType::Conventional, 0x1000, 4),
        descriptor(MemoryType::AcpiNvs, 0x2000, 1),
    ];
    assert_eq!(convert(&entries), Err(ConversionError::Overlap));
}

#[test]
fn conventional_memory_below_one_mebibyte_is_kept() {
    let entries = [
        descriptor(MemoryType::Conventional, 0, 0x9F),
        descriptor(MemoryType::Reserved, 0x9F000, 0x61),
        descriptor(MemoryType::Conventional, 0x10_0000, 0x100),
    ];
    let regions = convert(&entries).unwrap();
    assert_eq!(regions.len(), 3);
    assert_eq!(regions.first().map(|region| region.start), Some(0));
    assert_eq!(
        regions.first().map(|region| region.kind),
        Some(BootRegionKind::Usable.code())
    );
}

#[test]
fn a_descriptor_whose_end_overflows_is_rejected() {
    let too_many_pages = MemoryDescriptor {
        pages: u64::MAX,
        ..descriptor(MemoryType::Conventional, 0x1000, 1)
    };
    assert_eq!(convert(&[too_many_pages]), Err(ConversionError::Overflow));
    let at_the_top = descriptor(MemoryType::Conventional, u64::MAX - PAGE_SIZE + 1, 1);
    assert_eq!(
        convert(&[at_the_top]),
        Err(ConversionError::Overflow),
        "the region array names an exclusive end, which the last page has not"
    );
    let one_page_lower = descriptor(MemoryType::Conventional, u64::MAX - 2 * PAGE_SIZE + 1, 1);
    assert_eq!(
        convert(&[one_page_lower]).map(|regions| regions.len()),
        Ok(1)
    );
    assert_eq!(too_many_pages.bytes(), None);
    assert_eq!(at_the_top.end(), None);
    assert_eq!(one_page_lower.bytes(), Some(PAGE_SIZE));
}

#[test]
fn more_regions_than_the_boot_information_holds_are_rejected() {
    let at_limit: Vec<MemoryDescriptor> = (0..MAX_BOOT_REGIONS)
        .map(|index| {
            descriptor(
                MemoryType::Conventional,
                u64::try_from(index).unwrap() * 4 * PAGE_SIZE + PAGE_SIZE,
                1,
            )
        })
        .collect();
    assert_eq!(
        convert(&at_limit).map(|regions| regions.len()),
        Ok(MAX_BOOT_REGIONS)
    );

    let mut above = at_limit;
    above.push(descriptor(
        MemoryType::Conventional,
        u64::try_from(MAX_BOOT_REGIONS).unwrap() * 4 * PAGE_SIZE + PAGE_SIZE,
        1,
    ));
    assert_eq!(convert(&above), Err(ConversionError::TooManyRegions));
}

#[test]
fn regions_that_merge_below_the_limit_are_accepted() {
    let touching: Vec<MemoryDescriptor> = (0..MAX_BOOT_REGIONS + 8)
        .map(|index| {
            descriptor(
                MemoryType::Conventional,
                u64::try_from(index).unwrap() * PAGE_SIZE,
                1,
            )
        })
        .collect();
    assert_eq!(
        convert(&touching),
        Err(ConversionError::TooManyRegions),
        "the limit applies before merging, so that the array never overflows"
    );
}

#[test]
fn the_error_renders_a_message() {
    for error in [
        ConversionError::TooManyRegions,
        ConversionError::Overflow,
        ConversionError::Overlap,
    ] {
        assert!(!format!("{error}").is_empty());
    }
}

#[test]
fn property_the_result_is_sorted_and_disjoint() {
    let entries = vec(
        test_support::generators::pair(range(0u32..=17), range(0u64..=64)),
        0..=32,
    );
    check("uefi_regions", &entries, |cases| {
        let descriptors: Vec<MemoryDescriptor> = cases
            .iter()
            .map(|(kind, page)| {
                descriptor(
                    MemoryType::from_u32(*kind).unwrap_or(MemoryType::Reserved),
                    page * PAGE_SIZE,
                    1,
                )
            })
            .collect();
        let regions = match convert(&descriptors) {
            Ok(regions) => regions,
            Err(ConversionError::Overlap | ConversionError::TooManyRegions) => return Ok(()),
            Err(error) => return Err(format!("unexpected {error}")),
        };
        let mut previous: Option<BootRegion> = None;
        for region in regions {
            if region.len == 0 {
                return Err("empty region".to_owned());
            }
            if let Some(previous) = previous {
                let end = previous.end().ok_or("overflow".to_owned())?;
                if region.start < end {
                    return Err(format!("{region:?} shares bytes with the one before it"));
                }
                if region.start == end && region.kind == previous.kind {
                    return Err(format!("{region:?} should have been merged"));
                }
            }
            previous = Some(region);
        }
        Ok(())
    });
}
