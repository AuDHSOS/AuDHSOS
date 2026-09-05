// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The GUID partition table of the disk image: a protective master boot
//! record, the primary and backup headers, and the partition entry array.
//!
//! Invariants: the two headers name each other; both checksums cover what
//! they claim to cover; the partition lies inside the usable range the
//! headers report.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "the offsets are bounded by the sector and array sizes this module defines"
)]

use crate::error::Error;
use crate::image::crc32::crc32;

/// Size of one sector in bytes.
pub(crate) const SECTOR: usize = 512;

/// `log2(SECTOR)`, so that sector arithmetic needs no division.
pub(crate) const SECTOR_SHIFT: u32 = 9;

const _: () = assert!(SECTOR == 1 << SECTOR_SHIFT);

/// The number of whole sectors in `bytes`.
pub(crate) const fn sectors_of(bytes: u64) -> u64 {
    bytes >> SECTOR_SHIFT
}

/// Number of entries in the partition array.
pub(crate) const ENTRY_COUNT: u32 = 128;

/// Size of one partition entry in bytes.
pub(crate) const ENTRY_LEN: usize = 128;

/// Number of sectors the partition array occupies.
pub(crate) const ARRAY_SECTORS: u64 = 32;

/// Length of the header in bytes; the rest of its sector is zero.
pub(crate) const HEADER_LEN: usize = 92;

/// First sector of the partition.
pub(crate) const PARTITION_START: u64 = 2048;

/// First sector a partition may use.
pub(crate) const FIRST_USABLE: u64 = 34;

/// Signature of a partition table header.
pub(crate) const HEADER_SIGNATURE: [u8; 8] = *b"EFI PART";

/// Revision 1.0.
pub(crate) const HEADER_REVISION: u32 = 0x0001_0000;

/// `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`, the EFI system partition.
pub(crate) const ESP_TYPE_GUID: [u8; 16] = [
    0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B,
];

/// The identifier of the disk this project writes; fixed, so that two runs
/// produce the same image.
pub(crate) const DISK_GUID: [u8; 16] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0xA0, 0xD1, 0x00, 0x53, 0x4F, 0x53, 0x48, 0x44,
];

/// The identifier of the one partition; fixed for the same reason.
pub(crate) const PARTITION_GUID: [u8; 16] = [
    0x21, 0x43, 0x65, 0x87, 0x09, 0xBA, 0xDC, 0xFE, 0xA1, 0xD2, 0x00, 0x53, 0x4F, 0x53, 0x48, 0x44,
];

/// Name of the partition, as UTF-16 code units.
pub(crate) const PARTITION_NAME: &str = "AUDHSOS ESP";

/// Smallest disk the structures fit on: the protective record, both
/// headers, both arrays, and at least one sector of partition.
pub(crate) const MIN_SECTORS: u64 = PARTITION_START + FIRST_USABLE + 1;

/// Writes the protective record, both headers, and both arrays into
/// `image`, which is `sectors` sectors long, and reports the sector range
/// the partition occupies.
///
/// # Errors
///
/// [`Error::Usage`] if the disk is too small for the structures.
pub(crate) fn write(image: &mut [u8], sectors: u64) -> Result<(u64, u64), Error> {
    if sectors < MIN_SECTORS
        || image.len() < usize::try_from(sectors).unwrap_or(usize::MAX) * SECTOR
    {
        return Err(Error::Usage(format!(
            "a disk of {sectors} sectors is too small for a partition table"
        )));
    }
    let last = sectors.saturating_sub(1);
    let (_first, last_usable) = partition_range(sectors);
    let backup_array = last.saturating_sub(ARRAY_SECTORS);

    write_protective_mbr(image, sectors);
    let array = partition_array(PARTITION_START, last_usable);
    let array_crc = crc32(&array);
    put(image, sector_offset(2), &array);
    put(image, sector_offset(backup_array), &array);

    let primary = header(1, last, 2, last_usable, array_crc);
    put(image, sector_offset(1), &primary);
    let backup = header(last, 1, backup_array, last_usable, array_crc);
    put(image, sector_offset(last), &backup);
    Ok((PARTITION_START, last_usable))
}

/// The sectors the partition occupies on a disk of `sectors` sectors: from
/// [`PARTITION_START`] to the last sector before the backup array.
pub(crate) const fn partition_range(sectors: u64) -> (u64, u64) {
    let last = sectors.saturating_sub(1);
    let backup_array = last.saturating_sub(ARRAY_SECTORS);
    (PARTITION_START, backup_array.saturating_sub(1))
}

/// The byte offset of a sector.
pub(crate) fn sector_offset(sector: u64) -> usize {
    usize::try_from(sector)
        .unwrap_or(usize::MAX)
        .saturating_mul(SECTOR)
}

fn put(image: &mut [u8], offset: usize, bytes: &[u8]) {
    if let Some(slot) = offset
        .checked_add(bytes.len())
        .and_then(|end| image.get_mut(offset..end))
    {
        slot.copy_from_slice(bytes);
    }
}

/// The record that keeps a tool without GUID partition table support from
/// believing the disk is empty.
fn write_protective_mbr(image: &mut [u8], sectors: u64) {
    let size = u32::try_from(sectors.saturating_sub(1)).unwrap_or(u32::MAX);
    let mut entry = [0u8; 16];
    put(&mut entry, 1, &[0x00, 0x02, 0x00]);
    put(&mut entry, 4, &[0xEE]);
    put(&mut entry, 5, &[0xFF, 0xFF, 0xFF]);
    put(&mut entry, 8, &1u32.to_le_bytes());
    put(&mut entry, 12, &size.to_le_bytes());
    put(image, 446, &entry);
    put(image, 510, &[0x55, 0xAA]);
}

/// One partition table header, with its own checksum filled in.
fn header(
    current: u64,
    backup: u64,
    array_start: u64,
    last_usable: u64,
    array_crc: u32,
) -> [u8; SECTOR] {
    let mut sector = [0u8; SECTOR];
    put(&mut sector, 0, &HEADER_SIGNATURE);
    put(&mut sector, 8, &HEADER_REVISION.to_le_bytes());
    put(
        &mut sector,
        12,
        &u32::try_from(HEADER_LEN).unwrap_or(0).to_le_bytes(),
    );
    put(&mut sector, 24, &current.to_le_bytes());
    put(&mut sector, 32, &backup.to_le_bytes());
    put(&mut sector, 40, &FIRST_USABLE.to_le_bytes());
    put(&mut sector, 48, &last_usable.to_le_bytes());
    put(&mut sector, 56, &DISK_GUID);
    put(&mut sector, 72, &array_start.to_le_bytes());
    put(&mut sector, 80, &ENTRY_COUNT.to_le_bytes());
    put(
        &mut sector,
        84,
        &u32::try_from(ENTRY_LEN).unwrap_or(0).to_le_bytes(),
    );
    put(&mut sector, 88, &array_crc.to_le_bytes());
    let checksum = crc32(sector.get(..HEADER_LEN).unwrap_or(&[]));
    put(&mut sector, 16, &checksum.to_le_bytes());
    sector
}

/// The partition entry array: one entry in use, the rest zero.
fn partition_array(first: u64, last: u64) -> Vec<u8> {
    let mut array = vec![0u8; ENTRY_LEN * usize::try_from(ENTRY_COUNT).unwrap_or(0)];
    put(&mut array, 0, &ESP_TYPE_GUID);
    put(&mut array, 16, &PARTITION_GUID);
    put(&mut array, 32, &first.to_le_bytes());
    put(&mut array, 40, &last.to_le_bytes());
    for (index, unit) in PARTITION_NAME.encode_utf16().enumerate() {
        put(&mut array, 56 + index * 2, &unit.to_le_bytes());
    }
    array
}
