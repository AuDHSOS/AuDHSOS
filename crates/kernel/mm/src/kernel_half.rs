// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What every address space shares: the upper half.
//!
//! Invariants: a root this module prepares carries exactly the top-level
//! entries of the kernel and nothing of the user half; the entries are
//! copied, not the tables below them, so a kernel mapping made after a
//! root was built — a kernel stack, above all — is reachable through every
//! address space at once.
//!
//! That holds because the kernel occupies its top-level entries before it
//! creates the first address space and never gains another: the window
//! lies inside one entry, and the stacks, the boot information page, and
//! the image inside another (D-65). A test in QEMU holds the kernel to it.

use audhsos_abi::layout::{KERNEL_SPACE_START, PAGE_SIZE};
use kernel_hal_api::paging::{FrameAccess, FrameSource};
use kernel_types::PhysFrame;

use crate::page_table::{EntryFormat, PageTable};

/// Number of entries in a top-level page table.
pub const ENTRIES: usize = 512;

/// The first entry of the upper half: everything from here on belongs to
/// the kernel and is shared by every address space.
pub const FIRST_KERNEL_ENTRY: usize = 256;

const _: () = assert!(
    KERNEL_SPACE_START == 0xFFFF_8000_0000_0000,
    "the upper half starts at entry 256 of the top-level table"
);

/// Why the kernel half could not be shared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShareError {
    /// One of the two roots is not reachable.
    Unreachable(PhysFrame),
}

impl From<ShareError> for audhsos_abi::Error {
    fn from(error: ShareError) -> Self {
        match error {
            ShareError::Unreachable(_) => audhsos_abi::Error::OutOfKernelMemory,
        }
    }
}

/// Copies the top-level entries of the kernel from `kernel` into `target`
/// and returns how many entries carry something.
///
/// The lower half of `target` is left as it is, which for a fresh frame is
/// empty.
///
/// # Errors
///
/// [`ShareError::Unreachable`] when a root is not reachable through
/// `access`.
pub fn share<F, A>(
    access: &mut A,
    kernel: PhysFrame,
    target: PhysFrame,
) -> Result<usize, ShareError>
where
    F: EntryFormat + Copy,
    A: FrameAccess<PageTable<F>>,
{
    let mut entries = [F::EMPTY; ENTRIES];
    {
        let table = access
            .table(kernel)
            .ok_or(ShareError::Unreachable(kernel))?;
        for (index, slot) in entries.iter_mut().enumerate() {
            *slot = table.entry(index);
        }
    }
    let table = access
        .table_mut(target)
        .ok_or(ShareError::Unreachable(target))?;
    let mut shared: usize = 0;
    for (index, entry) in entries.iter().enumerate().skip(FIRST_KERNEL_ENTRY) {
        if entry.is_present() {
            shared = shared.saturating_add(1);
        }
        table.set_entry(index, *entry);
    }
    Ok(shared)
}

/// The top-level entries of the kernel half that carry something, as their
/// indices.
///
/// # Errors
///
/// [`ShareError::Unreachable`] when the root is not reachable.
pub fn kernel_entries<F, A>(access: &A, root: PhysFrame) -> Result<[bool; ENTRIES], ShareError>
where
    F: EntryFormat + Copy,
    A: FrameAccess<PageTable<F>>,
{
    let table = access.table(root).ok_or(ShareError::Unreachable(root))?;
    let mut present = [false; ENTRIES];
    for (index, slot) in present.iter_mut().enumerate().skip(FIRST_KERNEL_ENTRY) {
        *slot = table.entry(index).is_present();
    }
    Ok(present)
}

/// `true` if `target` carries every kernel entry of `kernel`.
///
/// # Errors
///
/// [`ShareError::Unreachable`] when a root is not reachable.
pub fn shares_kernel_half<F, A>(
    access: &A,
    kernel: PhysFrame,
    target: PhysFrame,
) -> Result<bool, ShareError>
where
    F: EntryFormat + Copy + PartialEq,
    A: FrameAccess<PageTable<F>>,
{
    let expected = access
        .table(kernel)
        .ok_or(ShareError::Unreachable(kernel))?;
    let mut wanted = [F::EMPTY; ENTRIES];
    for (index, slot) in wanted.iter_mut().enumerate() {
        *slot = expected.entry(index);
    }
    let table = access
        .table(target)
        .ok_or(ShareError::Unreachable(target))?;
    for (index, entry) in wanted.iter().enumerate().skip(FIRST_KERNEL_ENTRY) {
        if table.entry(index) != *entry {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The index of the top-level entry an address falls into.
#[must_use]
pub const fn entry_of(address: u64) -> usize {
    // A top-level entry covers 512 GiB: nine bits of the address, starting
    // at bit thirty-nine.
    #[expect(
        clippy::as_conversions,
        reason = "the mask keeps nine bits, which every index width holds, and the function is const"
    )]
    let index = ((address >> 39) & 0x1FF) as usize;
    index
}

const _: () = assert!(entry_of(KERNEL_SPACE_START) == FIRST_KERNEL_ENTRY);
const _: () = assert!(entry_of(0) == 0);
const _: () = assert!(entry_of(u64::MAX) == ENTRIES - 1);
const _: () = assert!(PAGE_SIZE == 4096);

/// Gives back every page table of the user half of `root`, and the root
/// itself, and returns how many frames went back.
///
/// The frames the leaves point at are not touched: they belong to the
/// memory objects the mappings were made from, and those outlive the
/// address space. Only what the reserve lent for the tables goes back.
///
/// The kernel half is shared by every address space, so the tables below
/// its entries are never followed.
pub fn free_user_half<F, A, S>(access: &mut A, frames: &mut S, root: PhysFrame) -> u64
where
    F: EntryFormat + Copy,
    A: FrameAccess<PageTable<F>>,
    S: FrameSource,
{
    let mut freed = 0_u64;
    let top = F::LEVELS.saturating_sub(1);
    for index in 0..FIRST_KERNEL_ENTRY {
        let Some(entry) = access.table(root).map(|table| table.entry(index)) else {
            break;
        };
        if !entry.is_present() {
            continue;
        }
        let Some(child) = entry.frame() else {
            continue;
        };
        freed = freed.saturating_add(free_subtree(access, frames, child, top.saturating_sub(1)));
        if let Some(table) = access.table_mut(root) {
            table.set_entry(index, F::EMPTY);
        }
    }
    frames.release_frame(root);
    freed.saturating_add(1)
}

/// Gives back the table in `frame` and everything below it. `level` is the
/// level of that table, counted the way [`EntryFormat::index`] counts:
/// level zero holds the leaves, whose frames are left alone.
fn free_subtree<F, A, S>(access: &mut A, frames: &mut S, frame: PhysFrame, level: usize) -> u64
where
    F: EntryFormat + Copy,
    A: FrameAccess<PageTable<F>>,
    S: FrameSource,
{
    let mut freed = 0_u64;
    if level > 0 {
        for index in 0..F::ENTRIES {
            let Some(entry) = access.table(frame).map(|table| table.entry(index)) else {
                break;
            };
            let Some(child) = entry.frame().filter(|_| entry.is_present()) else {
                continue;
            };
            freed =
                freed.saturating_add(free_subtree(access, frames, child, level.saturating_sub(1)));
        }
    }
    if let Some(table) = access.table_mut(frame) {
        *table = PageTable::new();
    }
    frames.release_frame(frame);
    freed.saturating_add(1)
}
