// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The regions of an address space: what is mapped where, with which
//! permissions, and from which backing object.
//!
//! Invariants: the regions of a table are sorted by their first page,
//! non-empty, pairwise disjoint, and, for a user table, entirely inside the
//! mappable part of the user half.

use core::fmt;

use audhsos_abi::layout::{PAGE_SIZE, USER_SPACE_START};
use kernel_types::{Page, PageRange, VirtAddr};

use crate::page_table::Permissions;

/// Why a region operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegionError {
    /// The request covers no page.
    Empty,
    /// An address or a length is not a multiple of the page size.
    Unaligned,
    /// The request reaches beyond the address space.
    Overflow,
    /// The request lies below the lowest mappable address or in the kernel
    /// half.
    OutsideUserSpace,
    /// The request overlaps an existing region.
    AddressInUse,
    /// The request covers pages that no region holds.
    NotMapped,
    /// The table has no slot left for the region the request would create.
    QuotaExceeded,
}

impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegionError::Empty => f.write_str("the request covers no page"),
            RegionError::Unaligned => f.write_str("the request is not page-aligned"),
            RegionError::Overflow => f.write_str("the request leaves the address space"),
            RegionError::OutsideUserSpace => f.write_str("the request is outside user space"),
            RegionError::AddressInUse => f.write_str("the range overlaps an existing region"),
            RegionError::NotMapped => f.write_str("the range has no region"),
            RegionError::QuotaExceeded => f.write_str("the region table is full"),
        }
    }
}

impl From<RegionError> for audhsos_abi::Error {
    fn from(error: RegionError) -> Self {
        match error {
            RegionError::Empty | RegionError::Overflow | RegionError::OutsideUserSpace => {
                audhsos_abi::Error::InvalidArgument
            }
            RegionError::Unaligned => audhsos_abi::Error::Unaligned,
            RegionError::AddressInUse => audhsos_abi::Error::AddressInUse,
            RegionError::NotMapped => audhsos_abi::Error::NotMapped,
            RegionError::QuotaExceeded => audhsos_abi::Error::QuotaExceeded,
        }
    }
}

/// The page range a user request of `len` bytes at `start` names.
///
/// # Errors
///
/// [`RegionError::Empty`] for a length of zero; [`RegionError::Unaligned`]
/// if the start or the length is not page-aligned;
/// [`RegionError::Overflow`] if the range leaves the address space;
/// [`RegionError::OutsideUserSpace`] if it starts below
/// [`USER_SPACE_START`] or in the kernel half.
pub fn user_range(start: u64, len: u64) -> Result<PageRange, RegionError> {
    if len == 0 {
        return Err(RegionError::Empty);
    }
    if !start.is_multiple_of(PAGE_SIZE) || !len.is_multiple_of(PAGE_SIZE) {
        return Err(RegionError::Unaligned);
    }
    let address = VirtAddr::new(start).map_err(|_| RegionError::OutsideUserSpace)?;
    if !address.is_user() || start < USER_SPACE_START {
        return Err(RegionError::OutsideUserSpace);
    }
    let page = Page::from_start(address).map_err(|_| RegionError::Unaligned)?;
    let count = len.wrapping_div(PAGE_SIZE);
    PageRange::new(page, count).map_err(|_| RegionError::Overflow)
}

/// One mapped range and what backs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region<B: Copy> {
    /// The pages the region covers.
    pub pages: PageRange,
    /// What the region is backed by.
    pub backing: B,
    /// Offset of the first page inside the backing object.
    pub offset: u64,
    /// The permissions of the mapping.
    pub perms: Permissions,
}

impl<B: Copy> Region<B> {
    /// The part of the region below `page`, or `None` if nothing is left.
    pub(crate) fn head(self, page: Page) -> Option<Region<B>> {
        let count = page.number().checked_sub(self.pages.start().number())?;
        if count == 0 {
            return None;
        }
        Some(Region {
            pages: PageRange::new(self.pages.start(), count).ok()?,
            ..self
        })
    }

    /// The part of the region at and above `page`, or `None` if nothing is
    /// left.
    pub(crate) fn tail(self, page: Page) -> Option<Region<B>> {
        let skipped = page.number().checked_sub(self.pages.start().number())?;
        let count = self.pages.count().checked_sub(skipped)?;
        if count == 0 {
            return None;
        }
        Some(Region {
            pages: PageRange::new(page, count).ok()?,
            offset: self.offset.checked_add(skipped.checked_mul(PAGE_SIZE)?)?,
            ..self
        })
    }

    /// The part of the region inside `pages`, or `None` if they do not
    /// overlap.
    pub(crate) fn clipped(self, pages: PageRange) -> Option<Region<B>> {
        let start = self.pages.start().number().max(pages.start().number());
        let end = self.pages.end_number().min(pages.end_number());
        if end <= start {
            return None;
        }
        let page = self
            .pages
            .start()
            .checked_add(start.checked_sub(self.pages.start().number())?)?;
        self.tail(page).and_then(|region| {
            Some(Region {
                pages: PageRange::new(page, end.checked_sub(start)?).ok()?,
                ..region
            })
        })
    }
}

/// The regions one operation removed, and what is left of the request.
#[derive(Clone, Copy, Debug)]
pub struct Removed<B: Copy, const M: usize> {
    items: [Option<Region<B>>; M],
    len: usize,
    remaining: Option<PageRange>,
}

impl<B: Copy, const M: usize> Removed<B, M> {
    const fn new() -> Self {
        Removed {
            items: [None; M],
            len: 0,
            remaining: None,
        }
    }

    fn push(&mut self, region: Region<B>) -> bool {
        match self.items.get_mut(self.len) {
            Some(slot) => {
                *slot = Some(region);
                self.len = self.len.saturating_add(1);
                true
            }
            None => false,
        }
    }

    /// The removed regions, lowest first.
    pub fn iter(&self) -> impl Iterator<Item = &Region<B>> {
        self.items.iter().take(self.len).flatten()
    }

    /// The number of removed regions.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// `true` if nothing was removed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The part of the request this call did not reach, if any. The caller
    /// repeats the operation with it.
    #[must_use]
    pub const fn remaining(&self) -> Option<PageRange> {
        self.remaining
    }
}

/// The regions of one address space, at most `N` of them.
#[derive(Clone, Copy, Debug)]
pub struct RegionTable<B: Copy, const N: usize> {
    regions: [Option<Region<B>>; N],
    len: usize,
    user: bool,
}

impl<B: Copy, const N: usize> Default for RegionTable<B, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: Copy, const N: usize> RegionTable<B, N> {
    /// An empty table for a user address space.
    #[must_use]
    pub const fn new() -> Self {
        RegionTable {
            regions: [None; N],
            len: 0,
            user: true,
        }
    }

    /// An empty table for the kernel address space, where the checks
    /// against the user half do not apply.
    #[must_use]
    pub const fn kernel() -> Self {
        RegionTable {
            regions: [None; N],
            len: 0,
            user: false,
        }
    }

    /// The number of regions.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// `true` if the table holds no region.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of regions the table can still take.
    #[must_use]
    pub const fn free_slots(&self) -> usize {
        N.saturating_sub(self.len)
    }

    /// The regions, lowest first.
    pub fn iter(&self) -> impl Iterator<Item = &Region<B>> {
        self.regions.iter().take(self.len).flatten()
    }

    /// The region in slot `index`. Slots at or above `len` hold nothing, so
    /// an index outside the used part reports nothing.
    pub(crate) fn get(&self, index: usize) -> Option<Region<B>> {
        self.regions.get(index).copied().flatten()
    }

    /// Writes `region` into slot `index`; an index outside the array
    /// changes nothing.
    pub(crate) fn put(&mut self, index: usize, region: Option<Region<B>>) {
        if let Some(slot) = self.regions.get_mut(index) {
            *slot = region;
        }
    }

    fn set(&mut self, index: usize, region: Region<B>) {
        self.put(index, Some(region));
    }

    /// Inserts `region` at `index`, moving the rest up.
    pub(crate) fn insert_at(&mut self, index: usize, region: Region<B>) -> Result<(), RegionError> {
        if self.len >= N || index > self.len {
            return Err(RegionError::QuotaExceeded);
        }
        let mut position = self.len;
        while position > index {
            let previous = self.get(position.saturating_sub(1));
            self.put(position, previous);
            position = position.saturating_sub(1);
        }
        self.put(index, Some(region));
        self.len = self.len.saturating_add(1);
        Ok(())
    }

    /// Removes the region at `index`, moving the rest down.
    pub(crate) fn remove_at(&mut self, index: usize) {
        if index >= self.len {
            return;
        }
        let mut position = index;
        while position.saturating_add(1) < self.len {
            let next = self.get(position.saturating_add(1));
            self.put(position, next);
            position = position.saturating_add(1);
        }
        self.put(self.len.saturating_sub(1), None);
        self.len = self.len.saturating_sub(1);
    }

    /// The position at which a region starting at `page` belongs.
    fn position_for(&self, page: Page) -> usize {
        let mut index = 0;
        while index < self.len {
            match self.get(index) {
                Some(region) if region.pages.start().number() < page.number() => {
                    index = index.saturating_add(1);
                }
                _ => break,
            }
        }
        index
    }

    /// The index of the region holding `page`.
    fn index_of(&self, page: Page) -> Option<usize> {
        (0..self.len).find(|index| {
            self.get(*index)
                .is_some_and(|region| region.pages.contains(page))
        })
    }

    /// The region holding `page`.
    #[must_use]
    pub fn find(&self, page: Page) -> Option<&Region<B>> {
        self.regions
            .iter()
            .take(self.len)
            .flatten()
            .find(|region| region.pages.contains(page))
    }

    const fn check_range(&self, pages: PageRange) -> Result<(), RegionError> {
        if pages.is_empty() {
            return Err(RegionError::Empty);
        }
        if !self.user {
            return Ok(());
        }
        if !pages.is_user() || pages.start().start().as_u64() < USER_SPACE_START {
            return Err(RegionError::OutsideUserSpace);
        }
        Ok(())
    }

    /// Adds `region` to the table.
    ///
    /// # Errors
    ///
    /// [`RegionError::Empty`] for an empty range;
    /// [`RegionError::OutsideUserSpace`] for a range outside the mappable
    /// user half; [`RegionError::AddressInUse`] if it overlaps an existing
    /// region; [`RegionError::QuotaExceeded`] if the table is full.
    pub fn insert(&mut self, region: Region<B>) -> Result<(), RegionError> {
        self.check_range(region.pages)?;
        if self
            .iter()
            .any(|existing| existing.pages.overlaps(region.pages))
        {
            return Err(RegionError::AddressInUse);
        }
        let index = self.position_for(region.pages.start());
        self.insert_at(index, region)
    }

    /// Removes the pages of `pages` from the table and returns what was
    /// removed. At most `M` pieces are removed per call; the rest of the
    /// request comes back in [`Removed::remaining`].
    ///
    /// # Errors
    ///
    /// [`RegionError::Empty`] for an empty range;
    /// [`RegionError::OutsideUserSpace`] for a range outside the mappable
    /// user half; [`RegionError::NotMapped`] if no region overlaps the
    /// range; [`RegionError::QuotaExceeded`] if a split would need a slot
    /// the table does not have.
    pub fn remove<const M: usize>(
        &mut self,
        pages: PageRange,
    ) -> Result<Removed<B, M>, RegionError> {
        self.check_range(pages)?;
        if !self.iter().any(|region| region.pages.overlaps(pages)) {
            return Err(RegionError::NotMapped);
        }
        let mut removed = Removed::<B, M>::new();
        let mut index = 0;
        while index < self.len {
            let Some(region) = self.get(index) else { break };
            if !region.pages.overlaps(pages) {
                index = index.saturating_add(1);
                continue;
            }
            let Some(piece) = region.clipped(pages) else {
                index = index.saturating_add(1);
                continue;
            };
            if removed.len() >= M {
                removed.remaining = remaining_from(piece.pages.start(), pages);
                return Ok(removed);
            }
            let head = region.head(piece.pages.start());
            let tail = piece
                .pages
                .last()
                .and_then(|last| last.checked_add(1))
                .and_then(|next| region.tail(next));
            match (head, tail) {
                (None, None) => self.remove_at(index),
                (Some(head), None) => {
                    self.set(index, head);
                    index = index.saturating_add(1);
                }
                (None, Some(tail)) => {
                    self.set(index, tail);
                    index = index.saturating_add(1);
                }
                (Some(head), Some(tail)) => {
                    if self.len >= N {
                        return Err(RegionError::QuotaExceeded);
                    }
                    self.set(index, head);
                    self.insert_at(index.saturating_add(1), tail)?;
                    index = index.saturating_add(2);
                }
            }
            removed.push(piece);
        }
        Ok(removed)
    }

    /// Sets the permissions of `pages`, splitting the region that holds
    /// them into up to three.
    ///
    /// # Errors
    ///
    /// [`RegionError::Empty`] for an empty range;
    /// [`RegionError::OutsideUserSpace`] for a range outside the mappable
    /// user half; [`RegionError::NotMapped`] if the range is not covered by
    /// one region; [`RegionError::QuotaExceeded`] if the table has no room
    /// for the split.
    pub fn protect(&mut self, pages: PageRange, perms: Permissions) -> Result<(), RegionError> {
        self.check_range(pages)?;
        let index = self.index_of(pages.start()).ok_or(RegionError::NotMapped)?;
        let region = self.get(index).ok_or(RegionError::NotMapped)?;
        if pages.end_number() > region.pages.end_number() {
            return Err(RegionError::NotMapped);
        }
        let head = region.head(pages.start());
        let tail = pages
            .last()
            .and_then(|last| last.checked_add(1))
            .and_then(|next| region.tail(next));
        let middle = Region {
            perms,
            ..region.clipped(pages).ok_or(RegionError::NotMapped)?
        };
        let extra = usize::from(head.is_some()).saturating_add(usize::from(tail.is_some()));
        if self.free_slots() < extra {
            return Err(RegionError::QuotaExceeded);
        }
        let next = if let Some(head) = head {
            self.set(index, head);
            self.insert_at(index.saturating_add(1), middle)?;
            index.saturating_add(2)
        } else {
            self.set(index, middle);
            index.saturating_add(1)
        };
        if let Some(tail) = tail {
            self.insert_at(next, tail)?;
        }
        Ok(())
    }

    /// `true` if the regions are sorted, non-empty, disjoint, and, for a
    /// user table, inside the mappable user half.
    #[must_use]
    pub fn check_invariants(&self) -> bool {
        let mut previous: Option<PageRange> = None;
        for region in self.iter() {
            if region.pages.is_empty() {
                return false;
            }
            if self.user
                && (!region.pages.is_user()
                    || region.pages.start().start().as_u64() < USER_SPACE_START)
            {
                return false;
            }
            if let Some(previous) = previous
                && previous.end_number() > region.pages.start().number()
            {
                return false;
            }
            previous = Some(region.pages);
        }
        true
    }
}

/// The part of `pages` at and above `page`.
pub(crate) fn remaining_from(page: Page, pages: PageRange) -> Option<PageRange> {
    let count = pages.end_number().checked_sub(page.number())?;
    if count == 0 {
        return None;
    }
    PageRange::new(page, count).ok()
}
