// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Virtual addresses, pages, and page ranges.
//!
//! Invariants: a [`VirtAddr`] is canonical (bits above bit 47 equal bit 47);
//! a [`Page`] is page-aligned; a [`PageRange`] lies entirely in one half of
//! the address space and never wraps around.

use core::fmt;

use audhsos_abi::layout::{
    KERNEL_SPACE_START, PAGE_SHIFT, PAGE_SIZE, USER_SPACE_END, VIRT_ADDR_BITS,
};

use crate::{Alignment, Error};

/// A canonical virtual address.
///
/// The representation is the word itself, because the saved context of a
/// thread is a virtual address that the context switch writes through a
/// pointer (D-67).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct VirtAddr(u64);

const fn is_canonical(raw: u64) -> bool {
    raw < USER_SPACE_END || raw >= KERNEL_SPACE_START
}

impl VirtAddr {
    /// Address zero.
    pub const ZERO: VirtAddr = VirtAddr(0);

    /// The highest user address.
    pub const USER_MAX: VirtAddr = VirtAddr(USER_SPACE_END - 1);

    /// The lowest kernel address.
    pub const KERNEL_MIN: VirtAddr = VirtAddr(KERNEL_SPACE_START);

    /// The highest address.
    pub const MAX: VirtAddr = VirtAddr(u64::MAX);

    /// Validates that `raw` is canonical.
    ///
    /// # Errors
    ///
    /// `NonCanonical` if the bits above bit 47 do not all equal bit 47.
    pub const fn new(raw: u64) -> Result<Self, Error> {
        if is_canonical(raw) {
            Ok(VirtAddr(raw))
        } else {
            Err(Error::NonCanonical(raw))
        }
    }

    /// The raw address.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// `true` for addresses in the user half.
    #[must_use]
    pub const fn is_user(self) -> bool {
        self.0 < USER_SPACE_END
    }

    /// `true` for addresses in the kernel half.
    #[must_use]
    pub const fn is_kernel(self) -> bool {
        self.0 >= KERNEL_SPACE_START
    }

    /// `true` if the address is a multiple of `alignment`.
    #[must_use]
    pub const fn is_aligned(self, alignment: Alignment) -> bool {
        alignment.is_aligned(self.0)
    }

    /// Rounds down to a multiple of `alignment`. The result stays in the
    /// same half, because both half boundaries are aligned to every
    /// alignment up to `2^47`.
    #[must_use]
    pub const fn align_down(self, alignment: Alignment) -> VirtAddr {
        VirtAddr(alignment.align_down(self.0))
    }

    /// Rounds up to a multiple of `alignment`; fails if the result would
    /// leave the half of the address space or overflow.
    ///
    /// # Errors
    ///
    /// `CrossesCanonicalHole` if rounding leaves the half of the address
    /// space; `Overflow` if it leaves `u64`.
    pub const fn align_up(self, alignment: Alignment) -> Result<VirtAddr, Error> {
        match alignment.align_up(self.0) {
            Some(raw) if is_canonical(raw) && (raw < USER_SPACE_END) == self.is_user() => {
                Ok(VirtAddr(raw))
            }
            Some(_) => Err(Error::CrossesCanonicalHole),
            None => Err(Error::Overflow),
        }
    }

    /// The address `bytes` further up, if it stays in the same half.
    #[must_use]
    pub const fn checked_add(self, bytes: u64) -> Option<VirtAddr> {
        match self.0.checked_add(bytes) {
            Some(raw) if is_canonical(raw) && (raw < USER_SPACE_END) == self.is_user() => {
                Some(VirtAddr(raw))
            }
            _ => None,
        }
    }

    /// The address `bytes` further down, if it stays in the same half.
    #[must_use]
    pub const fn checked_sub(self, bytes: u64) -> Option<VirtAddr> {
        match self.0.checked_sub(bytes) {
            Some(raw) if is_canonical(raw) && (raw < USER_SPACE_END) == self.is_user() => {
                Some(VirtAddr(raw))
            }
            _ => None,
        }
    }

    /// The offset of the address inside its page.
    #[must_use]
    pub const fn offset_in_page(self) -> u64 {
        self.0 & Alignment::PAGE.mask()
    }

    /// The page containing the address.
    #[must_use]
    pub const fn page(self) -> Page {
        Page(self.align_down(Alignment::PAGE))
    }
}

impl fmt::Debug for VirtAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VirtAddr({:#x})", self.0)
    }
}

impl fmt::Display for VirtAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

/// A page-aligned virtual page.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Page(VirtAddr);

impl Page {
    /// The page starting at `start`; fails if `start` is not page-aligned.
    ///
    /// # Errors
    ///
    /// `Unaligned` if `start` is not a multiple of the page size.
    pub const fn from_start(start: VirtAddr) -> Result<Self, Error> {
        if start.is_aligned(Alignment::PAGE) {
            Ok(Page(start))
        } else {
            Err(Error::Unaligned {
                address: start.0,
                alignment: PAGE_SIZE,
            })
        }
    }

    /// The page containing `address`.
    #[must_use]
    pub const fn containing(address: VirtAddr) -> Self {
        address.page()
    }

    /// The page number (`start / PAGE_SIZE`).
    #[must_use]
    pub const fn number(self) -> u64 {
        self.0.0 >> PAGE_SHIFT
    }

    /// The first address of the page.
    #[must_use]
    pub const fn start(self) -> VirtAddr {
        self.0
    }

    /// The last address of the page.
    #[must_use]
    pub const fn last_address(self) -> VirtAddr {
        VirtAddr(self.0.0 | Alignment::PAGE.mask())
    }

    /// `true` for pages in the user half.
    #[must_use]
    pub const fn is_user(self) -> bool {
        self.0.is_user()
    }

    /// The page `count` pages further up, if it stays in the same half.
    #[must_use]
    pub const fn checked_add(self, count: u64) -> Option<Page> {
        match count.checked_mul(PAGE_SIZE) {
            Some(bytes) => match self.0.checked_add(bytes) {
                Some(start) => Some(Page(start)),
                None => None,
            },
            None => None,
        }
    }

    /// The page `count` pages further down, if it stays in the same half.
    #[must_use]
    pub const fn checked_sub(self, count: u64) -> Option<Page> {
        match count.checked_mul(PAGE_SIZE) {
            Some(bytes) => match self.0.checked_sub(bytes) {
                Some(start) => Some(Page(start)),
                None => None,
            },
            None => None,
        }
    }
}

impl fmt::Debug for Page {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Page({:#x})", self.0.0)
    }
}

/// A range of consecutive pages inside one half of the address space,
/// given by its first page and a count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageRange {
    start: Page,
    count: u64,
}

/// One past the highest page number of the user half.
const USER_END_PAGE: u64 = USER_SPACE_END >> PAGE_SHIFT;

/// One past the highest page number of the kernel half.
const KERNEL_END_PAGE: u64 = 1 << (64 - PAGE_SHIFT);

const _: () = assert!(VIRT_ADDR_BITS == 48);

impl PageRange {
    /// `count` pages starting at `start`; fails if the range would leave the
    /// half of `start`.
    ///
    /// # Errors
    ///
    /// `CrossesCanonicalHole` if a user range would reach the hole;
    /// `Overflow` if a kernel range would wrap around.
    pub const fn new(start: Page, count: u64) -> Result<Self, Error> {
        let limit = if start.is_user() {
            USER_END_PAGE
        } else {
            KERNEL_END_PAGE
        };
        match start.number().checked_add(count) {
            Some(end) if end <= limit => Ok(PageRange { start, count }),
            Some(_) if start.is_user() => Err(Error::CrossesCanonicalHole),
            _ => Err(Error::Overflow),
        }
    }

    /// The pages between two addresses: `start` rounded down and `end`
    /// rounded up, where `end` is exclusive. Fails for `end < start`, for
    /// addresses in different halves, and when rounding overflows.
    ///
    /// # Errors
    ///
    /// `Overflow` for `end < start`; `CrossesCanonicalHole` for addresses in
    /// different halves; the errors of [`PageRange::new`] otherwise.
    pub const fn from_addresses(start: VirtAddr, end: VirtAddr) -> Result<Self, Error> {
        if end.0 < start.0 {
            return Err(Error::Overflow);
        }
        if start.is_user() != end.is_user() {
            return Err(Error::CrossesCanonicalHole);
        }
        let first = start.page();
        let end_page_number = match Alignment::PAGE.align_up(end.0) {
            Some(raw) => raw >> PAGE_SHIFT,
            // `end` lies in the last page of the address space.
            None => KERNEL_END_PAGE,
        };
        Self::new(first, end_page_number.wrapping_sub(first.number()))
    }

    /// The first page.
    #[must_use]
    pub const fn start(self) -> Page {
        self.start
    }

    /// The number of pages.
    #[must_use]
    pub const fn count(self) -> u64 {
        self.count
    }

    /// The number of bytes.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.count << PAGE_SHIFT
    }

    /// `true` if the range holds no page.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.count == 0
    }

    /// `true` for ranges in the user half.
    #[must_use]
    pub const fn is_user(self) -> bool {
        self.start.is_user()
    }

    /// The page number one past the last page.
    #[must_use]
    pub const fn end_number(self) -> u64 {
        self.start.number().wrapping_add(self.count)
    }

    /// The last page, if the range is not empty.
    #[must_use]
    pub const fn last(self) -> Option<Page> {
        if self.count == 0 {
            None
        } else {
            self.start.checked_add(self.count.wrapping_sub(1))
        }
    }

    /// The address one past the last page, if it is canonical (it is not at
    /// the top of either half).
    #[must_use]
    pub const fn end_address(self) -> Option<VirtAddr> {
        match self.start.0.checked_add(self.bytes()) {
            Some(address) => Some(address),
            None => None,
        }
    }

    /// `true` if `page` lies in the range.
    #[must_use]
    pub const fn contains(self, page: Page) -> bool {
        page.number() >= self.start.number() && page.number() < self.end_number()
    }

    /// `true` if the ranges share at least one page.
    #[must_use]
    pub const fn overlaps(self, other: PageRange) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.start.number() < other.end_number()
            && other.start.number() < self.end_number()
    }

    /// Iterates over the pages.
    #[must_use]
    pub const fn iter(self) -> Pages {
        Pages {
            next: self.start.number(),
            end: self.end_number(),
        }
    }
}

impl IntoIterator for PageRange {
    type Item = Page;
    type IntoIter = Pages;

    fn into_iter(self) -> Pages {
        self.iter()
    }
}

/// Iterator over the pages of a [`PageRange`].
#[derive(Clone, Debug)]
pub struct Pages {
    next: u64,
    end: u64,
}

impl Iterator for Pages {
    type Item = Page;

    fn next(&mut self) -> Option<Page> {
        if self.next >= self.end {
            return None;
        }
        let page = VirtAddr::new(self.next << PAGE_SHIFT).ok().map(Page)?;
        self.next = self.next.wrapping_add(1);
        Some(page)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = usize::try_from(self.end.saturating_sub(self.next)).unwrap_or(usize::MAX);
        (remaining, Some(remaining))
    }
}
