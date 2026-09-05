// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The physical memory window: every frame of the machine, reachable at a
//! fixed offset in the kernel half.
//!
//! Invariants: the window maps every physical frame at
//! `base + address`, read and write, for the whole run; the kernel is the
//! only writer of the memory the window hands out; a frame start is
//! aligned to the frame size, which is at least the alignment of every
//! type this module hands out a reference to.

use audhsos_abi::layout::{PAGE_SIZE, PHYS_WINDOW_BASE};
use kernel_hal_api::paging::FrameAccess;
use kernel_types::{PhysFrame, VirtAddr};

/// The number of bytes of one frame, as the length of an array.
pub const FRAME_BYTES: usize = 4096;

const _: () = assert!(PAGE_SIZE == 4096);

/// Reaches physical memory through the window.
#[derive(Clone, Copy, Debug)]
pub struct PhysicalWindow {
    base: VirtAddr,
}

impl PhysicalWindow {
    /// A window at `base`.
    ///
    /// # Safety
    ///
    /// The window must map every physical frame at `base + address`, read
    /// and write, for as long as this value exists, and the kernel must be
    /// the only writer of the memory it hands out.
    #[must_use]
    pub const unsafe fn new(base: VirtAddr) -> Self {
        PhysicalWindow { base }
    }

    /// The window the layout names.
    ///
    /// # Safety
    ///
    /// The same as [`PhysicalWindow::new`].
    #[must_use]
    pub const unsafe fn kernel() -> Self {
        let base = match VirtAddr::new(PHYS_WINDOW_BASE) {
            Ok(base) => base,
            Err(_) => VirtAddr::ZERO,
        };
        // SAFETY: the caller promises what `new` requires.
        unsafe { Self::new(base) }
    }

    /// The virtual base of the window.
    #[must_use]
    pub const fn base(self) -> VirtAddr {
        self.base
    }

    /// The address `frame` is reachable at.
    const fn address_of(self, frame: PhysFrame) -> Option<u64> {
        self.base.as_u64().checked_add(frame.start().as_u64())
    }

    /// A pointer to the value of type `T` stored in `frame`.
    fn pointer<T>(self, frame: PhysFrame) -> Option<*mut T> {
        let address = self.address_of(frame)?;
        Some(core::ptr::without_provenance_mut::<T>(
            usize::try_from(address).ok()?,
        ))
    }

    /// The bytes of `frame`, for reading and writing.
    ///
    /// The borrow of `self` is what keeps two references to the same frame
    /// apart: a second call cannot happen while the first result lives.
    pub fn frame_bytes_mut(&mut self, frame: PhysFrame) -> Option<&mut [u8; FRAME_BYTES]> {
        let pointer = self.pointer::<[u8; FRAME_BYTES]>(frame)?;
        // SAFETY: the constructor promises that the window maps the frame
        // read and write for the lifetime of this value and that the
        // kernel is the only writer; the exclusive borrow of the window
        // means no other reference obtained from it is alive.
        Some(unsafe { &mut *pointer })
    }
}

impl<T> FrameAccess<T> for PhysicalWindow {
    fn table(&self, frame: PhysFrame) -> Option<&T> {
        let pointer = self.pointer::<T>(frame)?;
        // SAFETY: the constructor promises that the window maps the frame
        // for the lifetime of this value, and that the kernel is the only
        // writer, so the reference is valid and nobody mutates through it.
        Some(unsafe { &*pointer })
    }

    fn table_mut(&mut self, frame: PhysFrame) -> Option<&mut T> {
        let pointer = self.pointer::<T>(frame)?;
        // SAFETY: the constructor promises that the window maps the frame
        // read and write for the lifetime of this value, and that the
        // kernel is the only writer, so no other reference exists.
        Some(unsafe { &mut *pointer })
    }
}
