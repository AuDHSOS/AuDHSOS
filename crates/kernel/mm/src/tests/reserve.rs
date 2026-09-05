// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::reserve`, covering the reserve items of the catalog
//! 6.6.2.

#![allow(clippy::arithmetic_side_effects)]

use audhsos_abi::layout::PAGE_SIZE;
use kernel_hal_api::platform::{MemoryRegion, MemoryRegionKind};
use kernel_types::PhysAddr;

use crate::memory_map::{MapError, NormalizedMap, normalize};
use crate::reserve::{MAX_RESERVE_BYTES, MIN_RESERVE_BYTES, default_reserve_bytes, select_reserve};

const MIB: u64 = 1024 * 1024;

fn usable(start: u64, len: u64) -> MemoryRegion {
    MemoryRegion {
        start: PhysAddr::new(start).unwrap(),
        len,
        kind: MemoryRegionKind::Usable,
    }
}

fn map_of(regions: &[MemoryRegion]) -> NormalizedMap {
    normalize(regions).unwrap()
}

#[test]
fn the_default_size_is_a_sixteenth_clamped_to_four_and_sixty_four_mebibytes() {
    assert_eq!(default_reserve_bytes(0), MIN_RESERVE_BYTES);
    assert_eq!(default_reserve_bytes(16 * MIB), MIN_RESERVE_BYTES);
    assert_eq!(default_reserve_bytes(64 * MIB), MIN_RESERVE_BYTES);
    assert_eq!(default_reserve_bytes(256 * MIB), 16 * MIB);
    assert_eq!(default_reserve_bytes(1024 * MIB), MAX_RESERVE_BYTES);
    assert_eq!(default_reserve_bytes(u64::MAX), MAX_RESERVE_BYTES);
    assert_eq!(default_reserve_bytes(160 * MIB), 10 * MIB);
    assert!(default_reserve_bytes(100 * MIB + 8).is_multiple_of(PAGE_SIZE));
}

#[test]
fn the_reserve_is_carved_from_the_start_of_the_first_range_that_fits() {
    let map = map_of(&[usable(MIB, 4 * MIB), usable(64 * MIB, 128 * MIB)]);
    let expected = default_reserve_bytes(map.total_bytes());
    assert_eq!(expected, 132 * MIB / 16);
    let (reserve, rest) = select_reserve(&map, 0).unwrap();
    assert_eq!(
        reserve.start().start().as_u64(),
        64 * MIB,
        "the first range is too small, so the second one is used"
    );
    assert_eq!(reserve.bytes(), expected);
    assert_eq!(rest.total_bytes(), map.total_bytes() - expected);
    assert!(rest.is_sorted_and_disjoint());
    assert_eq!(rest.len(), 2);
    assert_eq!(
        rest.iter()
            .nth(1)
            .map(|range| range.start().start().as_u64()),
        Some(64 * MIB + expected)
    );
}

#[test]
fn a_range_that_holds_exactly_the_reserve_disappears_from_the_map() {
    let map = map_of(&[usable(16 * MIB, 4 * MIB)]);
    let (reserve, rest) = select_reserve(&map, 0).unwrap();
    assert_eq!(reserve.bytes(), MIN_RESERVE_BYTES);
    assert!(rest.is_empty());
    assert_eq!(rest.total_frames(), 0);
}

#[test]
fn usable_memory_smaller_than_the_reserve_is_an_error() {
    let map = map_of(&[usable(MIB, 2 * MIB)]);
    assert_eq!(
        select_reserve(&map, 0).err(),
        Some(MapError::ReserveDoesNotFit)
    );
    let scattered = map_of(&[usable(MIB, 2 * MIB), usable(8 * MIB, 3 * MIB)]);
    assert_eq!(
        select_reserve(&scattered, 0).err(),
        Some(MapError::ReserveDoesNotFit),
        "the reserve must fit into one range, not into the total"
    );
}

#[test]
fn an_override_replaces_the_default_size() {
    let map = map_of(&[usable(16 * MIB, 128 * MIB)]);
    let (reserve, rest) = select_reserve(&map, 2 * MIB).unwrap();
    assert_eq!(reserve.bytes(), 2 * MIB);
    assert_eq!(rest.total_bytes(), 126 * MIB);
    let (single, _) = select_reserve(&map, PAGE_SIZE).unwrap();
    assert_eq!(single.count(), 1);
}

#[test]
fn an_unaligned_override_is_rejected() {
    let map = map_of(&[usable(16 * MIB, 128 * MIB)]);
    assert_eq!(
        select_reserve(&map, PAGE_SIZE + 1).err(),
        Some(MapError::Address(kernel_types::Error::Unaligned {
            address: PAGE_SIZE + 1,
            alignment: PAGE_SIZE
        }))
    );
}

#[test]
fn an_override_not_below_the_usable_memory_is_rejected() {
    let map = map_of(&[usable(16 * MIB, 128 * MIB)]);
    assert_eq!(
        select_reserve(&map, 128 * MIB).err(),
        Some(MapError::Address(kernel_types::Error::Overflow))
    );
    assert_eq!(
        select_reserve(&map, 256 * MIB).err(),
        Some(MapError::Address(kernel_types::Error::Overflow))
    );
    assert!(select_reserve(&map, 128 * MIB - PAGE_SIZE).is_ok());
}

#[test]
fn a_reserve_smaller_than_one_frame_does_not_fit() {
    let map = map_of(&[usable(16 * MIB, 128 * MIB)]);
    assert_eq!(
        select_reserve(&map, PAGE_SIZE - 1).err(),
        Some(MapError::Address(kernel_types::Error::Unaligned {
            address: PAGE_SIZE - 1,
            alignment: PAGE_SIZE
        }))
    );
}

#[test]
fn the_reserve_is_removed_from_the_middle_of_the_map_without_disturbing_the_rest() {
    let map = map_of(&[
        usable(MIB, MIB),
        usable(16 * MIB, 128 * MIB),
        usable(512 * MIB, MIB),
    ]);
    let (reserve, rest) = select_reserve(&map, 8 * MIB).unwrap();
    assert_eq!(reserve.start().start().as_u64(), 16 * MIB);
    let starts: Vec<u64> = rest
        .iter()
        .map(|range| range.start().start().as_u64())
        .collect();
    assert_eq!(starts, vec![MIB, 24 * MIB, 512 * MIB]);
    assert!(rest.is_sorted_and_disjoint());
}
