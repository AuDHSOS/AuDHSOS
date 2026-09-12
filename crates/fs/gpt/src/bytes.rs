// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Little-endian numbers in and out of a sector. Every one of them
//! answers zero, or writes nothing, where the sector ends first, so that
//! no offset of this crate can index past its buffer.

/// The four-byte number at `offset`, or zero where the bytes end first.
#[must_use]
pub fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    bytes
        .get(offset..offset.saturating_add(4))
        .and_then(|slice| slice.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}

/// The eight-byte number at `offset`, or zero where the bytes end first.
#[must_use]
pub fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    bytes
        .get(offset..offset.saturating_add(8))
        .and_then(|slice| slice.try_into().ok())
        .map_or(0, u64::from_le_bytes)
}

/// The sixteen bytes at `offset`, or all zeros where the bytes end first.
#[must_use]
pub fn read_guid(bytes: &[u8], offset: usize) -> [u8; 16] {
    bytes
        .get(offset..offset.saturating_add(16))
        .and_then(|slice| slice.try_into().ok())
        .unwrap_or([0u8; 16])
}

/// Puts `value` at `offset`, and nothing where the bytes end first.
pub fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    if let Some(slot) = offset
        .checked_add(value.len())
        .and_then(|end| bytes.get_mut(offset..end))
    {
        slot.copy_from_slice(value);
    }
}
