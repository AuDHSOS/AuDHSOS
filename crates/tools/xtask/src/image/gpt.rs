// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The GUID partition table of the disk image.
//!
//! The structures are `fs-gpt`'s; what is here is the image's own: where
//! the one partition begins, what it is called, and the identifiers that
//! are fixed so that two runs produce the same bytes.

use fs_gpt::Entry;

use crate::error::Error;
use crate::image::device::Slice;

pub(crate) use fs_gpt::{ESP_TYPE_GUID, FIRST_USABLE, SECTOR, last_usable};

/// `log2(SECTOR)`, so that sector arithmetic needs no division.
pub(crate) const SECTOR_SHIFT: u32 = 9;

const _: () = assert!(SECTOR == 1 << SECTOR_SHIFT);

/// The number of whole sectors in `bytes`.
pub(crate) const fn sectors_of(bytes: u64) -> u64 {
    bytes >> SECTOR_SHIFT
}

/// First sector of the partition: a mebibyte in, which is the alignment
/// UEFI 2.11, section 5.3.1 suggests for a partition whose device may
/// have physical blocks larger than its logical ones.
pub(crate) const PARTITION_START: u64 = 2048;

/// The identifier of the disk this project writes; fixed, so that two
/// runs produce the same image.
pub(crate) const DISK_GUID: [u8; 16] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0xA0, 0xD1, 0x00, 0x53, 0x4F, 0x53, 0x48, 0x44,
];

/// The identifier of the one partition; fixed for the same reason.
pub(crate) const PARTITION_GUID: [u8; 16] = [
    0x21, 0x43, 0x65, 0x87, 0x09, 0xBA, 0xDC, 0xFE, 0xA1, 0xD2, 0x00, 0x53, 0x4F, 0x53, 0x48, 0x44,
];

/// Name of the partition, as UTF-16 code units.
pub(crate) const PARTITION_NAME: &str = "AUDHSOS ESP";

/// Smallest disk this image fits on: everything the table needs, and the
/// mebibyte the partition is aligned to above it.
pub(crate) const MIN_SECTORS: u64 = PARTITION_START + FIRST_USABLE + 1;

/// Writes the protective record, both headers, and both arrays into
/// `image`, which is `sectors` sectors long, and reports the sector range
/// the partition occupies.
///
/// # Errors
///
/// [`Error::Usage`] if the disk is too small for the structures.
pub(crate) fn write(image: &mut [u8], sectors: u64) -> Result<(u64, u64), Error> {
    let (first, last) = partition_range(sectors);
    let too_small = || {
        Error::Usage(format!(
            "a disk of {sectors} sectors is too small for a partition table"
        ))
    };
    if sectors < MIN_SECTORS {
        return Err(too_small());
    }
    // The disk is what the image holds of it, so a buffer shorter than
    // the sector count names is the same refusal.
    let bytes = image
        .get_mut(..sector_offset(sectors))
        .ok_or_else(too_small)?;
    let entry = Entry::new(ESP_TYPE_GUID, PARTITION_GUID, first, last, PARTITION_NAME)
        .map_err(|error| Error::Usage(format!("the partition entry: {error}")))?;
    let mut device = Slice { bytes };
    fs_gpt::write(&mut device, &DISK_GUID, &[entry])
        .map_err(|error| Error::Usage(format!("the partition table: {error}")))?;
    Ok((first, last))
}

/// The sectors the partition occupies on a disk of `sectors` sectors:
/// from [`PARTITION_START`] to the last sector before the backup array.
pub(crate) const fn partition_range(sectors: u64) -> (u64, u64) {
    (PARTITION_START, last_usable(sectors))
}

/// The byte offset of a sector.
pub(crate) fn sector_offset(sector: u64) -> usize {
    usize::try_from(sector)
        .unwrap_or(usize::MAX)
        .saturating_mul(SECTOR)
}
