// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Physical addresses, frames, and frame ranges.
//!
//! Invariants: a [`PhysAddr`] is below `2^PHYS_ADDR_BITS`; a [`PhysFrame`]
//! is page-aligned; a [`PhysFrameRange`] never extends beyond the last
//! representable frame.

use core::fmt;

use audhsos_abi::layout::{PAGE_SHIFT, PAGE_SIZE, PHYS_ADDR_BITS};

use crate::{Alignment, Error};

/// The highest physical address.
pub const MAX_PHYS_ADDR: u64 = (1 << PHYS_ADDR_BITS) - 1;

/// The highest frame number.
pub const MAX_FRAME_NUMBER: u64 = MAX_PHYS_ADDR >> PAGE_SHIFT;

/// A physical address.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysAddr(u64);

impl PhysAddr {
    /// Address zero.
    pub const ZERO: PhysAddr = PhysAddr(0);

    /// The highest address.
    pub const MAX: PhysAddr = PhysAddr(MAX_PHYS_ADDR);

    /// Validates that `raw` fits the physical address width.
    ///
    /// # Errors
    ///
    /// `ExceedsPhysicalLimit` if `raw` needs more than `PHYS_ADDR_BITS` bits.
    pub const fn new(raw: u64) -> Result<Self, Error> {
        if raw <= MAX_PHYS_ADDR {
            Ok(PhysAddr(raw))
        } else {
            Err(Error::ExceedsPhysicalLimit(raw))
        }
    }

    /// The raw address.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// `true` if the address is a multiple of `alignment`.
    #[must_use]
    pub const fn is_aligned(self, alignment: Alignment) -> bool {
        alignment.is_aligned(self.0)
    }

    /// Rounds down to a multiple of `alignment`.
    #[must_use]
    pub const fn align_down(self, alignment: Alignment) -> PhysAddr {
        PhysAddr(alignment.align_down(self.0))
    }

    /// Rounds up to a multiple of `alignment`; fails if the result exceeds
    /// the physical address width.
    ///
    /// # Errors
    ///
    /// `Overflow` if the rounded address exceeds the physical address width.
    pub const fn align_up(self, alignment: Alignment) -> Result<PhysAddr, Error> {
        match alignment.align_up(self.0) {
            Some(raw) if raw <= MAX_PHYS_ADDR => Ok(PhysAddr(raw)),
            _ => Err(Error::Overflow),
        }
    }

    /// The address `bytes` further up, if representable.
    #[must_use]
    pub const fn checked_add(self, bytes: u64) -> Option<PhysAddr> {
        match self.0.checked_add(bytes) {
            Some(raw) if raw <= MAX_PHYS_ADDR => Some(PhysAddr(raw)),
            _ => None,
        }
    }

    /// The address `bytes` further down, if representable.
    #[must_use]
    pub const fn checked_sub(self, bytes: u64) -> Option<PhysAddr> {
        match self.0.checked_sub(bytes) {
            Some(raw) => Some(PhysAddr(raw)),
            None => None,
        }
    }

    /// The offset of the address inside its frame.
    #[must_use]
    pub const fn offset_in_frame(self) -> u64 {
        self.0 & Alignment::PAGE.mask()
    }

    /// The frame containing the address.
    #[must_use]
    pub const fn frame(self) -> PhysFrame {
        PhysFrame(self.align_down(Alignment::PAGE))
    }
}

impl fmt::Debug for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PhysAddr({:#x})", self.0)
    }
}

impl fmt::Display for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

/// A page-aligned physical frame.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysFrame(PhysAddr);

impl PhysFrame {
    /// The frame starting at `start`; fails if `start` is not page-aligned.
    ///
    /// # Errors
    ///
    /// `Unaligned` if `start` is not a multiple of the page size.
    pub const fn from_start(start: PhysAddr) -> Result<Self, Error> {
        if start.is_aligned(Alignment::PAGE) {
            Ok(PhysFrame(start))
        } else {
            Err(Error::Unaligned {
                address: start.0,
                alignment: PAGE_SIZE,
            })
        }
    }

    /// The frame containing `address`.
    #[must_use]
    pub const fn containing(address: PhysAddr) -> Self {
        address.frame()
    }

    /// The frame with the given number; fails above [`MAX_FRAME_NUMBER`].
    ///
    /// # Errors
    ///
    /// `Overflow` above [`MAX_FRAME_NUMBER`].
    pub const fn from_number(number: u64) -> Result<Self, Error> {
        if number <= MAX_FRAME_NUMBER {
            Ok(PhysFrame(PhysAddr(number << PAGE_SHIFT)))
        } else {
            Err(Error::Overflow)
        }
    }

    /// The frame number (`start / PAGE_SIZE`).
    #[must_use]
    pub const fn number(self) -> u64 {
        self.0.0 >> PAGE_SHIFT
    }

    /// The first address of the frame.
    #[must_use]
    pub const fn start(self) -> PhysAddr {
        self.0
    }

    /// The last address of the frame.
    #[must_use]
    pub const fn last_address(self) -> PhysAddr {
        PhysAddr(self.0.0 | Alignment::PAGE.mask())
    }

    /// The frame `count` frames further up, if representable.
    #[must_use]
    pub const fn checked_add(self, count: u64) -> Option<PhysFrame> {
        match self.number().checked_add(count) {
            Some(number) if number <= MAX_FRAME_NUMBER => {
                Some(PhysFrame(PhysAddr(number << PAGE_SHIFT)))
            }
            _ => None,
        }
    }

    /// The frame `count` frames further down, if representable.
    #[must_use]
    pub const fn checked_sub(self, count: u64) -> Option<PhysFrame> {
        match self.number().checked_sub(count) {
            Some(number) => Some(PhysFrame(PhysAddr(number << PAGE_SHIFT))),
            None => None,
        }
    }
}

impl fmt::Debug for PhysFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PhysFrame({:#x})", self.0.0)
    }
}

/// A range of consecutive frames, given by its first frame and a count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PhysFrameRange {
    start: PhysFrame,
    count: u64,
}

impl PhysFrameRange {
    /// `count` frames starting at `start`; fails if the range would extend
    /// beyond the last representable frame.
    ///
    /// # Errors
    ///
    /// `Overflow` if the range would extend beyond the last representable
    /// frame.
    pub const fn new(start: PhysFrame, count: u64) -> Result<Self, Error> {
        match start.number().checked_add(count) {
            Some(end) if end <= MAX_FRAME_NUMBER.wrapping_add(1) => {
                Ok(PhysFrameRange { start, count })
            }
            _ => Err(Error::Overflow),
        }
    }

    /// The frames whose numbers lie in `start_number..end_number`; fails if
    /// `end_number` is below `start_number` or beyond the representable
    /// frames.
    ///
    /// # Errors
    ///
    /// `Overflow` if `end_number` is below `start_number` or beyond the
    /// representable frames.
    pub const fn from_numbers(start_number: u64, end_number: u64) -> Result<Self, Error> {
        if end_number < start_number {
            return Err(Error::Overflow);
        }
        match PhysFrame::from_number(start_number) {
            Ok(start) => Self::new(start, end_number.wrapping_sub(start_number)),
            Err(error) => Err(error),
        }
    }

    /// The first frame.
    #[must_use]
    pub const fn start(self) -> PhysFrame {
        self.start
    }

    /// The number of frames.
    #[must_use]
    pub const fn count(self) -> u64 {
        self.count
    }

    /// The number of bytes.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.count << PAGE_SHIFT
    }

    /// `true` if the range holds no frame.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }

    /// The frame number one past the last frame.
    #[must_use]
    pub const fn end_number(self) -> u64 {
        self.start.number().wrapping_add(self.count)
    }

    /// The last frame, if the range is not empty.
    #[must_use]
    pub const fn last(self) -> Option<PhysFrame> {
        if self.count == 0 {
            None
        } else {
            self.start.checked_add(self.count.wrapping_sub(1))
        }
    }

    /// `true` if `frame` lies in the range.
    #[must_use]
    pub const fn contains(self, frame: PhysFrame) -> bool {
        frame.number() >= self.start.number() && frame.number() < self.end_number()
    }

    /// `true` if the ranges share at least one frame.
    #[must_use]
    pub const fn overlaps(self, other: PhysFrameRange) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.start.number() < other.end_number()
            && other.start.number() < self.end_number()
    }

    /// The frames in both ranges, or `None` if they share none.
    #[must_use]
    pub const fn intersection(self, other: PhysFrameRange) -> Option<PhysFrameRange> {
        if !self.overlaps(other) {
            return None;
        }
        let start = if self.start.number() > other.start.number() {
            self.start
        } else {
            other.start
        };
        let end = if self.end_number() < other.end_number() {
            self.end_number()
        } else {
            other.end_number()
        };
        Some(PhysFrameRange {
            start,
            count: end.wrapping_sub(start.number()),
        })
    }

    /// Iterates over the frames.
    #[must_use]
    pub const fn iter(self) -> PhysFrames {
        PhysFrames {
            next: self.start.number(),
            end: self.end_number(),
        }
    }
}

impl IntoIterator for PhysFrameRange {
    type Item = PhysFrame;
    type IntoIter = PhysFrames;

    fn into_iter(self) -> PhysFrames {
        self.iter()
    }
}

/// Iterator over the frames of a [`PhysFrameRange`].
#[derive(Clone, Debug)]
pub struct PhysFrames {
    next: u64,
    end: u64,
}

impl Iterator for PhysFrames {
    type Item = PhysFrame;

    fn next(&mut self) -> Option<PhysFrame> {
        if self.next >= self.end {
            return None;
        }
        let frame = PhysFrame::from_number(self.next).ok()?;
        self.next = self.next.wrapping_add(1);
        Some(frame)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = usize::try_from(self.end.saturating_sub(self.next)).unwrap_or(usize::MAX);
        (remaining, Some(remaining))
    }
}
