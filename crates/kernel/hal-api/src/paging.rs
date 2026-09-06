// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Access to page-table memory, TLB control, and frame supply.
//!
//! Invariants: a `FrameAccess` implementation returns a reference to the
//! table stored in exactly the requested frame; a `FrameSource` never hands
//! out a frame twice before it was released.

use kernel_types::{Alignment, Page, PhysFrame};

/// The number of bytes of one frame, as the length of an array. A frame is a
/// page, and the alignment of `kernel-types` is what says how large that is.
pub const FRAME_BYTES: usize = 4096;

#[expect(
    clippy::as_conversions,
    reason = "widening an array length to the width of an alignment in a constant"
)]
const _: () = assert!(Alignment::PAGE.bytes() == FRAME_BYTES as u64);

/// Reaches the page table stored in a physical frame.
pub trait FrameAccess<T> {
    /// The table in `frame`, or `None` if the frame is not reachable.
    fn table(&self, frame: PhysFrame) -> Option<&T>;

    /// The table in `frame` for modification, or `None` if the frame is not
    /// reachable.
    fn table_mut(&mut self, frame: PhysFrame) -> Option<&mut T>;
}

/// Reaches the bytes of a physical frame.
///
/// This is the seam the kernel reads and writes an IPC buffer through: a
/// frame the kernel owns, whose contents are bytes and not a structure of
/// the architecture. The implementation of the reference machine is the
/// physical window.
pub trait FrameBytes {
    /// The bytes of `frame`, or `None` if the frame is not reachable.
    fn frame_bytes(&self, frame: PhysFrame) -> Option<&[u8; FRAME_BYTES]>;

    /// The bytes of `frame` for modification, or `None` if the frame is not
    /// reachable.
    ///
    /// The exclusive borrow of the implementation is what keeps two
    /// references to the same frame apart.
    fn frame_bytes_mut(&mut self, frame: PhysFrame) -> Option<&mut [u8; FRAME_BYTES]>;
}

/// Invalidates translation lookaside buffer entries.
pub trait TlbControl {
    /// Invalidates the translation of one page.
    fn flush_page(&mut self, page: Page);

    /// Invalidates every translation.
    fn flush_all(&mut self);
}

/// Loads the page-table root of an address space.
///
/// The kernel changes address spaces only between processes, because
/// loading the root costs the translation lookaside buffer (D-65); the
/// threads of one process switch without touching it.
pub trait AddressSpaceControl {
    /// Makes the address space rooted at `root` the one the processor
    /// translates through.
    fn activate(&mut self, root: PhysFrame);

    /// The root the processor translates through now.
    fn active(&self) -> PhysFrame;
}

/// Supplies frames for page tables.
pub trait FrameSource {
    /// A frame that is not in use, or `None` if none is left.
    fn allocate_frame(&mut self) -> Option<PhysFrame>;

    /// Returns a frame obtained from `allocate_frame`.
    fn release_frame(&mut self, frame: PhysFrame);
}
