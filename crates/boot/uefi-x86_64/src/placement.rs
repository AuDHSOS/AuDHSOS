// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Where the kernel image goes in physical memory and what each of its
//! segments may do.
//!
//! Invariants: the whole image lies in one physical range, so that the
//! boot information can report it as one; every segment keeps the offset
//! it has in the virtual image, so that a virtual address and a physical
//! address differ by one constant; the bytes a segment does not carry in
//! the file are zero.

use core::fmt;

use audhsos_abi::layout::{PAGE_SHIFT, PAGE_SIZE};
use audhsos_elf::image::{Image, MAX_SEGMENTS, Segment};
use audhsos_uefi::status::Status;
use kernel_mm::page_table::Permissions;
use kernel_types::{Alignment, PageRange, PhysFrameRange, VirtAddr};

use crate::firmware::Firmware;

/// One segment after it has been placed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlacedSegment {
    /// The pages the segment occupies.
    pub(crate) pages: PageRange,
    /// The frames the segment was copied into.
    pub(crate) frames: PhysFrameRange,
    /// What the mapping allows.
    pub(crate) perms: Permissions,
}

/// The kernel image in physical memory.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placement {
    /// The frames holding the whole image.
    pub(crate) frames: PhysFrameRange,
    /// The entry point, a virtual address.
    pub(crate) entry: u64,
    segments: [Option<PlacedSegment>; MAX_SEGMENTS],
    count: usize,
}

impl Placement {
    /// The placed segments, lowest virtual address first.
    pub(crate) fn segments(&self) -> impl Iterator<Item = PlacedSegment> + '_ {
        self.segments.iter().take(self.count).flatten().copied()
    }
}

/// Why an image could not be placed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlaceError {
    /// The image has no loadable segment.
    NoSegments,
    /// A segment does not start on a page boundary.
    Unaligned(u64),
    /// The image reaches beyond the addresses the loader can compute with.
    Overflow,
    /// The firmware could not supply the frames.
    OutOfMemory(Status),
}

impl fmt::Display for PlaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaceError::NoSegments => f.write_str("the kernel has no loadable segment"),
            PlaceError::Unaligned(vaddr) => {
                write!(f, "the segment at {vaddr:#x} is not page aligned")
            }
            PlaceError::Overflow => f.write_str("the kernel image does not fit"),
            PlaceError::OutOfMemory(status) => write!(f, "{status}"),
        }
    }
}

/// The page-aligned virtual range the whole image occupies, as start and
/// length.
pub(crate) fn span(image: &Image<'_>) -> Result<(u64, u64), PlaceError> {
    let mut lowest = u64::MAX;
    let mut highest = 0u64;
    let mut seen = false;
    for segment in image.segments() {
        let end = segment.end().ok_or(PlaceError::Overflow)?;
        lowest = lowest.min(segment.vaddr);
        highest = highest.max(end);
        seen = true;
    }
    if !seen {
        return Err(PlaceError::NoSegments);
    }
    let start = Alignment::PAGE.align_down(lowest);
    let end = Alignment::PAGE
        .align_up(highest)
        .ok_or(PlaceError::Overflow)?;
    let len = end.checked_sub(start).ok_or(PlaceError::Overflow)?;
    if len == 0 {
        return Err(PlaceError::NoSegments);
    }
    Ok((start, len))
}

/// What a segment may do once it is mapped.
pub(crate) const fn permissions(segment: Segment) -> Permissions {
    Permissions {
        write: segment.write,
        execute: segment.execute,
        user: false,
    }
}

/// Copies `image` into fresh frames and reports where every segment went.
///
/// # Errors
///
/// [`PlaceError`] for an image the loader cannot place and for every
/// firmware failure.
pub(crate) fn place(firmware: &Firmware<'_>, image: &Image<'_>) -> Result<Placement, PlaceError> {
    let (span_start, span_len) = span(image)?;
    let frames = firmware
        .allocate_pages(span_len.div_ceil(PAGE_SIZE))
        .map_err(PlaceError::OutOfMemory)?;
    // SAFETY: the frames were just allocated for the kernel image, nothing
    // else holds a reference to them, and the identity mapping is active.
    let bytes =
        unsafe { crate::memory::bytes_mut(frames, span_len) }.ok_or(PlaceError::Overflow)?;
    bytes.fill(0);
    let mut segments: [Option<PlacedSegment>; MAX_SEGMENTS] = [None; MAX_SEGMENTS];
    let mut count = 0usize;
    for segment in image.segments() {
        if !segment.vaddr.is_multiple_of(PAGE_SIZE) {
            return Err(PlaceError::Unaligned(segment.vaddr));
        }
        let offset = segment
            .vaddr
            .checked_sub(span_start)
            .ok_or(PlaceError::Overflow)?;
        copy_segment(bytes, offset, image.segment_bytes(segment))?;
        let placed = place_segment(segment, span_start, frames)?;
        let slot = segments.get_mut(count).ok_or(PlaceError::Overflow)?;
        *slot = Some(placed);
        count = count.saturating_add(1);
    }
    Ok(Placement {
        frames,
        entry: image.entry,
        segments,
        count,
    })
}

/// Copies the file content of one segment into the image buffer.
fn copy_segment(bytes: &mut [u8], offset: u64, content: &[u8]) -> Result<(), PlaceError> {
    if content.is_empty() {
        return Ok(());
    }
    let start = usize::try_from(offset).map_err(|_| PlaceError::Overflow)?;
    let end = start
        .checked_add(content.len())
        .ok_or(PlaceError::Overflow)?;
    let slot = bytes.get_mut(start..end).ok_or(PlaceError::Overflow)?;
    slot.copy_from_slice(content);
    Ok(())
}

/// The pages and the frames of one segment inside the placed image.
fn place_segment(
    segment: Segment,
    span_start: u64,
    frames: PhysFrameRange,
) -> Result<PlacedSegment, PlaceError> {
    let end = segment.end().ok_or(PlaceError::Overflow)?;
    let start = VirtAddr::new(segment.vaddr).map_err(|_| PlaceError::Overflow)?;
    let stop = VirtAddr::new(end).map_err(|_| PlaceError::Overflow)?;
    let pages = PageRange::from_addresses(start, stop).map_err(|_| PlaceError::Overflow)?;
    let offset = segment
        .vaddr
        .checked_sub(span_start)
        .ok_or(PlaceError::Overflow)?;
    let first = frames
        .start()
        .checked_add(offset >> PAGE_SHIFT)
        .ok_or(PlaceError::Overflow)?;
    let frames = PhysFrameRange::new(first, pages.count()).map_err(|_| PlaceError::Overflow)?;
    Ok(PlacedSegment {
        pages,
        frames,
        perms: permissions(segment),
    })
}
