// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The image as a device of sectors, which is what `fs-gpt` and `fs-fat`
//! ask for. A whole image is one of these and so is a partition inside
//! it: what the bytes are is the caller's business.

use fs_fat::{BlockDevice, SECTOR};

/// Bytes that are written as well as read.
pub(crate) struct Slice<'a> {
    /// The sectors, one after another.
    pub(crate) bytes: &'a mut [u8],
}

/// Bytes that are only read, which is how the tests look at an image the
/// product wrote.
#[cfg(test)]
pub(crate) struct View<'a> {
    /// The sectors, one after another.
    pub(crate) bytes: &'a [u8],
}

/// Where a sector begins, where the bytes hold it.
fn offset(len: usize, sector: u32) -> Option<usize> {
    let start = usize::try_from(sector).ok()?.checked_mul(SECTOR)?;
    if start.checked_add(SECTOR)? <= len {
        Some(start)
    } else {
        None
    }
}

/// Reads sector `sector` out of `bytes`.
fn read_from(bytes: &[u8], sector: u32, into: &mut [u8; SECTOR]) -> Result<(), fs_fat::Error> {
    let start = offset(bytes.len(), sector).ok_or(fs_fat::Error::Device(sector))?;
    let source = bytes
        .get(start..start.saturating_add(SECTOR))
        .ok_or(fs_fat::Error::Device(sector))?;
    into.copy_from_slice(source);
    Ok(())
}

/// How many whole sectors `bytes` holds.
fn sectors_in(bytes: &[u8]) -> u32 {
    u32::try_from(bytes.len() / SECTOR).unwrap_or(u32::MAX)
}

impl BlockDevice for Slice<'_> {
    fn sectors(&self) -> u32 {
        sectors_in(self.bytes)
    }

    fn read(&self, sector: u32, into: &mut [u8; SECTOR]) -> Result<(), fs_fat::Error> {
        read_from(self.bytes, sector, into)
    }

    fn write(&mut self, sector: u32, from: &[u8; SECTOR]) -> Result<(), fs_fat::Error> {
        let start = offset(self.bytes.len(), sector).ok_or(fs_fat::Error::Device(sector))?;
        let target = self
            .bytes
            .get_mut(start..start.saturating_add(SECTOR))
            .ok_or(fs_fat::Error::Device(sector))?;
        target.copy_from_slice(from);
        Ok(())
    }
}

#[cfg(test)]
impl BlockDevice for View<'_> {
    fn sectors(&self) -> u32 {
        sectors_in(self.bytes)
    }

    fn read(&self, sector: u32, into: &mut [u8; SECTOR]) -> Result<(), fs_fat::Error> {
        read_from(self.bytes, sector, into)
    }

    fn write(&mut self, sector: u32, _from: &[u8; SECTOR]) -> Result<(), fs_fat::Error> {
        Err(fs_fat::Error::Device(sector))
    }
}
