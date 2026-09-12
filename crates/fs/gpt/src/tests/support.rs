// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the tests of this crate build their devices out of.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use fs_fat::doubles::RamDisk;

use crate::entry::{ESP_TYPE_GUID, Entry, GUID_LEN};
use crate::error::Error;
use crate::header::Header;
use crate::table::write;

/// Blocks of the devices the tests write on: room for the table at both
/// ends and a partition between them.
pub(crate) const SECTORS: u32 = 2048;

/// First block of the partition every test writes, which is where the
/// image writer puts one.
pub(crate) const PARTITION_START: u64 = 1024;

/// The disk identifier of a test device.
pub(crate) const DISK_GUID: [u8; GUID_LEN] = [0xD1; GUID_LEN];

/// The partition identifier of a test device.
pub(crate) const PARTITION_GUID: [u8; GUID_LEN] = [0xB1; GUID_LEN];

/// A device of `SECTORS` blocks carrying one EFI system partition.
pub(crate) fn disk() -> RamDisk {
    let mut device = RamDisk::new(SECTORS);
    write(&mut device, &DISK_GUID, &[esp()]).expect("a table");
    device
}

/// The one entry a test device carries.
pub(crate) fn esp() -> Entry {
    Entry::new(
        ESP_TYPE_GUID,
        PARTITION_GUID,
        PARTITION_START,
        crate::header::last_usable(u64::from(SECTORS)),
        "AUDHSOS ESP",
    )
    .expect("an entry")
}

/// Puts `header` back into the block it names, with the checksum it needs
/// after whatever the caller changed.
pub(crate) fn put_header(device: &mut RamDisk, header: &Header) {
    let offset = usize::try_from(header.my_lba).unwrap_or(0) * crate::SECTOR;
    let sector = header.write();
    let bytes = device.bytes_mut();
    bytes[offset..offset + crate::SECTOR].copy_from_slice(&sector);
}

/// The error a call refused with.
pub(crate) fn refusal<T: core::fmt::Debug>(result: Result<T, Error>) -> Error {
    match result {
        Err(error) => error,
        Ok(value) => panic!("the call answered {value:?} rather than refusing"),
    }
}
