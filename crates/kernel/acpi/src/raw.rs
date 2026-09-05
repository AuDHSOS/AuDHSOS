// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Reading fixed-width little-endian fields out of table bytes.
//!
//! Invariant: every function here returns `None` rather than reading past
//! the end of the slice, so a truncated table is an error and never a
//! panic.

/// The `N` bytes at `offset`.
#[must_use]
pub fn array_at<const N: usize>(bytes: &[u8], offset: usize) -> Option<[u8; N]> {
    bytes.get(offset..)?.first_chunk::<N>().copied()
}

/// The byte at `offset`.
#[must_use]
pub fn u8_at(bytes: &[u8], offset: usize) -> Option<u8> {
    bytes.get(offset).copied()
}

/// The little-endian `u16` at `offset`.
#[must_use]
pub fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    array_at::<2>(bytes, offset).map(u16::from_le_bytes)
}

/// The little-endian `u32` at `offset`.
#[must_use]
pub fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    array_at::<4>(bytes, offset).map(u32::from_le_bytes)
}

/// The little-endian `u64` at `offset`.
#[must_use]
pub fn u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
    array_at::<8>(bytes, offset).map(u64::from_le_bytes)
}

/// The sum of the first `len` bytes modulo 256, which every ACPI table
/// defines as zero over the length it announces. Fewer bytes than `len`
/// are summed as they are, so the caller checks the length itself.
#[must_use]
pub fn sum_of(bytes: &[u8], len: usize) -> u8 {
    bytes
        .iter()
        .take(len)
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte))
}
