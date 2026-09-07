// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A block device made of nothing but memory, and one that can be told to
//! refuse a sector.

use std::vec;
use std::vec::Vec;

use crate::device::{BlockDevice, SECTOR};
use crate::error::Error;

/// A device whose sectors are a byte vector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RamDisk {
    /// The sectors, one after another.
    bytes: Vec<u8>,
    /// A sector the device will not move, so that a caller can see what a
    /// failing device does to an operation.
    refused: Option<u32>,
}

impl RamDisk {
    /// A device of `sectors` zeroed sectors.
    #[must_use]
    pub fn new(sectors: u32) -> RamDisk {
        let len = usize::try_from(sectors).unwrap_or(0).saturating_mul(SECTOR);
        RamDisk {
            bytes: vec![0u8; len],
            refused: None,
        }
    }

    /// A device over bytes that are already there. The length is rounded
    /// down to whole sectors.
    #[must_use]
    pub const fn from_bytes(bytes: Vec<u8>) -> RamDisk {
        RamDisk {
            bytes,
            refused: None,
        }
    }

    /// The bytes of every sector.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The bytes of every sector, to be written by hand — which is how a
    /// test builds a volume that no writer of this crate would produce.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    /// From now on the device refuses `sector`.
    pub const fn refuse(&mut self, sector: u32) {
        self.refused = Some(sector);
    }

    /// Where a sector begins, where the device has it.
    fn offset(&self, sector: u32) -> Option<usize> {
        if self.refused == Some(sector) {
            return None;
        }
        let start = usize::try_from(sector).ok()?.checked_mul(SECTOR)?;
        let end = start.checked_add(SECTOR)?;
        if end <= self.bytes.len() {
            Some(start)
        } else {
            None
        }
    }
}

impl BlockDevice for RamDisk {
    fn sectors(&self) -> u32 {
        u32::try_from(self.bytes.len().wrapping_div(SECTOR)).unwrap_or(u32::MAX)
    }

    fn read(&self, sector: u32, into: &mut [u8; SECTOR]) -> Result<(), Error> {
        let start = self.offset(sector).ok_or(Error::Device(sector))?;
        let source = self
            .bytes
            .get(start..start.saturating_add(SECTOR))
            .ok_or(Error::Device(sector))?;
        into.copy_from_slice(source);
        Ok(())
    }

    fn write(&mut self, sector: u32, from: &[u8; SECTOR]) -> Result<(), Error> {
        let start = self.offset(sector).ok_or(Error::Device(sector))?;
        let target = self
            .bytes
            .get_mut(start..start.saturating_add(SECTOR))
            .ok_or(Error::Device(sector))?;
        target.copy_from_slice(from);
        Ok(())
    }
}
