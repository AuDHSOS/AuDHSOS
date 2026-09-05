// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Reaching page-table memory through the physical window, and the
//! translation lookaside buffer.
//!
//! Invariants: the window maps every physical frame at
//! `PHYS_WINDOW_BASE + address`, read and write, for the whole run; the
//! kernel is the only writer of page-table memory.

use audhsos_abi::layout::PHYS_WINDOW_BASE;
use kernel_hal_api::paging::{FrameAccess, TlbControl};
use kernel_types::{Page, PhysFrame};

/// Reaches a frame through the physical memory window.
#[derive(Clone, Copy, Debug)]
pub struct WindowAccess {
    base: u64,
}

impl WindowAccess {
    /// An access through the window at `base`.
    ///
    /// # Safety
    ///
    /// The window must map every physical frame at `base + address`, read
    /// and write, for as long as this value exists, and the kernel must be
    /// the only writer of the memory it hands out.
    #[must_use]
    pub const unsafe fn new(base: u64) -> Self {
        WindowAccess { base }
    }

    /// An access through the window the layout names.
    ///
    /// # Safety
    ///
    /// The same as [`WindowAccess::new`].
    #[must_use]
    pub const unsafe fn kernel() -> Self {
        // SAFETY: the caller promises what `new` requires.
        unsafe { Self::new(PHYS_WINDOW_BASE) }
    }

    /// The virtual address `frame` is reachable at.
    const fn address_of(self, frame: PhysFrame) -> Option<u64> {
        self.base.checked_add(frame.start().as_u64())
    }
}

impl<T> FrameAccess<T> for WindowAccess {
    fn table(&self, frame: PhysFrame) -> Option<&T> {
        let address = self.address_of(frame)?;
        let pointer = core::ptr::without_provenance::<T>(usize::try_from(address).ok()?);
        // SAFETY: the constructor promises that the window maps the frame
        // for the lifetime of this value, and that the kernel is the only
        // writer, so the reference is valid and nobody mutates through it.
        Some(unsafe { &*pointer })
    }

    fn table_mut(&mut self, frame: PhysFrame) -> Option<&mut T> {
        let address = self.address_of(frame)?;
        let pointer = core::ptr::without_provenance_mut::<T>(usize::try_from(address).ok()?);
        // SAFETY: the constructor promises that the window maps the frame
        // read and write for the lifetime of this value, and that the
        // kernel is the only writer, so no other reference exists.
        Some(unsafe { &mut *pointer })
    }
}

/// Invalidates translations on this processor.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalTlb;

impl LocalTlb {
    /// The lookaside buffer of this processor.
    #[must_use]
    pub const fn new() -> Self {
        LocalTlb
    }
}

impl TlbControl for LocalTlb {
    fn flush_page(&mut self, page: Page) {
        // SAFETY: the caller changed the tables before asking for the
        // flush, which is what `invalidate_page` requires.
        unsafe {
            crate::instructions::invalidate_page(page.start().as_u64());
        }
    }

    fn flush_all(&mut self) {
        let root = crate::instructions::read_page_table_root();
        // SAFETY: writing the root that is already active flushes every
        // translation that is not global and changes no mapping.
        unsafe {
            crate::instructions::write_page_table_root(root);
        }
    }
}
