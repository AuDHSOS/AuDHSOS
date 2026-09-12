// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The table header: where everything else is, and the two checksums that
//! say it was not torn. UEFI 2.11, table 5.5.

use crate::SECTOR;
use crate::bytes::{put, read_guid, read_u32, read_u64};
use crate::crc32::crc32;
use crate::entry::{ENTRY_LEN, GUID_LEN};
use crate::error::Error;

/// Block of the primary header. The backup lies in the last block of the
/// device.
pub const HEADER_LBA: u64 = 1;

/// What a header begins with.
pub const HEADER_SIGNATURE: [u8; 8] = *b"EFI PART";

/// Revision 1.0, the only one this crate reads.
pub const HEADER_REVISION: u32 = 0x0001_0000;

/// Bytes of the header the fields need, and the size this crate writes.
/// A header may claim more; the rest of its block is reserved.
pub const HEADER_LEN: usize = 92;

/// Entries in the array this crate writes. The format asks for at least
/// 16384 bytes of array, which is what these make.
pub const ENTRY_COUNT: u32 = 128;

/// Blocks the array this crate writes occupies.
pub const ARRAY_SECTORS: u64 = 32;

/// First block a partition may use on a device this crate writes: the
/// protective record, the header, and the array lie below it.
pub const FIRST_USABLE: u64 = 34;

/// The smallest device a table fits on: a protective record, both
/// headers, both arrays, and one block of partition between them.
pub const MIN_SECTORS: u64 = 68;

/// The last block a partition may use on a device of `sectors` blocks:
/// the backup array and the backup header lie above it.
#[must_use]
pub const fn last_usable(sectors: u64) -> u64 {
    sectors
        .saturating_sub(1)
        .saturating_sub(ARRAY_SECTORS)
        .saturating_sub(1)
}

/// The header as numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// The block this header lies in.
    pub my_lba: u64,
    /// The block the other header lies in.
    pub alternate_lba: u64,
    /// First block a partition may use.
    pub first_usable: u64,
    /// Last block a partition may use.
    pub last_usable: u64,
    /// What this disk is.
    pub disk_guid: [u8; GUID_LEN],
    /// First block of the entry array this header describes.
    pub entry_lba: u64,
    /// Entries in that array.
    pub entry_count: u32,
    /// Bytes in one of them.
    pub entry_len: u32,
    /// The checksum the header carries for the array.
    pub array_crc: u32,
}

impl Header {
    /// The header in `sector`, which was read from block `lba` of a
    /// device of `sectors` blocks.
    ///
    /// Every check UEFI 2.11, section 5.3.2 asks for that can be made
    /// from the header alone is made here: the signature, the revision,
    /// the size, the header's own checksum, and that the header lies
    /// where it says it does. The array checksum needs the array and is
    /// [`crate::table::read`]'s.
    ///
    /// # Errors
    ///
    /// One of [`Error::Signature`], [`Error::Revision`],
    /// [`Error::HeaderSize`], [`Error::HeaderChecksum`],
    /// [`Error::MyLba`], [`Error::EntrySize`], [`Error::Usable`] or
    /// [`Error::ArrayRange`].
    pub fn parse(sector: &[u8; SECTOR], lba: u64, sectors: u64) -> Result<Header, Error> {
        if sector.get(..8) != Some(&HEADER_SIGNATURE) {
            return Err(Error::Signature);
        }
        let revision = read_u32(sector, 8);
        if revision != HEADER_REVISION {
            return Err(Error::Revision(revision));
        }
        let header_len = read_u32(sector, 12);
        let len = usize::try_from(header_len).unwrap_or(usize::MAX);
        if !(HEADER_LEN..=SECTOR).contains(&len) {
            return Err(Error::HeaderSize(header_len));
        }
        if checksum_of(sector, len) != read_u32(sector, 16) {
            return Err(Error::HeaderChecksum);
        }
        let my_lba = read_u64(sector, 24);
        if my_lba != lba {
            return Err(Error::MyLba(my_lba));
        }
        let entry_len = read_u32(sector, 84);
        if !entry_size_is_read(entry_len) {
            return Err(Error::EntrySize(entry_len));
        }
        let header = Header {
            my_lba,
            alternate_lba: read_u64(sector, 32),
            first_usable: read_u64(sector, 40),
            last_usable: read_u64(sector, 48),
            disk_guid: read_guid(sector, 56),
            entry_lba: read_u64(sector, 72),
            entry_count: read_u32(sector, 80),
            entry_len,
            array_crc: read_u32(sector, 88),
        };
        if header.last_usable < header.first_usable {
            return Err(Error::Usable(header.first_usable, header.last_usable));
        }
        header.check_array_range(sectors)?;
        Ok(header)
    }

    /// The header as the block it is written to.
    ///
    /// The size written is [`HEADER_LEN`] whatever the header was parsed
    /// from, and the rest of the block is zero: a header read with a
    /// larger size and written back loses the bytes above the fields,
    /// which the format reserves and asks to be zero anyway.
    #[must_use]
    pub fn write(&self) -> [u8; SECTOR] {
        let mut sector = [0u8; SECTOR];
        put(&mut sector, 0, &HEADER_SIGNATURE);
        put(&mut sector, 8, &HEADER_REVISION.to_le_bytes());
        put(
            &mut sector,
            12,
            &u32::try_from(HEADER_LEN).unwrap_or(0).to_le_bytes(),
        );
        put(&mut sector, 24, &self.my_lba.to_le_bytes());
        put(&mut sector, 32, &self.alternate_lba.to_le_bytes());
        put(&mut sector, 40, &self.first_usable.to_le_bytes());
        put(&mut sector, 48, &self.last_usable.to_le_bytes());
        put(&mut sector, 56, &self.disk_guid);
        put(&mut sector, 72, &self.entry_lba.to_le_bytes());
        put(&mut sector, 80, &self.entry_count.to_le_bytes());
        put(&mut sector, 84, &self.entry_len.to_le_bytes());
        put(&mut sector, 88, &self.array_crc.to_le_bytes());
        let checksum = checksum_of(&sector, HEADER_LEN);
        put(&mut sector, 16, &checksum.to_le_bytes());
        sector
    }

    /// Bytes the entry array covers: the count times the size, which is
    /// what the array checksum is taken over.
    #[must_use]
    pub fn array_len(&self) -> u64 {
        u64::from(self.entry_count).saturating_mul(u64::from(self.entry_len))
    }

    /// Blocks the entry array occupies, the last one rounded up.
    #[must_use]
    pub fn array_blocks(&self) -> u64 {
        let block = u64::try_from(SECTOR).unwrap_or(0);
        self.array_len()
            .saturating_add(block)
            .saturating_sub(1)
            .checked_div(block)
            .unwrap_or(0)
    }

    /// That the array lies on a device of `sectors` blocks and clear of
    /// the range it describes as usable. What the array holds is
    /// [`crate::table`]'s to check.
    ///
    /// # Errors
    ///
    /// [`Error::ArrayRange`].
    fn check_array_range(&self, sectors: u64) -> Result<(), Error> {
        let blocks = self.array_blocks();
        let end = self
            .entry_lba
            .checked_add(blocks)
            .ok_or(Error::ArrayRange)?;
        if blocks == 0 || end > sectors {
            return Err(Error::ArrayRange);
        }
        let below = end <= self.first_usable;
        let above = self.entry_lba > self.last_usable;
        if !below && !above {
            return Err(Error::ArrayRange);
        }
        Ok(())
    }
}

/// Whether an entry of `len` bytes is one this crate reads.
///
/// The format asks for 128 times a power of two (UEFI 2.11, table 5.5).
/// This crate asks for one thing more, that the size divide the block, so
/// that no entry straddles two of them and the walk of the array needs
/// one block at a time and no seam.
fn entry_size_is_read(len: u32) -> bool {
    let smallest = u32::try_from(ENTRY_LEN).unwrap_or(0);
    let block = u32::try_from(SECTOR).unwrap_or(0);
    len >= smallest
        && len.is_multiple_of(smallest)
        && len.checked_div(smallest).unwrap_or(0).is_power_of_two()
        && block.is_multiple_of(len)
}

/// The checksum over the first `len` bytes of `sector` with the field
/// that holds it read as zero, which is how UEFI 2.11, table 5.5 defines
/// it.
fn checksum_of(sector: &[u8; SECTOR], len: usize) -> u32 {
    let mut copy = *sector;
    put(&mut copy, 16, &0u32.to_le_bytes());
    crc32(copy.get(..len).unwrap_or(&[]))
}
