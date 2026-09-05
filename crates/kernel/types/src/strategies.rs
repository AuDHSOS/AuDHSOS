// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Generators of address types for property tests. Available behind the
//! feature `test-strategies` and in this crate's own tests.

use audhsos_abi::layout::{KERNEL_SPACE_START, PAGE_SHIFT, USER_SPACE_END};
use test_support::generators::{BoxGen, Generator, bool, one_of, pair, range};

use crate::phys::{MAX_FRAME_NUMBER, MAX_PHYS_ADDR};
use crate::{Alignment, Page, PageRange, PhysAddr, PhysFrame, PhysFrameRange, VirtAddr};

/// Any physical address, shrinking toward zero.
#[must_use]
pub fn any_phys_addr() -> BoxGen<PhysAddr> {
    range(0..=MAX_PHYS_ADDR)
        .map(|raw| PhysAddr::new(raw).unwrap_or(PhysAddr::ZERO))
        .boxed()
}

/// Any physical frame, shrinking toward frame zero.
#[must_use]
pub fn any_phys_frame() -> BoxGen<PhysFrame> {
    range(0..=MAX_FRAME_NUMBER)
        .map(|number| PhysFrame::from_number(number).unwrap_or(PhysAddr::ZERO.frame()))
        .boxed()
}

/// Any frame range of at most `2^20` frames, shrinking toward an empty
/// range at frame zero.
#[must_use]
pub fn any_phys_frame_range() -> BoxGen<PhysFrameRange> {
    pair(any_phys_frame(), range(0u64..=1 << 20))
        .map(|(start, count)| {
            let count = count.min(
                MAX_FRAME_NUMBER
                    .wrapping_add(1)
                    .saturating_sub(start.number()),
            );
            PhysFrameRange::new(start, count).unwrap_or(PhysFrameRange::EMPTY)
        })
        .boxed()
}

/// Any user-half virtual address, shrinking toward zero.
#[must_use]
pub fn any_user_virt_addr() -> BoxGen<VirtAddr> {
    range(0..=USER_SPACE_END.wrapping_sub(1))
        .map(|raw| VirtAddr::new(raw).unwrap_or(VirtAddr::ZERO))
        .boxed()
}

/// Any kernel-half virtual address, shrinking toward the lowest kernel
/// address.
#[must_use]
pub fn any_kernel_virt_addr() -> BoxGen<VirtAddr> {
    range(KERNEL_SPACE_START..=u64::MAX)
        .map(|raw| VirtAddr::new(raw).unwrap_or(VirtAddr::KERNEL_MIN))
        .boxed()
}

/// Any canonical virtual address; user addresses shrink toward zero, kernel
/// addresses toward the lowest kernel address, and the user half is
/// preferred when shrinking.
#[must_use]
pub fn any_virt_addr() -> BoxGen<VirtAddr> {
    pair(bool(), pair(any_user_virt_addr(), any_kernel_virt_addr()))
        .map(|(kernel, (user_addr, kernel_addr))| if kernel { kernel_addr } else { user_addr })
        .boxed()
}

/// Any page of either half.
#[must_use]
pub fn any_page() -> BoxGen<Page> {
    any_virt_addr().map(VirtAddr::page).boxed()
}

/// Any page range of at most `2^20` pages inside one half.
#[must_use]
pub fn any_page_range() -> BoxGen<PageRange> {
    pair(any_page(), range(0u64..=1 << 20))
        .map(|(start, count)| {
            let limit = if start.is_user() {
                USER_SPACE_END >> PAGE_SHIFT
            } else {
                1 << (64 - PAGE_SHIFT)
            };
            let count = count.min(limit.saturating_sub(start.number()));
            PageRange::new(start, count).unwrap_or_else(|_| {
                PageRange::new(start, 0).unwrap_or_else(|_| unreachable_page_range())
            })
        })
        .boxed()
}

/// Never called: an empty range at any page is always valid.
#[expect(
    clippy::panic,
    reason = "unreachable by construction; a count of zero always fits"
)]
fn unreachable_page_range<T>() -> T {
    panic!("an empty page range must be valid")
}

/// Any alignment from one byte to one gibibyte, shrinking toward one byte.
#[must_use]
pub fn any_alignment() -> BoxGen<Alignment> {
    one_of(
        (0..=30u32)
            .map(|shift| Alignment::new(1 << shift).unwrap_or(Alignment::BYTE))
            .collect(),
    )
    .boxed()
}
