// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the kernel's mapper needs from the loader: page-table memory
//! through the identity mapping, frames out of one contiguous pool, and a
//! translation lookaside buffer that has nothing to invalidate.
//!
//! Invariants: the pool hands every frame out at most once, so no two
//! tables share memory; the tables the loader builds are never active
//! while it edits them, which is why the flushes are empty.

use kernel_hal_api::paging::{FrameAccess, FrameSource, TlbControl};
use kernel_mm::page_table::{PageTable, X86Entry};
use kernel_types::{Page, PhysFrame, PhysFrameRange};

/// Reaches a page table through the identity mapping the firmware set up.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct IdentityAccess;

impl IdentityAccess {
    /// The access through the identity mapping.
    pub(crate) const fn new() -> Self {
        IdentityAccess
    }
}

impl FrameAccess<PageTable<X86Entry>> for IdentityAccess {
    fn table(&self, frame: PhysFrame) -> Option<&PageTable<X86Entry>> {
        let address = usize::try_from(frame.start().as_u64()).ok()?;
        let pointer = core::ptr::without_provenance::<PageTable<X86Entry>>(address);
        // SAFETY: the firmware identity maps every byte of memory while
        // boot services run and the loader keeps that mapping afterwards;
        // a frame start is aligned to the frame size, which is the
        // alignment of a page table.
        Some(unsafe { &*pointer })
    }

    fn table_mut(&mut self, frame: PhysFrame) -> Option<&mut PageTable<X86Entry>> {
        let address = usize::try_from(frame.start().as_u64()).ok()?;
        let pointer = core::ptr::without_provenance_mut::<PageTable<X86Entry>>(address);
        // SAFETY: the same mapping as in `table`, and the loader is the
        // only writer of the frames the pool hands out, so no other
        // reference to this table exists.
        Some(unsafe { &mut *pointer })
    }
}

/// Hands out the frames of one contiguous range, lowest first.
///
/// The range is one firmware allocation, so that the boot information can
/// report the page tables as a single physical range.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PoolFrames {
    range: PhysFrameRange,
    used: u64,
}

impl PoolFrames {
    /// A source over `range`.
    pub(crate) const fn new(range: PhysFrameRange) -> Self {
        PoolFrames { range, used: 0 }
    }
}

impl FrameSource for PoolFrames {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        if self.used >= self.range.count() {
            return None;
        }
        let frame = self.range.start().checked_add(self.used)?;
        self.used = self.used.saturating_add(1);
        Some(frame)
    }

    fn release_frame(&mut self, frame: PhysFrame) {
        let last = self.used.checked_sub(1);
        if last.and_then(|last| self.range.start().checked_add(last)) == Some(frame) {
            self.used = self.used.saturating_sub(1);
        }
    }
}

/// The lookaside buffer of a processor whose tables are not active yet.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NoTlb;

impl NoTlb {
    /// The buffer that has nothing to invalidate.
    pub(crate) const fn new() -> Self {
        NoTlb
    }
}

impl TlbControl for NoTlb {
    fn flush_page(&mut self, _page: Page) {}

    fn flush_all(&mut self) {}
}

/// The number of frames the tables of a mapping of `bytes` bytes need, the
/// root table excluded: one table per 2 MiB, per 1 GiB, and per 512 GiB.
pub(crate) const fn tables_for(bytes: u64) -> u64 {
    let level1 = bytes.div_ceil(1 << 21);
    let level2 = bytes.div_ceil(1 << 30);
    let level3 = bytes.div_ceil(1 << 39);
    level1.saturating_add(level2).saturating_add(level3)
}

/// The number of frames the pool needs for a machine whose highest
/// physical address is `highest`: the root, the tables of the physical
/// window, the tables of the identity mapping, and a reserve for the
/// kernel image, the boot stack, and the boot information page.
pub(crate) const fn pool_frames(highest: u64) -> u64 {
    let mapping = tables_for(highest);
    1u64.saturating_add(mapping.saturating_mul(2))
        .saturating_add(RESERVE_FRAMES)
}

/// Frames the pool keeps for the mappings that are not the physical window
/// or the identity mapping.
const RESERVE_FRAMES: u64 = 24;
