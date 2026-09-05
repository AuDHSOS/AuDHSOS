// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::memory_map`, covering the catalog items 6.6.2.

#![allow(clippy::arithmetic_side_effects)]

use audhsos_abi::layout::{MAX_BOOT_REGIONS, PAGE_SIZE};
use kernel_hal_api::platform::{MemoryRegion, MemoryRegionKind};
use kernel_types::{PhysAddr, PhysFrameRange};
use test_support::property::check;

use crate::memory_map::{MapError, NormalizedMap, Span, SpanList, normalize};
use crate::strategies::{any_memory_regions, any_populated_memory_regions};

const GIB: u64 = 1024 * 1024 * 1024;

fn region(start: u64, len: u64, kind: MemoryRegionKind) -> MemoryRegion {
    MemoryRegion {
        start: PhysAddr::new(start).unwrap(),
        len,
        kind,
    }
}

fn usable(start: u64, len: u64) -> MemoryRegion {
    region(start, len, MemoryRegionKind::Usable)
}

fn reserved(start: u64, len: u64) -> MemoryRegion {
    region(start, len, MemoryRegionKind::Reserved)
}

/// The normalized map as pairs of frame numbers.
fn spans(regions: &[MemoryRegion]) -> Vec<(u64, u64)> {
    normalize(regions)
        .unwrap()
        .iter()
        .map(|range| (range.start().number(), range.end_number()))
        .collect()
}

#[test]
fn an_empty_map_has_no_usable_memory() {
    assert_eq!(normalize(&[]).err(), Some(MapError::NoUsableMemory));
    assert_eq!(
        normalize(&[reserved(0, 4 * PAGE_SIZE)]).err(),
        Some(MapError::NoUsableMemory)
    );
}

#[test]
fn a_single_region_survives_and_reports_its_size() {
    let map = normalize(&[usable(0x1000, 4 * PAGE_SIZE)]).unwrap();
    assert_eq!(map.len(), 1);
    assert!(!map.is_empty());
    assert_eq!(map.total_frames(), 4);
    assert_eq!(map.total_bytes(), 4 * PAGE_SIZE);
    assert!(map.is_sorted_and_disjoint());
    assert_eq!(
        map.iter().next().map(|range| range.start().number()),
        Some(1)
    );
}

#[test]
fn a_region_of_one_frame_survives_and_a_region_of_zero_length_is_ignored() {
    assert_eq!(spans(&[usable(0x2000, PAGE_SIZE)]), vec![(2, 3)]);
    assert_eq!(
        spans(&[usable(0x2000, PAGE_SIZE), usable(0x8000, 0)]),
        vec![(2, 3)]
    );
    assert_eq!(
        spans(&[usable(0x2000, PAGE_SIZE), reserved(0x8000, 0)]),
        vec![(2, 3)]
    );
}

#[test]
fn unsorted_duplicate_touching_and_overlapping_regions_are_merged() {
    let unsorted = [
        usable(0x8000, PAGE_SIZE),
        usable(0x1000, PAGE_SIZE),
        usable(0x4000, PAGE_SIZE),
    ];
    assert_eq!(spans(&unsorted), vec![(1, 2), (4, 5), (8, 9)]);

    let duplicates = [usable(0x1000, 2 * PAGE_SIZE), usable(0x1000, 2 * PAGE_SIZE)];
    assert_eq!(spans(&duplicates), vec![(1, 3)]);

    let touching = [usable(0x1000, PAGE_SIZE), usable(0x2000, PAGE_SIZE)];
    assert_eq!(spans(&touching), vec![(1, 3)]);

    let partial = [usable(0x1000, 3 * PAGE_SIZE), usable(0x3000, 3 * PAGE_SIZE)];
    assert_eq!(spans(&partial), vec![(1, 6)]);

    let contained = [usable(0x1000, 8 * PAGE_SIZE), usable(0x3000, PAGE_SIZE)];
    assert_eq!(spans(&contained), vec![(1, 9)]);
}

#[test]
fn a_reserved_region_is_cut_out_at_every_position() {
    let base = usable(0x1000, 8 * PAGE_SIZE);

    let at_the_start = [base, reserved(0x1000, 2 * PAGE_SIZE)];
    assert_eq!(spans(&at_the_start), vec![(3, 9)]);

    let at_the_end = [base, reserved(0x7000, 2 * PAGE_SIZE)];
    assert_eq!(spans(&at_the_end), vec![(1, 7)]);

    let in_the_middle = [base, reserved(0x3000, 2 * PAGE_SIZE)];
    assert_eq!(spans(&in_the_middle), vec![(1, 3), (5, 9)]);

    let covering = [base, reserved(0x0, 16 * PAGE_SIZE)];
    assert_eq!(
        normalize(&covering).err(),
        Some(MapError::NoUsableMemory),
        "a reserved region covering everything leaves nothing"
    );

    let beyond_both_ends = [base, reserved(0x0, 32 * PAGE_SIZE)];
    assert_eq!(
        normalize(&beyond_both_ends).err(),
        Some(MapError::NoUsableMemory)
    );
}

#[test]
fn the_four_loader_ranges_are_cut_out_like_reserved_memory() {
    let kinds = [
        MemoryRegionKind::Kernel,
        MemoryRegionKind::BootImage,
        MemoryRegionKind::PageTables,
        MemoryRegionKind::BootStack,
        MemoryRegionKind::BootInfo,
        MemoryRegionKind::AcpiReclaimable,
        MemoryRegionKind::AcpiNvs,
        MemoryRegionKind::MmioReserved,
    ];
    for kind in kinds {
        let front = [
            usable(0x1000, 8 * PAGE_SIZE),
            region(0x1000, PAGE_SIZE, kind),
        ];
        assert_eq!(spans(&front), vec![(2, 9)], "{kind:?} at the front");
        let middle = [
            usable(0x1000, 8 * PAGE_SIZE),
            region(0x4000, PAGE_SIZE, kind),
        ];
        assert_eq!(
            spans(&middle),
            vec![(1, 4), (5, 9)],
            "{kind:?} in the middle"
        );
        let back = [
            usable(0x1000, 8 * PAGE_SIZE),
            region(0x8000, PAGE_SIZE, kind),
        ];
        assert_eq!(spans(&back), vec![(1, 8)], "{kind:?} at the back");
    }
}

#[test]
fn usable_regions_shrink_and_excluded_regions_grow_to_frame_boundaries() {
    assert_eq!(
        spans(&[usable(0x1800, 3 * PAGE_SIZE)]),
        vec![(2, 4)],
        "the start rounds up and the end rounds down"
    );
    assert_eq!(
        spans(&[usable(0x1000, 4 * PAGE_SIZE), reserved(0x2800, 8)]),
        vec![(1, 2), (3, 5)],
        "an excluded region grows to whole frames"
    );
}

#[test]
fn a_region_that_becomes_empty_after_alignment_is_dropped() {
    assert_eq!(
        normalize(&[usable(0x1800, 8)]).err(),
        Some(MapError::NoUsableMemory)
    );
    assert_eq!(
        spans(&[usable(0x1800, 8), usable(0x4000, PAGE_SIZE)]),
        vec![(4, 5)]
    );
}

#[test]
fn regions_above_four_gibibytes_are_kept() {
    let map = normalize(&[usable(4 * GIB, 16 * PAGE_SIZE)]).unwrap();
    assert_eq!(map.total_frames(), 16);
    assert_eq!(
        map.iter()
            .next()
            .map(|range| range.start().start().as_u64()),
        Some(4 * GIB)
    );
}

#[test]
fn a_region_whose_end_would_overflow_is_reported() {
    let overflowing = MemoryRegion {
        start: PhysAddr::MAX,
        len: u64::MAX,
        kind: MemoryRegionKind::Usable,
    };
    assert_eq!(
        normalize(&[overflowing]).err(),
        Some(MapError::Address(kernel_types::Error::Overflow))
    );
    let excluded = MemoryRegion {
        start: PhysAddr::MAX,
        len: u64::MAX,
        kind: MemoryRegionKind::Reserved,
    };
    assert_eq!(
        normalize(&[usable(0x1000, PAGE_SIZE), excluded]).err(),
        Some(MapError::Address(kernel_types::Error::Overflow))
    );
}

#[test]
fn a_region_reaching_the_top_of_the_address_space_is_clamped() {
    let top = MemoryRegion {
        start: PhysAddr::new(PhysAddr::MAX.as_u64() + 1 - 4 * PAGE_SIZE).unwrap(),
        len: 4 * PAGE_SIZE,
        kind: MemoryRegionKind::Usable,
    };
    let map = normalize(&[top]).unwrap();
    assert_eq!(map.total_frames(), 4);
    assert!(map.is_sorted_and_disjoint());
}

#[test]
fn the_region_count_at_the_limit_is_accepted_and_one_above_is_rejected() {
    let at_limit: Vec<MemoryRegion> = (0..MAX_BOOT_REGIONS)
        .map(|index| {
            usable(
                u64::try_from(index).unwrap() * 4 * PAGE_SIZE + PAGE_SIZE,
                PAGE_SIZE,
            )
        })
        .collect();
    let map = normalize(&at_limit).unwrap();
    assert_eq!(map.len(), MAX_BOOT_REGIONS);
    assert!(map.is_sorted_and_disjoint());

    let mut above = at_limit;
    above.push(usable(
        u64::try_from(MAX_BOOT_REGIONS).unwrap() * 4 * PAGE_SIZE + PAGE_SIZE,
        PAGE_SIZE,
    ));
    assert_eq!(normalize(&above).err(), Some(MapError::TooManyRegions));
}

#[test]
fn a_split_that_would_exceed_the_limit_is_rejected_instead_of_truncating() {
    let mut regions: Vec<MemoryRegion> = (0..MAX_BOOT_REGIONS)
        .map(|index| {
            usable(
                u64::try_from(index).unwrap() * 4 * PAGE_SIZE + PAGE_SIZE,
                PAGE_SIZE,
            )
        })
        .collect();
    regions.push(usable(GIB, 8 * PAGE_SIZE));
    assert_eq!(normalize(&regions).err(), Some(MapError::TooManyRegions));

    let mut splitting: Vec<MemoryRegion> = (0..MAX_BOOT_REGIONS - 1)
        .map(|index| {
            usable(
                u64::try_from(index).unwrap() * 4 * PAGE_SIZE + PAGE_SIZE,
                PAGE_SIZE,
            )
        })
        .collect();
    splitting.push(usable(GIB, 8 * PAGE_SIZE));
    assert_eq!(normalize(&splitting).unwrap().len(), MAX_BOOT_REGIONS);
    splitting.push(reserved(GIB + 2 * PAGE_SIZE, PAGE_SIZE));
    assert_eq!(normalize(&splitting).err(), Some(MapError::TooManyRegions));
}

#[test]
fn errors_render_a_message() {
    let errors = [
        MapError::TooManyRegions,
        MapError::NoUsableMemory,
        MapError::ReserveDoesNotFit,
        MapError::Address(kernel_types::Error::Overflow),
    ];
    for error in errors {
        assert!(!format!("{error}").is_empty());
    }
}

#[test]
fn property_the_result_is_sorted_disjoint_and_free_of_excluded_frames() {
    check(
        "normalize_invariants",
        &any_populated_memory_regions(),
        |regions| {
            let map = normalize(regions).map_err(|error| format!("rejected: {error}"))?;
            if !map.is_sorted_and_disjoint() {
                return Err("the result is not sorted and disjoint".to_owned());
            }
            if map.len() > MAX_BOOT_REGIONS {
                return Err("too many ranges".to_owned());
            }
            for range in map.iter() {
                for excluded in regions
                    .iter()
                    .filter(|region| region.kind != MemoryRegionKind::Usable && region.len != 0)
                {
                    let start = excluded.start.as_u64() >> 12;
                    let end = (excluded.start.as_u64() + excluded.len).div_ceil(4096);
                    if range.start().number() < end && start < range.end_number() {
                        return Err(format!("{range:?} keeps excluded frames"));
                    }
                }
            }
            Ok(())
        },
    );
}

#[test]
fn property_normalization_never_panics() {
    check("normalize_total", &any_memory_regions(), |regions| {
        let _ = normalize(regions);
        Ok(())
    });
}

#[test]
fn the_span_list_ignores_indices_outside_the_array() {
    let mut list = SpanList::default();
    let span = Span { start: 1, end: 2 };
    assert_eq!(list.len(), 0);
    assert_eq!(list.get(0), None, "the list is empty");
    list.put(MAX_BOOT_REGIONS + 5, span);
    assert_eq!(list.as_slice(), &[]);
    list.remove(3);
    assert_eq!(list.len(), 0, "removing outside the used part does nothing");
    assert_eq!(list.insert(3, span).err(), Some(MapError::TooManyRegions));
    assert_eq!(list.push(span), Ok(()));
    assert_eq!(list.get(0), Some(span));
    assert_eq!(
        list.get(1),
        None,
        "a slot above the used part reports nothing"
    );
    list.put(0, Span { start: 4, end: 6 });
    assert_eq!(list.get(0), Some(Span { start: 4, end: 6 }));
}

#[test]
fn the_span_list_reports_a_full_array() {
    let mut list = SpanList::default();
    for index in 0..MAX_BOOT_REGIONS {
        let start = u64::try_from(index).unwrap() * 2;
        assert_eq!(
            list.push(Span {
                start,
                end: start + 1
            }),
            Ok(())
        );
    }
    assert_eq!(list.len(), MAX_BOOT_REGIONS);
    assert_eq!(
        list.push(Span { start: 0, end: 1 }).err(),
        Some(MapError::TooManyRegions)
    );
    assert_eq!(
        list.insert(0, Span { start: 0, end: 1 }).err(),
        Some(MapError::TooManyRegions)
    );
}

#[test]
fn merging_drops_empty_spans_and_subtracting_nothing_changes_nothing() {
    let mut list = SpanList::default();
    list.push(Span { start: 4, end: 6 }).unwrap();
    list.push(Span { start: 2, end: 2 }).unwrap();
    list.push(Span { start: 1, end: 3 }).unwrap();
    list.sort_and_merge();
    assert_eq!(
        list.as_slice(),
        &[Span { start: 1, end: 3 }, Span { start: 4, end: 6 }],
        "the empty span is dropped and the rest is sorted"
    );
    assert_eq!(list.subtract(Span { start: 5, end: 5 }), Ok(()));
    assert_eq!(
        list.as_slice(),
        &[Span { start: 1, end: 3 }, Span { start: 4, end: 6 }],
        "an empty cut changes nothing"
    );
    assert_eq!(Span { start: 1, end: 3 }.frames(), 2);
    assert!(Span { start: 3, end: 3 }.is_empty());
}

#[test]
fn a_map_holding_an_empty_span_fails_its_own_invariant_check() {
    let mut list = SpanList::default();
    list.push(Span { start: 4, end: 4 }).unwrap();
    let map = NormalizedMap::from_spans(&list);
    assert!(
        !map.is_sorted_and_disjoint(),
        "an empty range breaks the invariant"
    );
    assert_eq!(map.total_frames(), 0);
    assert_eq!(map.iter().count(), 1);

    let mut unsorted = SpanList::default();
    unsorted.push(Span { start: 8, end: 9 }).unwrap();
    unsorted.push(Span { start: 1, end: 2 }).unwrap();
    assert!(!NormalizedMap::from_spans(&unsorted).is_sorted_and_disjoint());
}

#[test]
fn removing_an_empty_range_from_a_map_changes_nothing() {
    let map = normalize(&[usable(0x1000, 4 * PAGE_SIZE)]).unwrap();
    let same = map.without(PhysFrameRange::EMPTY).unwrap();
    assert_eq!(same.total_frames(), map.total_frames());
    assert!(same.is_sorted_and_disjoint());
}
