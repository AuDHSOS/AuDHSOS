// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Generators for the memory management property tests. Available behind
//! the feature `test-strategies` and in this crate's own tests.

extern crate alloc;

use alloc::vec::Vec;

use audhsos_abi::layout::{PAGE_SIZE, USER_SPACE_START};
use kernel_hal_api::platform::{MemoryRegion, MemoryRegionKind};
use kernel_types::{Page, PageRange, PhysAddr, VirtAddr};
use test_support::generators::{BoxGen, Generator, bool, one_of, pair, range, vec};

use crate::page_table::Permissions;

/// Every region kind, in the order of the enumeration.
const KINDS: [MemoryRegionKind; 10] = [
    MemoryRegionKind::Usable,
    MemoryRegionKind::Reserved,
    MemoryRegionKind::AcpiReclaimable,
    MemoryRegionKind::AcpiNvs,
    MemoryRegionKind::MmioReserved,
    MemoryRegionKind::Kernel,
    MemoryRegionKind::BootImage,
    MemoryRegionKind::PageTables,
    MemoryRegionKind::BootStack,
    MemoryRegionKind::BootInfo,
];

/// Any single region below 16 MiB, with byte-granular, possibly unaligned
/// bounds and a zero length now and then.
#[must_use]
pub fn any_memory_region() -> BoxGen<MemoryRegion> {
    pair(
        pair(range(0u64..=(16 * 1024 * 1024)), range(0u64..=(1 << 20))),
        one_of(KINDS.to_vec()),
    )
    .map(|((start, len), kind)| MemoryRegion {
        start: PhysAddr::new(start).unwrap_or(PhysAddr::ZERO),
        len,
        kind,
    })
    .boxed()
}

/// Lists of zero to sixty-four regions, unsorted and possibly overlapping.
#[must_use]
pub fn any_memory_regions() -> BoxGen<Vec<MemoryRegion>> {
    vec(any_memory_region(), 0..=64).boxed()
}

/// A list of regions that always holds usable memory, so that
/// normalization can succeed.
#[must_use]
pub fn any_populated_memory_regions() -> BoxGen<Vec<MemoryRegion>> {
    pair(any_memory_regions(), range(1u64..=256))
        .map(|(mut regions, pages)| {
            regions.push(MemoryRegion {
                start: PhysAddr::new(32 * 1024 * 1024).unwrap_or(PhysAddr::ZERO),
                len: pages.saturating_mul(PAGE_SIZE),
                kind: MemoryRegionKind::Usable,
            });
            regions
        })
        .boxed()
}

/// Any permission set.
#[must_use]
pub fn any_permissions() -> BoxGen<Permissions> {
    pair(bool(), pair(bool(), bool()))
        .map(|(write, (execute, user))| Permissions {
            write,
            execute,
            user,
        })
        .boxed()
}

/// Any page range inside the mappable user half, at most sixteen pages
/// long, in the lowest gibibyte above [`USER_SPACE_START`].
#[must_use]
pub fn any_user_page_range() -> BoxGen<PageRange> {
    pair(range(0u64..=255), range(1u64..=16))
        .map(|(offset, count)| {
            let start = USER_SPACE_START.saturating_add(offset.saturating_mul(PAGE_SIZE));
            let page = VirtAddr::new(start)
                .ok()
                .and_then(|address| Page::from_start(address).ok())
                .unwrap_or_else(|| VirtAddr::ZERO.page());
            PageRange::new(page, count).unwrap_or_else(|_| fallback_range(page))
        })
        .boxed()
}

fn fallback_range(page: Page) -> PageRange {
    PageRange::new(page, 0).unwrap_or_else(|_| unreachable_range())
}

/// Never called: an empty range at any page is valid.
#[expect(
    clippy::panic,
    reason = "unreachable by construction; a count of zero always fits"
)]
fn unreachable_range<T>() -> T {
    panic!("an empty page range must be valid")
}

/// One operation of the region table model test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionOp {
    /// Insert a region covering these pages with these permissions.
    Insert(PageRange, Permissions),
    /// Remove these pages.
    Remove(PageRange),
    /// Change the permissions of these pages.
    Protect(PageRange, Permissions),
    /// Look up the first page of this range.
    Find(PageRange),
}

/// Any region table operation.
#[must_use]
pub fn any_region_table_op() -> BoxGen<RegionOp> {
    pair(
        range(0u32..=3),
        pair(any_user_page_range(), any_permissions()),
    )
    .map(|(choice, (pages, perms))| match choice {
        1 => RegionOp::Remove(pages),
        2 => RegionOp::Protect(pages, perms),
        3 => RegionOp::Find(pages),
        _ => RegionOp::Insert(pages, perms),
    })
    .boxed()
}
