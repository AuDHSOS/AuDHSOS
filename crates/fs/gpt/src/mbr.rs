// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The protective record in the first block: what keeps a tool that never
//! heard of a partition table GUID from believing the device is empty.
//! UEFI 2.11, tables 5.3 and 5.4.

use crate::SECTOR;
use crate::bytes::put;

/// The type of the one record, which claims the whole device.
pub const PROTECTIVE_TYPE: u8 = 0xEE;

/// Offset of the first of the four records.
const RECORDS: usize = 446;

/// Bytes in one record.
const RECORD_LEN: usize = 16;

/// Records in the table.
const RECORD_COUNT: usize = 4;

/// Offset of the trailing signature.
const SIGNATURE: usize = 510;

/// The first block of a device of `sectors` blocks.
///
/// The record covers everything above itself. A device of more blocks
/// than a thirty-two-bit count holds gets the largest count there is,
/// which is what the format asks for rather than a truncation.
#[must_use]
pub fn protective(sectors: u64) -> [u8; SECTOR] {
    let mut sector = [0u8; SECTOR];
    let size = u32::try_from(sectors.saturating_sub(1)).unwrap_or(u32::MAX);
    let mut record = [0u8; RECORD_LEN];
    put(&mut record, 1, &[0x00, 0x02, 0x00]);
    put(&mut record, 4, &[PROTECTIVE_TYPE]);
    put(&mut record, 5, &[0xFF, 0xFF, 0xFF]);
    put(&mut record, 8, &1u32.to_le_bytes());
    put(&mut record, 12, &size.to_le_bytes());
    put(&mut sector, RECORDS, &record);
    put(&mut sector, SIGNATURE, &[0x55, 0xAA]);
    sector
}

/// Whether the block is a protective record: it ends in `0x55 0xAA` and
/// one of its four records is of type [`PROTECTIVE_TYPE`].
///
/// A device whose first block is a legacy table instead describes its
/// partitions there, and a table found behind such a record is one an
/// older tool left standing (UEFI 2.11, section 5.3.2).
#[must_use]
pub fn is_protective(sector: &[u8; SECTOR]) -> bool {
    if sector.get(SIGNATURE..SECTOR) != Some(&[0x55, 0xAA]) {
        return false;
    }
    (0..RECORD_COUNT).any(|index| {
        let offset = RECORDS
            .saturating_add(index.saturating_mul(RECORD_LEN))
            .saturating_add(4);
        sector.get(offset) == Some(&PROTECTIVE_TYPE)
    })
}
