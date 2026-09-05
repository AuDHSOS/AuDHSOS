// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The bitmap allocator over the kernel reserve.
//!
//! Invariants: a set bit means the frame is handed out; every bit above the
//! managed count is set from the start, so that no frame outside the reserve
//! is ever handed out; `free_count` always equals the number of clear bits
//! below the managed count.

use core::fmt;

use audhsos_abi::layout::PAGE_SHIFT;
use kernel_hal_api::paging::FrameSource;
use kernel_types::{Alignment, PhysFrame, PhysFrameRange};

use crate::reserve::MAX_RESERVE_BYTES;

/// Number of bits in one word of the bitmap.
pub const BITS_PER_WORD: u64 = 64;

/// Number of words the bitmap of the largest reserve needs.
pub const RESERVE_WORDS: usize = (64 * 1024 * 1024 / 4096) / 64;

/// Number of frames the allocator can manage.
pub const MAX_MANAGED_FRAMES: u64 = 64 * 1024 * 1024 / 4096;

const _: () = assert!(MAX_RESERVE_BYTES == 64 * 1024 * 1024);
const _: () = assert!(PAGE_SHIFT == 12);
const _: () = assert!(RESERVE_WORDS == 256);
const _: () = assert!(MAX_MANAGED_FRAMES == 16384);

/// Why a frame operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameError {
    /// No free frame, or no run of free frames, is left.
    OutOfFrames,
    /// The frame lies outside the managed range.
    NotManaged,
    /// The frame is not handed out.
    NotAllocated,
    /// The frame is already handed out.
    AlreadyAllocated,
    /// The requested count is zero or larger than the bitmap.
    InvalidCount,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::OutOfFrames => f.write_str("no free frame is left"),
            FrameError::NotManaged => f.write_str("the frame is outside the managed range"),
            FrameError::NotAllocated => f.write_str("the frame is not allocated"),
            FrameError::AlreadyAllocated => f.write_str("the frame is already allocated"),
            FrameError::InvalidCount => f.write_str("the frame count is out of range"),
        }
    }
}

impl From<FrameError> for audhsos_abi::Error {
    fn from(error: FrameError) -> Self {
        match error {
            FrameError::OutOfFrames => audhsos_abi::Error::OutOfKernelMemory,
            FrameError::NotManaged | FrameError::NotAllocated | FrameError::InvalidCount => {
                audhsos_abi::Error::InvalidArgument
            }
            FrameError::AlreadyAllocated => audhsos_abi::Error::AlreadyExists,
        }
    }
}

/// A bitmap allocator over one contiguous range of frames.
#[derive(Clone, Copy, Debug)]
pub struct BitmapFrameAllocator {
    base: PhysFrame,
    count: u64,
    bits: [u64; RESERVE_WORDS],
    free: u64,
}

impl BitmapFrameAllocator {
    /// An allocator over `range` with every frame free.
    ///
    /// # Errors
    ///
    /// [`FrameError::InvalidCount`] if `range` holds more frames than the
    /// bitmap has bits.
    pub fn new(range: PhysFrameRange) -> Result<Self, FrameError> {
        let count = range.count();
        if count > MAX_MANAGED_FRAMES {
            return Err(FrameError::InvalidCount);
        }
        let mut allocator = BitmapFrameAllocator {
            base: range.start(),
            count,
            bits: [u64::MAX; RESERVE_WORDS],
            free: count,
        };
        let mut number = 0;
        while number < count {
            allocator.set(number, false);
            number = number.saturating_add(1);
        }
        Ok(allocator)
    }

    /// The number of frames under management.
    #[must_use]
    pub const fn capacity(&self) -> u64 {
        self.count
    }

    /// The first managed frame.
    #[must_use]
    pub const fn base(&self) -> PhysFrame {
        self.base
    }

    /// The number of frames that are not handed out.
    #[must_use]
    pub const fn free_count(&self) -> u64 {
        self.free
    }

    /// The managed range.
    #[must_use]
    pub fn range(&self) -> PhysFrameRange {
        PhysFrameRange::new(self.base, self.count).unwrap_or(PhysFrameRange::EMPTY)
    }

    /// `true` if `frame` lies inside the managed range.
    #[must_use]
    pub fn manages(&self, frame: PhysFrame) -> bool {
        self.offset_of(frame).is_some()
    }

    fn offset_of(&self, frame: PhysFrame) -> Option<u64> {
        let offset = frame.number().checked_sub(self.base.number())?;
        (offset < self.count).then_some(offset)
    }

    pub(crate) fn is_set(&self, number: u64) -> bool {
        let word = usize::try_from(number.wrapping_div(BITS_PER_WORD)).unwrap_or(usize::MAX);
        let bit = number.wrapping_rem(BITS_PER_WORD);
        self.bits
            .get(word)
            .is_some_and(|value| value & (1u64 << bit) != 0)
    }

    /// Marks the frame with the given number; a number outside the
    /// bitmap changes nothing.
    pub(crate) fn set(&mut self, number: u64, allocated: bool) {
        let word = usize::try_from(number.wrapping_div(BITS_PER_WORD)).unwrap_or(usize::MAX);
        let bit = number.wrapping_rem(BITS_PER_WORD);
        if let Some(value) = self.bits.get_mut(word) {
            if allocated {
                *value |= 1u64 << bit;
            } else {
                *value &= !(1u64 << bit);
            }
        }
    }

    /// The number of the first free frame at or above `from`.
    fn first_free(&self, from: u64) -> Option<u64> {
        let mut number = from;
        while number < self.count {
            if !self.is_set(number) {
                return Some(number);
            }
            number = number.saturating_add(1);
        }
        None
    }

    /// Hands out one frame.
    ///
    /// # Errors
    ///
    /// [`FrameError::OutOfFrames`] if every frame is handed out.
    pub fn allocate(&mut self) -> Result<PhysFrame, FrameError> {
        let number = self.first_free(0).ok_or(FrameError::OutOfFrames)?;
        self.set(number, true);
        self.free = self.free.saturating_sub(1);
        self.base.checked_add(number).ok_or(FrameError::OutOfFrames)
    }

    /// Hands out `frame` if it is managed and free.
    ///
    /// # Errors
    ///
    /// [`FrameError::NotManaged`] if `frame` is outside the range;
    /// [`FrameError::AlreadyAllocated`] if it is already handed out.
    pub fn allocate_at(&mut self, frame: PhysFrame) -> Result<(), FrameError> {
        let number = self.offset_of(frame).ok_or(FrameError::NotManaged)?;
        if self.is_set(number) {
            return Err(FrameError::AlreadyAllocated);
        }
        self.set(number, true);
        self.free = self.free.saturating_sub(1);
        Ok(())
    }

    /// Hands out `count` consecutive frames whose first frame is aligned to
    /// `alignment`, using the first run that fits.
    ///
    /// # Errors
    ///
    /// [`FrameError::InvalidCount`] if `count` is zero or larger than the
    /// managed range; [`FrameError::OutOfFrames`] if no aligned run of that
    /// length is free.
    pub fn allocate_contiguous(
        &mut self,
        count: u64,
        alignment: Alignment,
    ) -> Result<PhysFrameRange, FrameError> {
        if count == 0 || count > self.count {
            return Err(FrameError::InvalidCount);
        }
        let mut number = 0;
        while let Some(candidate) = self.first_free(number) {
            let Some(start) = self.base.checked_add(candidate) else {
                return Err(FrameError::OutOfFrames);
            };
            if !start.start().is_aligned(alignment) {
                number = candidate.saturating_add(1);
                continue;
            }
            let Some(end) = candidate
                .checked_add(count)
                .filter(|end| *end <= self.count)
            else {
                return Err(FrameError::OutOfFrames);
            };
            if let Some(taken) = (candidate..end).find(|offset| self.is_set(*offset)) {
                number = taken.saturating_add(1);
                continue;
            }
            for offset in candidate..end {
                self.set(offset, true);
            }
            self.free = self.free.saturating_sub(count);
            return PhysFrameRange::new(start, count).map_err(|_| FrameError::OutOfFrames);
        }
        Err(FrameError::OutOfFrames)
    }

    /// Takes `frame` back.
    ///
    /// # Errors
    ///
    /// [`FrameError::NotManaged`] if `frame` is outside the range;
    /// [`FrameError::NotAllocated`] if it is not handed out.
    pub fn free(&mut self, frame: PhysFrame) -> Result<(), FrameError> {
        let number = self.offset_of(frame).ok_or(FrameError::NotManaged)?;
        if !self.is_set(number) {
            return Err(FrameError::NotAllocated);
        }
        self.set(number, false);
        self.free = self.free.saturating_add(1);
        Ok(())
    }

    /// Takes a whole range back. Nothing changes unless every frame of the
    /// range is managed and handed out.
    ///
    /// # Errors
    ///
    /// The errors of [`BitmapFrameAllocator::free`].
    pub fn free_contiguous(&mut self, range: PhysFrameRange) -> Result<(), FrameError> {
        for frame in range {
            let number = self.offset_of(frame).ok_or(FrameError::NotManaged)?;
            if !self.is_set(number) {
                return Err(FrameError::NotAllocated);
            }
        }
        for frame in range {
            let _ = self.free(frame);
        }
        Ok(())
    }
}

impl FrameSource for BitmapFrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        self.allocate().ok()
    }

    fn release_frame(&mut self, frame: PhysFrame) {
        let _ = self.free(frame);
    }
}

/// A frame source that hands out nothing, for a walk that only reads the
/// page tables.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoFrames;

impl NoFrames {
    /// The source that has no frame.
    #[must_use]
    pub const fn new() -> Self {
        NoFrames
    }
}

impl FrameSource for NoFrames {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        None
    }

    fn release_frame(&mut self, _frame: PhysFrame) {}
}
