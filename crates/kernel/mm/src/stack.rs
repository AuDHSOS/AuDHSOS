// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The kernel stack area: one slot per stack, each slot a guard page
//! followed by the pages the stack grows down through.
//!
//! Invariants: a slot is either fully mapped or fully unmapped; the guard
//! page of a slot is never mapped; a slot that an allocation could not
//! finish is given back with every frame it had taken.

use core::fmt;

use audhsos_abi::layout::{
    KERNEL_STACK_PAGES, KERNEL_STACK_SLOT_PAGES, KERNEL_STACKS_BASE, PAGE_SIZE,
};
use kernel_hal_api::paging::{FrameAccess, FrameSource, TlbControl};
use kernel_types::{Page, PageRange, VirtAddr};

use crate::mapper::{MapError, Mapper};
use crate::page_table::{CachePolicy, EntryFormat, PageTable, Permissions};

/// Number of bits in one word of the slot bitmap.
const BITS_PER_WORD: u64 = 64;

/// Why a kernel stack operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StackError {
    /// Every slot of the area is in use.
    Exhausted,
    /// The slot is not handed out, or lies outside the area.
    NotAllocated,
    /// No frame was left for a stack page.
    OutOfKernelMemory,
    /// The slot does not describe a usable page range.
    Address,
    /// A mapping operation failed.
    Map(MapError),
}

impl fmt::Display for StackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StackError::Exhausted => f.write_str("every kernel stack slot is in use"),
            StackError::NotAllocated => f.write_str("the kernel stack slot is not handed out"),
            StackError::OutOfKernelMemory => f.write_str("no frame is left for a kernel stack"),
            StackError::Address => f.write_str("the kernel stack slot is not addressable"),
            StackError::Map(error) => write!(f, "{error}"),
        }
    }
}

impl From<MapError> for StackError {
    fn from(error: MapError) -> Self {
        StackError::Map(error)
    }
}

impl From<StackError> for audhsos_abi::Error {
    fn from(error: StackError) -> Self {
        match error {
            StackError::Exhausted => audhsos_abi::Error::QuotaExceeded,
            StackError::NotAllocated => audhsos_abi::Error::NotMapped,
            StackError::OutOfKernelMemory => audhsos_abi::Error::OutOfKernelMemory,
            StackError::Address => audhsos_abi::Error::InvalidArgument,
            StackError::Map(error) => error.into(),
        }
    }
}

/// One kernel stack: the mapped pages and the slot they came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernelStack {
    pages: PageRange,
    index: u32,
}

impl KernelStack {
    /// The mapped pages, lowest first.
    #[must_use]
    pub const fn pages(&self) -> PageRange {
        self.pages
    }

    /// The slot the stack occupies.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// The address one past the highest byte of the stack, which is where
    /// a stack pointer starts.
    #[must_use]
    pub const fn top(&self) -> Option<VirtAddr> {
        self.pages.end_address()
    }

    /// The unmapped page below the stack.
    #[must_use]
    pub const fn guard(&self) -> Option<Page> {
        self.pages.start().checked_sub(1)
    }
}

/// The page range of slot `index`, guard page excluded.
///
/// # Errors
///
/// [`StackError::NotAllocated`] if `index` is at or above `slots`;
/// [`StackError::Address`] if the slot leaves the address space.
pub fn slot_pages(index: u32, slots: u32) -> Result<PageRange, StackError> {
    if index >= slots {
        return Err(StackError::NotAllocated);
    }
    let slot = u64::from(index)
        .checked_mul(KERNEL_STACK_SLOT_PAGES)
        .and_then(|pages| pages.checked_mul(PAGE_SIZE))
        .and_then(|bytes| bytes.checked_add(PAGE_SIZE))
        .and_then(|offset| KERNEL_STACKS_BASE.checked_add(offset))
        .ok_or(StackError::Address)?;
    let start = VirtAddr::new(slot)
        .and_then(Page::from_start)
        .map_err(|_| StackError::Address)?;
    PageRange::new(start, KERNEL_STACK_PAGES).map_err(|_| StackError::Address)
}

/// The slots of the kernel stack area, `N` bitmap words wide.
///
/// A word holds sixty-four slots, so a pool of `N` words manages
/// `N * 64` slots.
#[derive(Clone, Copy, Debug)]
pub struct StackPool<const N: usize> {
    used: [u64; N],
    live: u32,
}

impl<const N: usize> Default for StackPool<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> StackPool<N> {
    /// A pool whose slots are all free.
    #[must_use]
    pub const fn new() -> Self {
        StackPool {
            used: [0; N],
            live: 0,
        }
    }

    /// The number of slots the pool manages.
    #[must_use]
    pub fn capacity(&self) -> u32 {
        u64::try_from(N)
            .unwrap_or(u64::MAX)
            .saturating_mul(BITS_PER_WORD)
            .try_into()
            .unwrap_or(u32::MAX)
    }

    /// The number of slots that are handed out.
    #[must_use]
    pub const fn live(&self) -> u32 {
        self.live
    }

    /// `true` if no slot is handed out.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// `true` if slot `index` is handed out.
    #[must_use]
    pub fn is_used(&self, index: u32) -> bool {
        let (word, bit) = position(index);
        self.used.get(word).is_some_and(|value| value & bit != 0)
    }

    /// Marks slot `index` as handed out; an index outside the pool changes
    /// nothing.
    fn take(&mut self, index: u32) {
        let (word, bit) = position(index);
        if let Some(value) = self.used.get_mut(word) {
            *value |= bit;
            self.live = self.live.saturating_add(1);
        }
    }

    /// Marks slot `index` as free; an index outside the pool changes
    /// nothing.
    fn give_back(&mut self, index: u32) {
        let (word, bit) = position(index);
        if let Some(value) = self.used.get_mut(word) {
            *value &= !bit;
            self.live = self.live.saturating_sub(1);
        }
    }

    /// The lowest free slot.
    fn first_free(&self) -> Option<u32> {
        let capacity = self.capacity();
        (0..capacity).find(|index| !self.is_used(*index))
    }

    /// Takes a slot, maps [`KERNEL_STACK_PAGES`] frames into it, and
    /// returns the stack. Nothing is left mapped and no frame is left
    /// taken if the call fails.
    ///
    /// # Errors
    ///
    /// [`StackError::Exhausted`] if every slot is in use;
    /// [`StackError::OutOfKernelMemory`] if a frame was missing;
    /// [`StackError::Address`] if the slot is not addressable; the errors
    /// of [`Mapper::map`] otherwise.
    pub fn allocate<F, A, T, S>(
        &mut self,
        mapper: &mut Mapper<'_, F, A, T, S>,
    ) -> Result<KernelStack, StackError>
    where
        F: EntryFormat,
        A: FrameAccess<PageTable<F>>,
        T: TlbControl,
        S: FrameSource,
    {
        let index = self.first_free().ok_or(StackError::Exhausted)?;
        let pages = slot_pages(index, self.capacity())?;
        match map_stack(mapper, pages) {
            Ok(()) => {
                self.take(index);
                Ok(KernelStack { pages, index })
            }
            Err(error) => {
                unmap_stack(mapper, pages);
                Err(error)
            }
        }
    }

    /// Unmaps the pages of `stack`, gives every frame back, and frees the
    /// slot.
    ///
    /// # Errors
    ///
    /// [`StackError::NotAllocated`] if the slot is not handed out.
    pub fn release<F, A, T, S>(
        &mut self,
        mapper: &mut Mapper<'_, F, A, T, S>,
        stack: KernelStack,
    ) -> Result<(), StackError>
    where
        F: EntryFormat,
        A: FrameAccess<PageTable<F>>,
        T: TlbControl,
        S: FrameSource,
    {
        if !self.is_used(stack.index) {
            return Err(StackError::NotAllocated);
        }
        unmap_stack(mapper, stack.pages);
        self.give_back(stack.index);
        Ok(())
    }
}

/// The bitmap word and the bit of slot `index`.
fn position(index: u32) -> (usize, u64) {
    let index = u64::from(index);
    let word = usize::try_from(index.wrapping_div(BITS_PER_WORD)).unwrap_or(usize::MAX);
    (word, 1u64 << index.wrapping_rem(BITS_PER_WORD))
}

/// Maps one fresh frame at every page of `pages`.
fn map_stack<F, A, T, S>(
    mapper: &mut Mapper<'_, F, A, T, S>,
    pages: PageRange,
) -> Result<(), StackError>
where
    F: EntryFormat,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
    S: FrameSource,
{
    for page in pages {
        let frame = mapper
            .frames_mut()
            .allocate_frame()
            .ok_or(StackError::OutOfKernelMemory)?;
        if let Err(error) = mapper.map(page, frame, Permissions::READ_WRITE, CachePolicy::WriteBack)
        {
            mapper.frames_mut().release_frame(frame);
            return Err(StackError::Map(error));
        }
    }
    Ok(())
}

/// Unmaps every page of `pages` that has a translation and gives its frame
/// back. A page without a translation is skipped, which is what a rollback
/// of a half-finished allocation needs.
fn unmap_stack<F, A, T, S>(mapper: &mut Mapper<'_, F, A, T, S>, pages: PageRange)
where
    F: EntryFormat,
    A: FrameAccess<PageTable<F>>,
    T: TlbControl,
    S: FrameSource,
{
    for page in pages {
        if let Ok(frame) = mapper.unmap(page) {
            mapper.frames_mut().release_frame(frame);
        }
    }
}
