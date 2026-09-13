// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Reading numbers out of a page. Every number of the format is
//! big-endian, and every reader answers `None` where the bytes end first,
//! so no read of a short page indexes past it.

use crate::error::Error;

/// The byte at `offset`.
pub(crate) fn u8_at(bytes: &[u8], offset: usize) -> Option<u8> {
    bytes.get(offset).copied()
}

/// The two-byte number at `offset`.
pub(crate) fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    bytes
        .get(offset..end)
        .and_then(|slice| slice.try_into().ok())
        .map(u16::from_be_bytes)
}

/// The four-byte number at `offset`.
pub(crate) fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    bytes
        .get(offset..end)
        .and_then(|slice| slice.try_into().ok())
        .map(u32::from_be_bytes)
}

/// A variable-length integer, and how many bytes it took.
///
/// The format's varint is big-endian: the first eight bytes carry seven
/// bits each under a continuation bit, and a ninth byte carries all eight
/// of its bits, so nine bytes carry the whole sixty-four. O(1), at most
/// nine steps.
pub(crate) fn varint(bytes: &[u8]) -> Result<(u64, usize), Error> {
    let mut value: u64 = 0;
    for at in 0..8 {
        let byte = bytes.get(at).copied().ok_or(Error::Varint)?;
        value = value.wrapping_shl(7) | u64::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Ok((value, at.saturating_add(1)));
        }
    }
    let last = bytes.get(8).copied().ok_or(Error::Varint)?;
    Ok((value.wrapping_shl(8) | u64::from(last), 9))
}

/// The same value read as a signed one, which is what a rowid is.
pub(crate) const fn signed(value: u64) -> i64 {
    i64::from_le_bytes(value.to_le_bytes())
}

/// A `usize` from a `u64` of the file. A number wider than this machine
/// holds saturates, and every reader that uses one asks the bytes for it
/// afterwards, so a length no file can hold is refused where it is used
/// rather than where it is read.
pub(crate) fn size(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}
