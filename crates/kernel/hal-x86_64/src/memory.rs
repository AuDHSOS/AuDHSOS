// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the memory bring-up needs from the machine: the frame the active
//! tables are rooted in, the frame a kernel address translates to, and the
//! bytes of a frame read through the window.
//!
//! Invariant: every function here runs while the tables the loader built
//! are active, so the window maps all of memory and the boot information
//! page is where the layout says.

use audhsos_abi::layout::{BOOT_INFO_VADDR, PHYS_WINDOW_BASE};
use kernel_mm::frame_allocator::NoFrames;
use kernel_mm::mapper::Mapper;
use kernel_mm::page_table::X86Entry;
use kernel_types::{Alignment, Page, PhysAddr, PhysFrame, VirtAddr};

use crate::bootinfo::X86Platform;
use crate::paging::{LocalTlb, active_root};
use crate::window::{FRAME_BYTES, PhysicalWindow};

/// The frame a kernel address is mapped to in the active tables, or `None`
/// if nothing is mapped there.
///
/// # Safety
///
/// The window must map every physical frame read and write, which is what
/// the tables the loader built do.
#[must_use]
pub unsafe fn translate(address: u64) -> Option<PhysFrame> {
    let page = VirtAddr::new(address).and_then(Page::from_start).ok()?;
    let root = active_root().ok()?;
    // SAFETY: the caller promises that the window maps all of memory, and
    // the walk below is the only user of it while it lives.
    let mut window = unsafe { PhysicalWindow::kernel() };
    let mut tlb = LocalTlb::new();
    let mut frames = NoFrames::new();
    let mapper = Mapper::<'_, X86Entry, _, _, _>::new(root, &mut window, &mut tlb, &mut frames);
    mapper.translate(page).map(|(frame, _)| frame)
}

/// Adds the boot information page to the regions of `platform`, with the
/// physical address the walk of the loader's tables gives.
///
/// Returns `false` if the page has no translation or the region array is
/// full; the kernel reports what it has either way.
///
/// # Safety
///
/// The same as [`translate`].
pub unsafe fn register_boot_info(platform: &mut X86Platform) -> bool {
    // SAFETY: the caller promises that the window maps all of memory.
    let Some(frame) = (unsafe { translate(BOOT_INFO_VADDR) }) else {
        return false;
    };
    platform.push_boot_info(frame.start())
}

/// The bytes at `address`, as a slice through the physical window.
///
/// This is what a caller needs when the bytes span more than one frame —
/// the root task inside the boot image, say. The window maps physical
/// memory contiguously, so the frames of a physical range are contiguous
/// virtual bytes too.
///
/// # Safety
///
/// The window must map all of memory, which it does once the memory
/// bring-up has run, and `address .. address + len` must lie inside a
/// region the loader reported. Nothing may write those bytes while the
/// slice lives.
#[must_use]
pub unsafe fn physical_slice(address: PhysAddr, len: u64) -> Option<&'static [u8]> {
    let base = PHYS_WINDOW_BASE.checked_add(address.as_u64())?;
    let start = usize::try_from(base).ok()?;
    let count = usize::try_from(len).ok()?;
    let pointer = core::ptr::without_provenance::<u8>(start);
    // SAFETY: the caller promises the window maps the range and that
    // nothing writes it, so the bytes are readable and unchanging for as
    // long as the kernel runs.
    Some(unsafe { core::slice::from_raw_parts(pointer, count) })
}

/// The bytes of the frame holding `address`, copied out of the window.
///
/// # Safety
///
/// The same as [`translate`].
#[must_use]
pub unsafe fn read_frame(address: PhysAddr) -> Option<[u8; FRAME_BYTES]> {
    let frame = PhysFrame::from_start(address.align_down(Alignment::PAGE)).ok()?;
    // SAFETY: the caller promises that the window maps all of memory, and
    // the copy below is the only user of it while it lives.
    let mut window = unsafe { PhysicalWindow::kernel() };
    window.frame_bytes_mut(frame).map(|bytes| *bytes)
}
