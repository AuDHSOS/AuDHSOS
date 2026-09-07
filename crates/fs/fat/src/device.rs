// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The one thing this crate asks of the world: a sector in and a sector
//! out.

use crate::error::Error;

/// Bytes in one sector. FAT allows others; this crate reads 512, which is
/// what every layout constant here counts in.
pub const SECTOR: usize = 512;

/// A device that moves whole sectors of [`SECTOR`] bytes.
///
/// The trait says nothing about where the bytes are. An implementation
/// over a byte slice is what a test and an image writer use; a driver is
/// what a file system server will use, and neither the chains nor the
/// directories of this crate can tell them apart.
pub trait BlockDevice {
    /// How many sectors the device has. A sector at or above this number
    /// is not readable and not writable.
    fn sectors(&self) -> u32;

    /// Reads one sector.
    ///
    /// # Errors
    ///
    /// [`Error::Device`] for a sector the device does not have, or one it
    /// would not move.
    fn read(&self, sector: u32, into: &mut [u8; SECTOR]) -> Result<(), Error>;

    /// Writes one sector.
    ///
    /// # Errors
    ///
    /// [`Error::Device`] for a sector the device does not have, or one it
    /// would not move.
    fn write(&mut self, sector: u32, from: &[u8; SECTOR]) -> Result<(), Error>;
}

impl<D: BlockDevice + ?Sized> BlockDevice for &mut D {
    fn sectors(&self) -> u32 {
        (**self).sectors()
    }

    fn read(&self, sector: u32, into: &mut [u8; SECTOR]) -> Result<(), Error> {
        (**self).read(sector, into)
    }

    fn write(&mut self, sector: u32, from: &[u8; SECTOR]) -> Result<(), Error> {
        (**self).write(sector, from)
    }
}

/// The two-byte number at `offset`, or zero where the sector ends first.
pub(crate) fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    bytes
        .get(offset..offset.saturating_add(2))
        .and_then(|slice| slice.try_into().ok())
        .map_or(0, u16::from_le_bytes)
}

/// The four-byte number at `offset`, or zero where the sector ends first.
pub(crate) fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    bytes
        .get(offset..offset.saturating_add(4))
        .and_then(|slice| slice.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}

/// Puts `value` at `offset`, and nothing where the sector ends first.
pub(crate) fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    if let Some(slot) = offset
        .checked_add(value.len())
        .and_then(|end| bytes.get_mut(offset..end))
    {
        slot.copy_from_slice(value);
    }
}
