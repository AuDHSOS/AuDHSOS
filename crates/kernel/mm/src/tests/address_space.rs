// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::address_space`, covering the catalog items 6.6.5.

#![allow(clippy::arithmetic_side_effects)]

use audhsos_abi::layout::{KERNEL_SPACE_START, PAGE_SIZE, USER_SPACE_END, USER_SPACE_START};
use kernel_types::{Page, PageRange, VirtAddr};
use test_support::generators::vec;
use test_support::property::check;

use crate::address_space::{
    Inserted, Region, RegionError, RegionTable, remaining_from, user_range,
};
use crate::page_table::Permissions;
use crate::strategies::{RegionOp, any_region_table_op};

/// Backing objects are named by a number in these tests.
type Table<const N: usize> = RegionTable<u32, N>;

fn page(address: u64) -> Page {
    VirtAddr::new(address).unwrap().page()
}

fn pages(address: u64, count: u64) -> PageRange {
    PageRange::new(page(address), count).unwrap()
}

fn region(address: u64, count: u64, backing: u32) -> Region<u32> {
    Region {
        pages: pages(address, count),
        backing,
        offset: 0,
        perms: Permissions::READ_WRITE.for_user(),
    }
}

/// The regions of a table as `(start page, count, backing, offset)`.
fn layout<const N: usize>(table: &Table<N>) -> Vec<(u64, u64, u32, u64)> {
    table
        .iter()
        .map(|region| {
            (
                region.pages.start().number(),
                region.pages.count(),
                region.backing,
                region.offset,
            )
        })
        .collect()
}

const BASE: u64 = USER_SPACE_START;

#[test]
fn a_new_table_is_empty_and_reports_its_free_slots() {
    let table: Table<4> = Table::new();
    assert!(table.is_empty());
    assert_eq!(table.len(), 0);
    assert_eq!(table.free_slots(), 4);
    assert!(table.check_invariants());
    assert_eq!(table.find(page(BASE)), None);
    let default: Table<4> = Table::default();
    assert!(default.is_empty());
}

#[test]
fn regions_are_kept_sorted_no_matter_in_which_order_they_arrive() {
    let mut table: Table<8> = Table::new();
    for (offset, backing) in [(8u64, 3u32), (0, 1), (4, 2)] {
        table
            .insert(region(BASE + offset * PAGE_SIZE, 2, backing))
            .unwrap();
    }
    assert_eq!(
        layout(&table)
            .iter()
            .map(|entry| entry.2)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(table.check_invariants());
    assert_eq!(table.find(page(BASE)).map(|r| r.backing), Some(1));
    assert_eq!(
        table.find(page(BASE + 5 * PAGE_SIZE)).map(|r| r.backing),
        Some(2)
    );
    assert_eq!(table.find(page(BASE + 3 * PAGE_SIZE)), None);
}

#[test]
fn an_overlap_at_every_position_is_reported_as_address_in_use() {
    let mut table: Table<8> = Table::new();
    table.insert(region(BASE + 4 * PAGE_SIZE, 4, 1)).unwrap();
    let overlaps = [
        (BASE + 2 * PAGE_SIZE, 4u64, "at the start"),
        (BASE + 6 * PAGE_SIZE, 4, "at the end"),
        (BASE + 5 * PAGE_SIZE, 2, "inside"),
        (BASE, 16, "covering it"),
        (BASE + 4 * PAGE_SIZE, 4, "exactly"),
    ];
    for (start, count, what) in overlaps {
        assert_eq!(
            table.insert(region(start, count, 2)).err(),
            Some(RegionError::AddressInUse),
            "an overlap {what} must be rejected"
        );
    }
    assert_eq!(table.len(), 1);
}

#[test]
fn two_regions_that_touch_are_allowed_and_stay_separate() {
    let mut table: Table<8> = Table::new();
    table.insert(region(BASE, 2, 1)).unwrap();
    table.insert(region(BASE + 2 * PAGE_SIZE, 2, 2)).unwrap();
    assert_eq!(table.len(), 2);
    assert!(table.check_invariants());
    assert_eq!(
        layout(&table),
        vec![(BASE / PAGE_SIZE, 2, 1, 0), (BASE / PAGE_SIZE + 2, 2, 2, 0)]
    );
}

#[test]
fn every_malformed_request_has_its_own_error() {
    assert_eq!(user_range(BASE, 0).err(), Some(RegionError::Empty));
    assert_eq!(
        user_range(BASE + 1, PAGE_SIZE).err(),
        Some(RegionError::Unaligned),
        "an unaligned start"
    );
    assert_eq!(
        user_range(BASE, PAGE_SIZE + 1).err(),
        Some(RegionError::Unaligned),
        "an unaligned length"
    );
    assert_eq!(
        user_range(USER_SPACE_END - PAGE_SIZE, 2 * PAGE_SIZE).err(),
        Some(RegionError::Overflow),
        "the range would leave the user half"
    );
    assert_eq!(
        user_range(0, PAGE_SIZE).err(),
        Some(RegionError::OutsideUserSpace),
        "the forbidden low 64 KiB"
    );
    assert_eq!(
        user_range(USER_SPACE_START - PAGE_SIZE, PAGE_SIZE).err(),
        Some(RegionError::OutsideUserSpace)
    );
    assert_eq!(
        user_range(KERNEL_SPACE_START, PAGE_SIZE).err(),
        Some(RegionError::OutsideUserSpace),
        "the kernel half"
    );
    assert_eq!(
        user_range(USER_SPACE_END, PAGE_SIZE).err(),
        Some(RegionError::OutsideUserSpace),
        "a non-canonical address"
    );
    assert_eq!(user_range(BASE, 2 * PAGE_SIZE), Ok(pages(BASE, 2)));
}

#[test]
fn a_user_table_refuses_ranges_outside_the_mappable_user_half() {
    let mut table: Table<4> = Table::new();
    assert_eq!(
        table.insert(region(PAGE_SIZE, 1, 1)).err(),
        Some(RegionError::OutsideUserSpace)
    );
    assert_eq!(
        table.insert(region(KERNEL_SPACE_START, 1, 1)).err(),
        Some(RegionError::OutsideUserSpace)
    );
    let empty = Region {
        pages: PageRange::new(page(BASE), 0).unwrap(),
        ..region(BASE, 1, 1)
    };
    assert_eq!(table.insert(empty).err(), Some(RegionError::Empty));
    assert!(table.is_empty());
}

#[test]
fn a_kernel_table_accepts_the_kernel_half() {
    let mut table: Table<4> = Table::kernel();
    table.insert(region(KERNEL_SPACE_START, 2, 1)).unwrap();
    table.insert(region(PAGE_SIZE, 1, 2)).unwrap();
    assert_eq!(table.len(), 2);
    assert!(table.check_invariants());
    assert_eq!(
        table.find(page(KERNEL_SPACE_START)).map(|r| r.backing),
        Some(1)
    );
}

#[test]
fn unmapping_a_whole_region_removes_it() {
    let mut table: Table<8> = Table::new();
    table.insert(region(BASE, 4, 1)).unwrap();
    table.insert(region(BASE + 8 * PAGE_SIZE, 4, 2)).unwrap();
    let removed = table.remove::<3>(pages(BASE, 4)).unwrap();
    assert_eq!(removed.len(), 1);
    assert!(!removed.is_empty());
    assert_eq!(removed.remaining(), None);
    assert_eq!(
        removed.iter().next().map(|r| (r.backing, r.pages.count())),
        Some((1, 4))
    );
    assert_eq!(table.len(), 1);
    assert!(table.check_invariants());
}

#[test]
fn unmapping_the_middle_of_a_region_splits_it_and_moves_the_offset() {
    let mut table: Table<8> = Table::new();
    table.insert(region(BASE, 8, 1)).unwrap();
    let removed = table.remove::<3>(pages(BASE + 2 * PAGE_SIZE, 3)).unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(
        removed.iter().next().map(|r| (r.pages.count(), r.offset)),
        Some((3, 2 * PAGE_SIZE))
    );
    assert_eq!(
        layout(&table),
        vec![
            (BASE / PAGE_SIZE, 2, 1, 0),
            (BASE / PAGE_SIZE + 5, 3, 1, 5 * PAGE_SIZE)
        ]
    );
    assert!(table.check_invariants());
}

#[test]
fn unmapping_the_head_or_the_tail_of_a_region_keeps_the_rest() {
    let mut head: Table<8> = Table::new();
    head.insert(region(BASE, 8, 1)).unwrap();
    head.remove::<3>(pages(BASE, 3)).unwrap();
    assert_eq!(
        layout(&head),
        vec![(BASE / PAGE_SIZE + 3, 5, 1, 3 * PAGE_SIZE)]
    );

    let mut tail: Table<8> = Table::new();
    tail.insert(region(BASE, 8, 1)).unwrap();
    tail.remove::<3>(pages(BASE + 5 * PAGE_SIZE, 3)).unwrap();
    assert_eq!(layout(&tail), vec![(BASE / PAGE_SIZE, 5, 1, 0)]);
}

#[test]
fn unmapping_across_two_regions_removes_the_overlapping_parts() {
    let mut table: Table<8> = Table::new();
    table.insert(region(BASE, 4, 1)).unwrap();
    table.insert(region(BASE + 4 * PAGE_SIZE, 4, 2)).unwrap();
    let removed = table.remove::<3>(pages(BASE + 2 * PAGE_SIZE, 4)).unwrap();
    assert_eq!(removed.len(), 2);
    assert_eq!(
        removed
            .iter()
            .map(|r| (r.backing, r.pages.count()))
            .collect::<Vec<_>>(),
        vec![(1, 2), (2, 2)]
    );
    assert_eq!(
        layout(&table),
        vec![
            (BASE / PAGE_SIZE, 2, 1, 0),
            (BASE / PAGE_SIZE + 6, 2, 2, 2 * PAGE_SIZE)
        ]
    );
    assert!(table.check_invariants());
}

#[test]
fn unmapping_more_pieces_than_fit_reports_what_is_left() {
    let mut table: Table<8> = Table::new();
    for index in 0..4u64 {
        table
            .insert(region(
                BASE + index * 2 * PAGE_SIZE,
                2,
                u32::try_from(index).unwrap(),
            ))
            .unwrap();
    }
    let removed = table.remove::<2>(pages(BASE, 8)).unwrap();
    assert_eq!(removed.len(), 2);
    let remaining = removed.remaining().unwrap();
    assert_eq!(remaining.start().number(), BASE / PAGE_SIZE + 4);
    assert_eq!(remaining.count(), 4);
    assert_eq!(table.len(), 2);
    let rest = table.remove::<2>(remaining).unwrap();
    assert_eq!(rest.len(), 2);
    assert_eq!(rest.remaining(), None);
    assert!(table.is_empty());
}

#[test]
fn unmapping_a_range_without_a_region_is_reported() {
    let mut table: Table<8> = Table::new();
    table.insert(region(BASE + 8 * PAGE_SIZE, 2, 1)).unwrap();
    assert_eq!(
        table.remove::<3>(pages(BASE, 4)).err(),
        Some(RegionError::NotMapped)
    );
    assert_eq!(
        table.remove::<3>(pages(BASE, 0)).err(),
        Some(RegionError::Empty)
    );
    assert_eq!(
        table.remove::<3>(pages(KERNEL_SPACE_START, 1)).err(),
        Some(RegionError::OutsideUserSpace)
    );
    assert_eq!(table.len(), 1);
}

#[test]
fn protecting_a_sub_range_splits_a_region_into_three() {
    let mut table: Table<8> = Table::new();
    table.insert(region(BASE, 8, 1)).unwrap();
    let read_only = Permissions::READ_ONLY.for_user();
    assert_eq!(
        table.protect(pages(BASE + 2 * PAGE_SIZE, 3), read_only),
        Ok(2),
        "the head and the tail are regions the table did not have"
    );
    assert_eq!(table.len(), 3);
    let perms: Vec<Permissions> = table.iter().map(|r| r.perms).collect();
    assert_eq!(
        perms,
        vec![
            Permissions::READ_WRITE.for_user(),
            read_only,
            Permissions::READ_WRITE.for_user()
        ]
    );
    assert_eq!(
        layout(&table),
        vec![
            (BASE / PAGE_SIZE, 2, 1, 0),
            (BASE / PAGE_SIZE + 2, 3, 1, 2 * PAGE_SIZE),
            (BASE / PAGE_SIZE + 5, 3, 1, 5 * PAGE_SIZE)
        ]
    );
    assert!(table.check_invariants());
}

#[test]
fn protecting_a_head_a_tail_or_a_whole_region_needs_fewer_slots() {
    let read_only = Permissions::READ_ONLY.for_user();

    let mut head: Table<8> = Table::new();
    head.insert(region(BASE, 8, 1)).unwrap();
    head.protect(pages(BASE, 3), read_only).unwrap();
    assert_eq!(head.len(), 2);
    assert_eq!(head.iter().next().map(|r| r.perms), Some(read_only));

    let mut tail: Table<8> = Table::new();
    tail.insert(region(BASE, 8, 1)).unwrap();
    tail.protect(pages(BASE + 5 * PAGE_SIZE, 3), read_only)
        .unwrap();
    assert_eq!(tail.len(), 2);
    assert_eq!(tail.iter().nth(1).map(|r| r.perms), Some(read_only));

    let mut whole: Table<8> = Table::new();
    whole.insert(region(BASE, 8, 1)).unwrap();
    whole.protect(pages(BASE, 8), read_only).unwrap();
    assert_eq!(whole.len(), 1);
    assert_eq!(whole.iter().next().map(|r| r.perms), Some(read_only));
}

#[test]
fn protecting_a_range_that_no_single_region_covers_is_reported() {
    let mut table: Table<8> = Table::new();
    table.insert(region(BASE, 2, 1)).unwrap();
    table.insert(region(BASE + 2 * PAGE_SIZE, 2, 2)).unwrap();
    let read_only = Permissions::READ_ONLY.for_user();
    assert_eq!(
        table.protect(pages(BASE, 4), read_only).err(),
        Some(RegionError::NotMapped),
        "the range spans two regions"
    );
    assert_eq!(
        table
            .protect(pages(BASE + 8 * PAGE_SIZE, 1), read_only)
            .err(),
        Some(RegionError::NotMapped)
    );
    assert_eq!(
        table.protect(pages(BASE, 0), read_only).err(),
        Some(RegionError::Empty)
    );
    assert_eq!(table.len(), 2);
}

#[test]
fn the_table_takes_exactly_its_capacity_and_not_one_more() {
    let mut table: Table<3> = Table::new();
    for index in 0..3u64 {
        table
            .insert(region(
                BASE + index * 2 * PAGE_SIZE,
                1,
                u32::try_from(index).unwrap(),
            ))
            .unwrap();
    }
    assert_eq!(table.free_slots(), 0);
    assert_eq!(
        table.insert(region(BASE + 8 * PAGE_SIZE, 1, 9)).err(),
        Some(RegionError::QuotaExceeded)
    );
    assert_eq!(table.len(), 3);
}

#[test]
fn a_split_that_would_exceed_the_capacity_fails_before_any_change() {
    let mut table: Table<2> = Table::new();
    table.insert(region(BASE, 8, 1)).unwrap();
    table.insert(region(BASE + 16 * PAGE_SIZE, 1, 2)).unwrap();
    let before = layout(&table);
    assert_eq!(
        table.remove::<3>(pages(BASE + 2 * PAGE_SIZE, 2)).err(),
        Some(RegionError::QuotaExceeded)
    );
    assert_eq!(layout(&table), before, "the failed split changed nothing");
    assert_eq!(
        table
            .protect(
                pages(BASE + 2 * PAGE_SIZE, 2),
                Permissions::READ_ONLY.for_user()
            )
            .err(),
        Some(RegionError::QuotaExceeded)
    );
    assert_eq!(layout(&table), before);
}

#[test]
fn the_address_space_can_be_filled_with_maximal_regions() {
    let mut table: Table<4> = Table::new();
    let span = 1u64 << 30;
    for index in 0..4u64 {
        let start = BASE + index * span;
        table
            .insert(region(
                start,
                span / PAGE_SIZE - 1,
                u32::try_from(index).unwrap(),
            ))
            .unwrap();
    }
    assert_eq!(table.len(), 4);
    assert!(table.check_invariants());
    assert_eq!(
        table.insert(region(BASE + 4 * span, 1, 9)).err(),
        Some(RegionError::QuotaExceeded)
    );
}

#[test]
fn errors_render_a_message_and_map_to_the_abi() {
    let cases = [
        (RegionError::Empty, audhsos_abi::Error::InvalidArgument),
        (RegionError::Unaligned, audhsos_abi::Error::Unaligned),
        (RegionError::Overflow, audhsos_abi::Error::InvalidArgument),
        (
            RegionError::OutsideUserSpace,
            audhsos_abi::Error::InvalidArgument,
        ),
        (RegionError::AddressInUse, audhsos_abi::Error::AddressInUse),
        (RegionError::NotMapped, audhsos_abi::Error::NotMapped),
        (
            RegionError::QuotaExceeded,
            audhsos_abi::Error::QuotaExceeded,
        ),
    ];
    for (error, expected) in cases {
        assert!(!format!("{error}").is_empty());
        assert_eq!(audhsos_abi::Error::from(error), expected);
    }
}

#[test]
fn property_the_table_stays_sorted_disjoint_and_inside_the_user_half() {
    check(
        "region_table_invariants",
        &vec(any_region_table_op(), 0..=48),
        |ops| {
            let mut table: Table<16> = Table::new();
            let mut backing = 0u32;
            for op in ops {
                match *op {
                    RegionOp::Insert(pages, perms) => {
                        backing = backing.wrapping_add(1);
                        let _ = table.insert(Region {
                            pages,
                            backing,
                            offset: 0,
                            perms,
                        });
                    }
                    RegionOp::Remove(pages) => {
                        let _ = table.remove::<3>(pages);
                    }
                    RegionOp::Protect(pages, perms) => {
                        let _ = table.protect(pages, perms);
                    }
                    RegionOp::Find(pages) => {
                        let found = table.find(pages.start());
                        if let Some(region) = found
                            && !region.pages.contains(pages.start())
                        {
                            return Err("find returned a region without the page".to_owned());
                        }
                    }
                }
                if !table.check_invariants() {
                    return Err(format!("invariants broken after {op:?}"));
                }
                if table.len() > 16 {
                    return Err("more regions than slots".to_owned());
                }
            }
            Ok(())
        },
    );
}

#[test]
fn the_slot_helpers_ignore_indices_outside_the_array() {
    let mut table: Table<2> = Table::new();
    assert_eq!(table.get(0), None, "an unused slot holds nothing");
    table.put(99, Some(region(BASE, 1, 7)));
    assert!(table.is_empty());
    table.remove_at(99);
    assert!(table.is_empty());
    assert_eq!(
        table.insert_at(5, region(BASE, 1, 7)).err(),
        Some(RegionError::QuotaExceeded),
        "an index above the used part is refused"
    );
    table.insert(region(BASE, 1, 7)).unwrap();
    assert_eq!(table.get(0).map(|r| r.backing), Some(7));
    assert_eq!(table.get(1), None);
    assert_eq!(table.get(99), None);
    table.put(99, None);
    assert_eq!(table.len(), 1);
    table.remove_at(0);
    assert!(table.is_empty());
    assert_eq!(table.get(0), None, "the vacated slot was cleared");
}

#[test]
fn clipping_a_region_against_a_disjoint_range_yields_nothing() {
    let subject = region(BASE + 4 * PAGE_SIZE, 4, 1);
    assert_eq!(subject.clipped(pages(BASE, 2)), None, "entirely below");
    assert_eq!(
        subject.clipped(pages(BASE + 16 * PAGE_SIZE, 2)),
        None,
        "entirely above"
    );
    let inside = subject.clipped(pages(BASE + 5 * PAGE_SIZE, 2)).unwrap();
    assert_eq!(inside.pages.count(), 2);
    assert_eq!(inside.offset, PAGE_SIZE);
    assert_eq!(subject.head(subject.pages.start()), None);
    assert_eq!(
        subject
            .tail(subject.pages.start().checked_add(4).unwrap())
            .map(|r| r.pages.count()),
        None,
        "a tail behind the region is empty"
    );
}

#[test]
fn the_remainder_of_a_request_is_empty_once_it_is_finished() {
    let request = pages(BASE, 4);
    assert_eq!(
        remaining_from(page(BASE + 2 * PAGE_SIZE), request).map(PageRange::count),
        Some(2)
    );
    assert_eq!(
        remaining_from(page(BASE + 4 * PAGE_SIZE), request),
        None,
        "nothing is left at the end of the request"
    );
    assert_eq!(
        remaining_from(page(BASE + 8 * PAGE_SIZE), request),
        None,
        "a page behind the request leaves nothing"
    );
}

#[test]
fn a_table_with_an_empty_region_fails_its_own_invariant_check() {
    let mut table: Table<2> = Table::kernel();
    let empty = Region {
        pages: PageRange::new(page(BASE), 0).unwrap(),
        backing: 1,
        offset: 0,
        perms: Permissions::READ_WRITE,
    };
    table.insert(region(BASE, 1, 1)).unwrap();
    table.put(0, Some(empty));
    assert!(!table.check_invariants(), "an empty region is not allowed");

    let mut overlapping: Table<2> = Table::kernel();
    overlapping.insert(region(BASE, 4, 1)).unwrap();
    overlapping
        .insert(region(BASE + 8 * PAGE_SIZE, 4, 2))
        .unwrap();
    overlapping.put(1, Some(region(BASE + 2 * PAGE_SIZE, 4, 2)));
    assert!(!overlapping.check_invariants(), "the regions now overlap");

    let mut outside: Table<2> = Table::new();
    outside.insert(region(BASE, 1, 1)).unwrap();
    assert!(outside.check_invariants());
    outside.put(0, Some(region(KERNEL_SPACE_START, 1, 1)));
    assert!(
        !outside.check_invariants(),
        "a user table must stay in the user half"
    );
}

/// A region that names `backing` at `offset` inside it.
fn window(address: u64, count: u64, backing: u32, offset: u64) -> Region<u32> {
    Region {
        pages: pages(address, count),
        backing,
        offset,
        perms: Permissions::READ_WRITE.for_user(),
    }
}

#[test]
fn a_mapping_that_continues_one_that_is_there_grows_it_instead() {
    let mut table: Table<8> = Table::new();
    assert_eq!(table.insert(window(BASE, 4, 1, 0)), Ok(Inserted::Added));
    assert_eq!(
        table.insert(window(BASE + 4 * PAGE_SIZE, 4, 1, 4 * PAGE_SIZE)),
        Ok(Inserted::Merged)
    );
    assert_eq!(table.len(), 1);
    assert_eq!(layout(&table), vec![(BASE / PAGE_SIZE, 8, 1, 0)]);
    assert!(table.check_invariants());
    assert_eq!(
        table.insert(window(BASE + 8 * PAGE_SIZE, 2, 1, 8 * PAGE_SIZE)),
        Ok(Inserted::Merged)
    );
    assert_eq!(table.len(), 1);
    assert_eq!(
        table
            .find(page(BASE + 9 * PAGE_SIZE))
            .map(|held| held.offset),
        Some(0)
    );
}

#[test]
fn a_range_that_continues_nothing_is_a_region_of_its_own() {
    let base = window(BASE, 4, 1, 0);
    let cases = [
        (
            window(BASE + 4 * PAGE_SIZE, 2, 2, 4 * PAGE_SIZE),
            "another object",
        ),
        (
            window(BASE + 4 * PAGE_SIZE, 2, 1, 8 * PAGE_SIZE),
            "an offset that does not continue",
        ),
        (
            Region {
                perms: Permissions::READ_ONLY.for_user(),
                ..window(BASE + 4 * PAGE_SIZE, 2, 1, 4 * PAGE_SIZE)
            },
            "other permissions",
        ),
        (
            window(BASE + 8 * PAGE_SIZE, 2, 1, 8 * PAGE_SIZE),
            "a gap between them",
        ),
    ];
    for (next, why) in cases {
        let mut table: Table<8> = Table::new();
        table.insert(base).unwrap();
        assert_eq!(table.insert(next), Ok(Inserted::Added), "{why}");
        assert_eq!(table.len(), 2, "{why}");
    }
}

#[test]
fn a_removal_says_whether_the_region_it_took_from_is_gone() {
    let mut table: Table<8> = Table::new();
    table.insert(window(BASE, 8, 1, 0)).unwrap();
    let removed = table.remove::<2>(pages(BASE, 2)).unwrap();
    let taken: Vec<bool> = removed.taken().map(|piece| piece.vanished).collect();
    assert_eq!(taken, vec![false], "the region only got shorter");
    assert_eq!(table.len(), 1);

    let removed = table.remove::<2>(pages(BASE + 4 * PAGE_SIZE, 2)).unwrap();
    assert_eq!(
        removed
            .taken()
            .map(|piece| piece.vanished)
            .collect::<Vec<_>>(),
        vec![false],
        "the region was split in two, so it is still there"
    );
    assert_eq!(table.len(), 2);

    let rest: Vec<PageRange> = table.iter().map(|held| held.pages).collect();
    let mut gone = Vec::new();
    for range in rest {
        let removed = table.remove::<2>(range).unwrap();
        gone.extend(removed.taken().map(|piece| piece.vanished));
    }
    assert_eq!(gone, vec![true, true]);
    assert!(table.is_empty());
}

#[test]
fn what_a_removal_took_is_the_piece_it_reports() {
    let mut table: Table<8> = Table::new();
    table.insert(window(BASE, 4, 7, 0)).unwrap();
    let removed = table.remove::<2>(pages(BASE, 4)).unwrap();
    let piece = removed.taken().next().unwrap();
    assert!(piece.vanished);
    assert_eq!(piece.region.backing, 7);
    assert_eq!(piece.region.pages.count(), 4);
    assert_eq!(removed.iter().count(), 1);
}

#[test]
fn a_protection_says_how_many_regions_the_table_gained() {
    let read_only = Permissions::READ_ONLY.for_user();
    let mut table: Table<8> = Table::new();
    table.insert(region(BASE, 4, 1)).unwrap();
    assert_eq!(table.protect(pages(BASE, 4), read_only), Ok(0), "all of it");
    assert_eq!(table.len(), 1);

    let mut table: Table<8> = Table::new();
    table.insert(region(BASE, 4, 1)).unwrap();
    assert_eq!(
        table.protect(pages(BASE, 2), read_only),
        Ok(1),
        "at the front of it"
    );
    assert_eq!(table.len(), 2);

    let mut table: Table<8> = Table::new();
    table.insert(region(BASE, 4, 1)).unwrap();
    assert_eq!(
        table.protect(pages(BASE + 2 * PAGE_SIZE, 2), read_only),
        Ok(1),
        "at the end of it"
    );
    assert_eq!(table.len(), 2);
}
