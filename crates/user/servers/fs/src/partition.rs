// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One partition of a disk, as a device of its own.
//!
//! A disk that carries a partition table holds its volume inside a
//! partition, not at sector zero, so `fs-fat` is handed a device whose
//! first sector is the partition's first sector. The whole of the
//! adapter is one addition and one bound.
//!
//! Invariant: a sector at or above the partition's length is refused, so
//! nothing reached through this touches a sector of another partition or
//! of the table itself.

use fs_fat::{BlockDevice, Error, SECTOR};

/// A window over the sectors of one partition.
#[derive(Clone, Copy, Debug)]
pub struct Partition<D> {
    device: D,
    first: u32,
    sectors: u32,
}

impl<D: BlockDevice> Partition<D> {
    /// The partition of `device` from `first_lba` to `last_lba`, the last
    /// itself included, as a GPT entry names them.
    ///
    /// Answers `None` for a partition that ends before it starts, or that
    /// reaches past the device, or whose numbers are no sector count of
    /// this system.
    #[must_use]
    pub fn new(device: D, first_lba: u64, last_lba: u64) -> Option<Self> {
        let first = u32::try_from(first_lba).ok()?;
        let last = u32::try_from(last_lba).ok()?;
        let sectors = last.checked_sub(first)?.checked_add(1)?;
        (last < device.sectors()).then_some(Partition {
            device,
            first,
            sectors,
        })
    }

    /// The disk the partition lies on.
    pub const fn device(&self) -> &D {
        &self.device
    }

    /// The sector of the disk that `sector` of the partition is, or
    /// `None` for one the partition does not hold.
    fn at(&self, sector: u32) -> Option<u32> {
        (sector < self.sectors).then_some(self.first.saturating_add(sector))
    }
}

impl<D: BlockDevice> BlockDevice for Partition<D> {
    fn sectors(&self) -> u32 {
        self.sectors
    }

    fn read(&self, sector: u32, into: &mut [u8; SECTOR]) -> Result<(), Error> {
        let at = self.at(sector).ok_or(Error::Device(sector))?;
        self.device.read(at, into)
    }

    fn write(&mut self, sector: u32, from: &[u8; SECTOR]) -> Result<(), Error> {
        let at = self.at(sector).ok_or(Error::Device(sector))?;
        self.device.write(at, from)
    }
}
