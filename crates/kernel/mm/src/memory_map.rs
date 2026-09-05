// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Turning the byte-granular, unsorted, possibly overlapping region list of
//! the firmware into a sorted, disjoint, frame-aligned list of usable
//! ranges.
//!
//! Invariants: a [`NormalizedMap`] holds at most [`MAX_BOOT_REGIONS`]
//! ranges; the ranges are sorted by start, non-empty, and pairwise disjoint
//! with a gap of at least one frame between them.

use audhsos_abi::layout::{MAX_BOOT_REGIONS, PAGE_SHIFT, PAGE_SIZE};
use kernel_hal_api::platform::{MemoryRegion, MemoryRegionKind};
use kernel_types::{PhysFrameRange, phys::MAX_FRAME_NUMBER};

/// Why a memory map could not be normalized.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MapError {
    /// More ranges than [`MAX_BOOT_REGIONS`] were needed at some step.
    TooManyRegions,
    /// Nothing usable remained.
    NoUsableMemory,
    /// No usable range is large enough for the kernel reserve.
    ReserveDoesNotFit,
    /// An address or a length is not representable.
    Address(kernel_types::Error),
}

impl core::fmt::Display for MapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MapError::TooManyRegions => f.write_str("the memory map needs too many ranges"),
            MapError::NoUsableMemory => f.write_str("no usable memory remains"),
            MapError::ReserveDoesNotFit => f.write_str("no usable range holds the kernel reserve"),
            MapError::Address(error) => write!(f, "{error}"),
        }
    }
}

/// A half-open range of frame numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Span {
    /// First frame number.
    pub(crate) start: u64,
    /// One past the last frame number.
    pub(crate) end: u64,
}

impl Span {
    /// `true` if the span holds no frame.
    pub(crate) const fn is_empty(self) -> bool {
        self.end <= self.start
    }

    /// The number of frames.
    pub(crate) const fn frames(self) -> u64 {
        self.end.saturating_sub(self.start)
    }
}

/// A fixed-capacity, ordered list of spans.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SpanList {
    items: [Span; MAX_BOOT_REGIONS],
    len: usize,
}

impl Default for SpanList {
    fn default() -> Self {
        SpanList {
            items: [Span { start: 0, end: 0 }; MAX_BOOT_REGIONS],
            len: 0,
        }
    }
}

impl SpanList {
    /// The spans in list order.
    pub(crate) fn as_slice(&self) -> &[Span] {
        self.items.get(..self.len).unwrap_or(&[])
    }

    /// The number of spans.
    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    /// The span in slot `index`, or `None` outside the used part.
    pub(crate) fn get(&self, index: usize) -> Option<Span> {
        self.items.get(index).copied().filter(|_| index < self.len)
    }

    /// Writes `span` into slot `index`; an index outside the array changes
    /// nothing.
    pub(crate) fn put(&mut self, index: usize, span: Span) {
        if let Some(slot) = self.items.get_mut(index) {
            *slot = span;
        }
    }

    /// Appends `span` unless the list is full.
    pub(crate) fn push(&mut self, span: Span) -> Result<(), MapError> {
        let slot = self
            .items
            .get_mut(self.len)
            .ok_or(MapError::TooManyRegions)?;
        *slot = span;
        self.len = self.len.saturating_add(1);
        Ok(())
    }

    /// Inserts `span` at `index`, moving the rest up.
    pub(crate) fn insert(&mut self, index: usize, span: Span) -> Result<(), MapError> {
        if self.len >= MAX_BOOT_REGIONS || index > self.len {
            return Err(MapError::TooManyRegions);
        }
        let mut position = self.len;
        while position > index {
            let previous = self.get(position.saturating_sub(1)).unwrap_or_default();
            self.put(position, previous);
            position = position.saturating_sub(1);
        }
        self.put(index, span);
        self.len = self.len.saturating_add(1);
        Ok(())
    }

    /// Removes the span at `index`, moving the rest down.
    pub(crate) fn remove(&mut self, index: usize) {
        if index >= self.len {
            return;
        }
        let mut position = index;
        while position.saturating_add(1) < self.len {
            let next = self.get(position.saturating_add(1)).unwrap_or_default();
            self.put(position, next);
            position = position.saturating_add(1);
        }
        self.len = self.len.saturating_sub(1);
    }

    /// Sorts by start and merges spans that overlap or touch, dropping the
    /// empty ones.
    pub(crate) fn sort_and_merge(&mut self) {
        let mut index = 1;
        while index < self.len {
            let mut position = index;
            while position > 0 {
                let previous = position.saturating_sub(1);
                match (self.get(previous), self.get(position)) {
                    (Some(left), Some(right)) if left.start > right.start => {
                        self.put(previous, right);
                        self.put(position, left);
                    }
                    _ => break,
                }
                position = previous;
            }
            index = index.saturating_add(1);
        }
        let mut index = 0;
        while let Some(current) = self.get(index) {
            if current.is_empty() {
                self.remove(index);
                continue;
            }
            match self.get(index.saturating_add(1)) {
                Some(next) if next.start <= current.end => {
                    self.put(
                        index,
                        Span {
                            start: current.start,
                            end: current.end.max(next.end),
                        },
                    );
                    self.remove(index.saturating_add(1));
                }
                _ => index = index.saturating_add(1),
            }
        }
    }

    /// Removes every frame of `cut` from the list, splitting spans where
    /// needed.
    pub(crate) fn subtract(&mut self, cut: Span) -> Result<(), MapError> {
        if cut.is_empty() {
            return Ok(());
        }
        let mut index = 0;
        while let Some(current) = self.get(index) {
            if current.end <= cut.start || cut.end <= current.start {
                index = index.saturating_add(1);
                continue;
            }
            if cut.start <= current.start && current.end <= cut.end {
                self.remove(index);
                continue;
            }
            if cut.start <= current.start {
                self.put(
                    index,
                    Span {
                        start: cut.end,
                        end: current.end,
                    },
                );
                index = index.saturating_add(1);
                continue;
            }
            if current.end <= cut.end {
                self.put(
                    index,
                    Span {
                        start: current.start,
                        end: cut.start,
                    },
                );
                index = index.saturating_add(1);
                continue;
            }
            self.put(
                index,
                Span {
                    start: current.start,
                    end: cut.start,
                },
            );
            self.insert(
                index.saturating_add(1),
                Span {
                    start: cut.end,
                    end: current.end,
                },
            )?;
            index = index.saturating_add(2);
        }
        Ok(())
    }
}

/// The usable memory of the machine, sorted, disjoint, and frame-aligned.
#[derive(Clone, Copy, Debug)]
pub struct NormalizedMap {
    spans: SpanList,
}

impl NormalizedMap {
    pub(crate) const fn from_spans(spans: &SpanList) -> Self {
        NormalizedMap { spans: *spans }
    }

    /// The number of usable ranges.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.spans.len()
    }

    /// `true` if no usable range remains.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.spans.len() == 0
    }

    /// The usable ranges, lowest first.
    pub fn iter(&self) -> impl Iterator<Item = PhysFrameRange> + '_ {
        self.spans
            .as_slice()
            .iter()
            .filter_map(|span| PhysFrameRange::from_numbers(span.start, span.end).ok())
    }

    /// The total number of usable frames.
    #[must_use]
    pub fn total_frames(&self) -> u64 {
        self.spans
            .as_slice()
            .iter()
            .fold(0u64, |total, span| total.saturating_add(span.frames()))
    }

    /// The total number of usable bytes.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.total_frames() << PAGE_SHIFT
    }

    /// `true` if the ranges are sorted, non-empty, and separated by at
    /// least one frame.
    #[must_use]
    pub fn is_sorted_and_disjoint(&self) -> bool {
        let spans = self.spans.as_slice();
        spans.iter().all(|span| !span.is_empty())
            && spans
                .windows(2)
                .all(|pair| match (pair.first(), pair.get(1)) {
                    (Some(left), Some(right)) => left.end < right.start,
                    _ => true,
                })
    }

    /// Removes `range` from the map.
    pub(crate) fn without(&self, range: PhysFrameRange) -> Result<NormalizedMap, MapError> {
        let mut spans = self.spans;
        spans.subtract(Span {
            start: range.start().number(),
            end: range.end_number(),
        })?;
        spans.sort_and_merge();
        Ok(NormalizedMap { spans })
    }
}

/// The number of the frame holding `address`.
const fn frame_floor(address: u64) -> u64 {
    address >> PAGE_SHIFT
}

/// The number of the first frame at or above `address`.
const fn frame_ceil(address: u64) -> u64 {
    let floor = address >> PAGE_SHIFT;
    if address.is_multiple_of(PAGE_SIZE) {
        floor
    } else {
        floor.saturating_add(1)
    }
}

/// The first byte above a region, or an error if it is not representable.
fn region_end(region: &MemoryRegion) -> Result<u64, MapError> {
    region
        .start
        .as_u64()
        .checked_add(region.len)
        .ok_or(MapError::Address(kernel_types::Error::Overflow))
}

/// The frame numbers a usable region contributes: the start rounded up, the
/// end rounded down.
fn usable_span(region: &MemoryRegion) -> Result<Span, MapError> {
    let end = region_end(region)?;
    Ok(Span {
        start: frame_ceil(region.start.as_u64()),
        end: frame_floor(end).min(MAX_FRAME_NUMBER.saturating_add(1)),
    })
}

/// The frame numbers an excluded region covers: the start rounded down, the
/// end rounded up.
fn excluded_span(region: &MemoryRegion) -> Result<Span, MapError> {
    let end = region_end(region)?;
    Ok(Span {
        start: frame_floor(region.start.as_u64()),
        end: frame_ceil(end),
    })
}

/// Normalizes the firmware memory map.
///
/// Usable regions shrink to frame boundaries, every other region grows to
/// frame boundaries and is then subtracted from the usable ones.
///
/// # Errors
///
/// [`MapError::TooManyRegions`] if more than [`MAX_BOOT_REGIONS`] ranges are
/// needed at any step; [`MapError::NoUsableMemory`] if nothing remains;
/// [`MapError::Address`] if a region reaches beyond the address space.
pub fn normalize(regions: &[MemoryRegion]) -> Result<NormalizedMap, MapError> {
    let mut usable = SpanList::default();
    for region in regions {
        if region.kind != MemoryRegionKind::Usable || region.len == 0 {
            continue;
        }
        let span = usable_span(region)?;
        if !span.is_empty() {
            usable.push(span)?;
        }
    }
    usable.sort_and_merge();
    for region in regions {
        if region.kind == MemoryRegionKind::Usable || region.len == 0 {
            continue;
        }
        let span = excluded_span(region)?;
        usable.subtract(span)?;
    }
    usable.sort_and_merge();
    if usable.len() == 0 {
        return Err(MapError::NoUsableMemory);
    }
    Ok(NormalizedMap::from_spans(&usable))
}
