// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The one conversion from a physical range the firmware allocated into a
//! byte slice.
//!
//! Invariant: while boot services run the firmware maps every byte of
//! physical memory at its own address, and the loader keeps that identity
//! mapping in its own tables, so a physical address is a valid pointer for
//! the whole run of the loader.

use kernel_types::{PhysAddr, PhysFrameRange};

/// The bytes of `range`, at most `len` of them.
///
/// # Safety
///
/// The range must have come from `Firmware::allocate_pages`, nothing else
/// may hold a reference to it, and the identity mapping must still be
/// active.
pub(crate) unsafe fn bytes_mut<'a>(range: PhysFrameRange, len: u64) -> Option<&'a mut [u8]> {
    if len > range.bytes() {
        return None;
    }
    let address = usize::try_from(range.start().start().as_u64()).ok()?;
    let len = usize::try_from(len).ok()?;
    let pointer = core::ptr::without_provenance_mut::<u8>(address);
    // SAFETY: the caller promises that the frames are allocated, identity
    // mapped, and not borrowed anywhere else; `len` was checked against
    // the length of the range, so the slice stays inside it.
    Some(unsafe { core::slice::from_raw_parts_mut(pointer, len) })
}

/// The first frame of `range` as a physical address.
pub(crate) const fn start_of(range: PhysFrameRange) -> PhysAddr {
    range.start().start()
}
