// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The bound of a volatile access to a device window (D-113).
//!
//! `user-sys-x86_64` makes each access through the offset this module
//! answers, so the precondition of its `unsafe` blocks is tested here, on
//! the host.

/// The offset of a `width`-byte access at `offset` of the `len` bytes that
/// start at address `base`.
///
/// `None` when the access does not lie whole inside the window, when
/// `base + offset` is not a multiple of `width` (issue #99), or when
/// `width` is zero.
#[must_use]
pub fn checked_offset(base: usize, len: usize, offset: usize, width: usize) -> Option<usize> {
    let end = offset.checked_add(width)?;
    if end > len {
        return None;
    }
    let address = base.checked_add(offset)?;
    if address.checked_rem(width)? != 0 {
        return None;
    }
    Some(offset)
}
