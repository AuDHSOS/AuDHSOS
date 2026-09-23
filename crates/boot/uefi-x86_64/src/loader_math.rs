// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bounds used by the loader before allocating firmware pages.

use audhsos_abi::layout::PAGE_SIZE;
use audhsos_elf::image::Segment;
use kernel_types::Alignment;

/// Maximum size of the memory map allocation, in pages.
pub const MAX_MAP_BUFFER_PAGES: u64 = 256;

/// Keep a page of space for descriptors added after the first map read.
#[must_use]
pub fn map_buffer_pages(current: u64, map_size: usize) -> Option<u64> {
    let size = u64::try_from(map_size).ok()?;
    let required = size.div_ceil(PAGE_SIZE).checked_add(1)?;
    if required <= current {
        return Some(current);
    }
    let next = current.saturating_mul(2).max(required);
    (next <= MAX_MAP_BUFFER_PAGES).then_some(next)
}

/// One table per 2 MiB, 1 GiB, and 512 GiB crossed by a range.
fn tables_for_range(start: u64, bytes: u64) -> u64 {
    let Some(last) = start.checked_add(bytes).and_then(|end| end.checked_sub(1)) else {
        return u64::MAX;
    };
    [21, 30, 39].into_iter().fold(0u64, |total, shift| {
        total.saturating_add(
            (last >> shift)
                .saturating_sub(start >> shift)
                .saturating_add(1),
        )
    })
}

/// Page tables for the root, physical maps, image, stack, and information.
#[must_use]
pub fn pool_frames(highest: u64, image_start: u64, image_bytes: u64) -> u64 {
    let physical = tables_for_range(0, highest);
    let image = tables_for_range(image_start, image_bytes);
    // The stack and information page share a directory in PML4 slot 511.
    1u64.saturating_add(physical.saturating_mul(2))
        .saturating_add(image)
        .saturating_add(4)
}

/// Why the occupied image span cannot be computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanError {
    /// Every segment is empty.
    NoSegments,
    /// A segment or aligned span crosses the address limit.
    Overflow,
}

/// The page-aligned span occupied by nonempty loadable segments.
///
/// # Errors
///
/// [`SpanError::NoSegments`] for an empty image, or [`SpanError::Overflow`]
/// when the aligned span exceeds the address limit.
pub fn image_span(segments: impl Iterator<Item = Segment>) -> Result<(u64, u64), SpanError> {
    let mut lowest = u64::MAX;
    let mut highest = 0u64;
    for segment in segments.filter(|segment| segment.mem_size != 0) {
        let end = segment.end().ok_or(SpanError::Overflow)?;
        lowest = lowest.min(segment.vaddr);
        highest = highest.max(end);
    }
    if lowest == u64::MAX {
        return Err(SpanError::NoSegments);
    }
    let start = Alignment::PAGE.align_down(lowest);
    let end = Alignment::PAGE
        .align_up(highest)
        .ok_or(SpanError::Overflow)?;
    let len = end.checked_sub(start).ok_or(SpanError::Overflow)?;
    if len == 0 {
        return Err(SpanError::NoSegments);
    }
    Ok((start, len))
}
