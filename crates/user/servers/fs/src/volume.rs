// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Which volume a disk carries, and what may be done to it.
//!
//! Two cases, and the difference between them is the whole of it.
//!
//! A disk that carries a partition table holds a volume somebody else
//! made: the firmware wrote the table and reads the partition. Such a
//! disk is mounted and never formatted — the boot disk of this machine is
//! one, and a format there is the loader and the kernel gone.
//!
//! A disk that carries no table is one nothing has written. It is
//! formatted, which is what the scratch disk of the reference machine
//! needs on its first boot (D-136).
//!
//! Invariant: [`mount`] writes a file system onto a disk only where it
//! found neither a partition table nor a volume.

use audhsos_abi::Error;
use fs_fat::{BlockDevice, FileSystem, FormatOptions};
use fs_gpt::ESP_TYPE_GUID;

use crate::error::{refusal, table_refusal};
use crate::partition::Partition;

/// Whether `device` carries a partition table.
///
/// A disk that does is one somebody else partitioned — on this machine
/// the disk the firmware booted from — and it is the volume the system
/// reads and never writes. A disk that does not is the system's own.
#[must_use]
pub fn is_partitioned<D: BlockDevice>(device: &D) -> bool {
    !matches!(
        fs_gpt::read(device),
        Err(fs_gpt::Error::NotProtective | fs_gpt::Error::Signature)
    )
}

/// The volume `device` carries, formatted where it carried none.
///
/// The answer is a file system over a [`Partition`] either way: over the
/// partition the table names, or over the whole disk where there is no
/// table, so that one type covers both.
///
/// # Errors
///
/// [`Error::NotFound`] for a disk whose table names no partition this
/// system reads; [`Error::InvalidArgument`] for one whose partition does
/// not lie on it; and the refusals of `fs-fat` for a volume that is none.
pub fn mount<D: BlockDevice>(device: D) -> Result<FileSystem<Partition<D>>, Error> {
    match fs_gpt::read(&device) {
        Ok(header) => {
            let entry = fs_gpt::find(&device, &header, &ESP_TYPE_GUID)
                .map_err(table_refusal)?
                .ok_or(Error::NotFound)?;
            let window = Partition::new(device, entry.first_lba, entry.last_lba)
                .ok_or(Error::InvalidArgument)?;
            FileSystem::mount(window).map_err(refusal)
        }
        // A disk nothing has partitioned, whose whole is the volume. It
        // is the two refusals that say the table is not there and not
        // that it is wrong: no protective record in the first block, and
        // no `EFI PART` where the header belongs. Every other refusal is
        // a table this system cannot read, and a disk whose partitions it
        // cannot read is one it must not format.
        Err(fs_gpt::Error::NotProtective | fs_gpt::Error::Signature) => whole(device),
        Err(error) => Err(table_refusal(error)),
    }
}

/// The volume over the whole disk, formatted where there is none.
fn whole<D: BlockDevice>(device: D) -> Result<FileSystem<Partition<D>>, Error> {
    let last = u64::from(device.sectors().saturating_sub(1));
    let window = Partition::new(device, 0, last).ok_or(Error::InvalidArgument)?;
    match fs_fat::info(&window) {
        Ok(Some(_)) => FileSystem::mount(window).map_err(refusal),
        Ok(None) => FileSystem::format(window, &FormatOptions::default()).map_err(refusal),
        Err(error) => Err(refusal(error)),
    }
}
