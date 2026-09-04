// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Access to page-table memory, TLB control, and frame supply.
//!
//! Invariants: a `FrameAccess` implementation returns a reference to the
//! table stored in exactly the requested frame; a `FrameSource` never hands
//! out a frame twice before it was released.

use kernel_types::{Page, PhysFrame};

/// Reaches the page table stored in a physical frame.
pub trait FrameAccess<T> {
    /// The table in `frame`, or `None` if the frame is not reachable.
    fn table(&self, frame: PhysFrame) -> Option<&T>;

    /// The table in `frame` for modification, or `None` if the frame is not
    /// reachable.
    fn table_mut(&mut self, frame: PhysFrame) -> Option<&mut T>;
}

/// Invalidates translation lookaside buffer entries.
pub trait TlbControl {
    /// Invalidates the translation of one page.
    fn flush_page(&mut self, page: Page);

    /// Invalidates every translation.
    fn flush_all(&mut self);
}

/// Supplies frames for page tables.
pub trait FrameSource {
    /// A frame that is not in use, or `None` if none is left.
    fn allocate_frame(&mut self) -> Option<PhysFrame>;

    /// Returns a frame obtained from `allocate_frame`.
    fn release_frame(&mut self, frame: PhysFrame);
}
