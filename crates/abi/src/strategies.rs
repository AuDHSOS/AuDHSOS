// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Generators of the boot structures for property tests. Available behind
//! the feature `test-strategies` and in this crate's own tests.

extern crate alloc;

use alloc::vec::Vec;

use test_support::generators::{BoxGen, Generator, bool, bytes, one_of, pair, range, vec};

use crate::boot_image::{BOOT_IMAGE_HEADER_LEN, BootImageHeader};
use crate::boot_info::{BootRegion, BootRegionKind};
use crate::layout::PAGE_SIZE;

/// A boot image header whose root task and archive are page-aligned, start
/// behind the header, and do not overlap.
#[must_use]
pub fn any_boot_image_header() -> BoxGen<BootImageHeader> {
    pair(
        pair(range(1u64..=4), range(1u64..=16)),
        pair(range(0u64..=8), range(0u64..=4)),
    )
    .map(|((offset_pages, root_pages), (gap_pages, reserve_pages))| {
        let root_task_offset = offset_pages.saturating_mul(PAGE_SIZE);
        let root_task_len = root_pages.saturating_mul(PAGE_SIZE);
        let archive_offset = root_task_offset
            .saturating_add(root_task_len)
            .saturating_add(gap_pages.saturating_mul(PAGE_SIZE));
        BootImageHeader {
            root_task_offset,
            root_task_len,
            archive_offset,
            archive_len: gap_pages.saturating_mul(1024),
            kernel_reserve_size: reserve_pages.saturating_mul(PAGE_SIZE),
        }
    })
    .boxed()
}

/// The smallest image length a header is valid for.
#[must_use]
pub fn image_len_for(header: &BootImageHeader) -> u64 {
    let root_end = header.root_task_offset.saturating_add(header.root_task_len);
    let archive_end = header.archive_offset.saturating_add(header.archive_len);
    root_end.max(archive_end)
}

/// Sixty-four bytes that are a boot image header with up to four bytes
/// replaced, sometimes truncated, so that most cases are near-valid.
#[must_use]
pub fn any_boot_image_bytes() -> BoxGen<Vec<u8>> {
    let mutations = vec(pair(range(0usize..=63), range(0u8..=u8::MAX)), 0..=4);
    pair(any_boot_image_header(), pair(mutations, bool()))
        .map(|(header, (mutations, truncate))| {
            let mut bytes = header.to_bytes().to_vec();
            for (index, value) in mutations {
                if let Some(slot) = bytes.get_mut(index) {
                    *slot = value;
                }
            }
            if truncate {
                bytes.truncate(BOOT_IMAGE_HEADER_LEN.saturating_sub(1));
            }
            bytes
        })
        .boxed()
}

/// Any byte string of at most 256 bytes, for the boot information parser.
#[must_use]
pub fn any_boot_info_bytes() -> BoxGen<Vec<u8>> {
    bytes(0..=256).boxed()
}

/// Any region kind code, including unknown ones.
#[must_use]
pub fn any_region_kind_code() -> BoxGen<u32> {
    range(0u32..=8).boxed()
}

/// Any region with a page-aligned start and a non-zero length.
#[must_use]
pub fn any_boot_region() -> BoxGen<BootRegion> {
    pair(
        pair(range(0u64..=1024), range(1u64..=64)),
        one_of(BootRegionKind::ALL.to_vec()),
    )
    .map(|((start_pages, len_pages), kind)| {
        BootRegion::new(
            start_pages.saturating_mul(PAGE_SIZE),
            len_pages.saturating_mul(PAGE_SIZE),
            kind,
        )
    })
    .boxed()
}
