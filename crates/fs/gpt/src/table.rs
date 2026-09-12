// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The table on a device: reading the one that is there, walking its
//! entries, and writing a fresh one.

use fs_fat::BlockDevice;

use crate::SECTOR;
use crate::crc32::Crc32;
use crate::entry::{ENTRY_LEN, Entry, GUID_LEN};
use crate::error::Error;
use crate::header::{
    ARRAY_SECTORS, ENTRY_COUNT, FIRST_USABLE, HEADER_LBA, Header, MIN_SECTORS, last_usable,
};
use crate::mbr::{is_protective, protective};

/// Block of the protective record.
const PROTECTIVE_LBA: u64 = 0;

/// Block the array of a table this crate writes begins at.
const ARRAY_LBA: u64 = 2;

/// Reads the table of `device`: the primary one, or the backup where the
/// primary is torn.
///
/// Every check UEFI 2.11, section 5.3.2 asks for is made: the protective
/// record, the signature, the header checksum, that the header lies where
/// it says, and the checksum of the entry array. A primary that fails any
/// of them sends the read to the last block, which is where the backup
/// lies; the error of the primary is what comes back when the backup
/// fails too, because that is the table that was meant to be read.
///
/// Reading costs O(A) block reads, where A is the blocks of the entry
/// array, because the checksum covers all of them.
///
/// # Errors
///
/// [`Error::TooSmall`] for a device below [`MIN_SECTORS`] blocks,
/// [`Error::NotProtective`] for a device partitioned the legacy way, and
/// otherwise what [`Header::parse`] refused or [`Error::ArrayChecksum`].
pub fn read<D: BlockDevice>(device: &D) -> Result<Header, Error> {
    let sectors = u64::from(device.sectors());
    if sectors < MIN_SECTORS {
        return Err(Error::TooSmall(sectors));
    }
    let mut sector = [0u8; SECTOR];
    read_block(device, PROTECTIVE_LBA, &mut sector)?;
    if !is_protective(&sector) {
        return Err(Error::NotProtective);
    }
    let primary = read_header(device, HEADER_LBA, sectors, &mut sector);
    let Err(refused) = primary else {
        return primary;
    };
    read_header(device, sectors.saturating_sub(1), sectors, &mut sector).map_err(|_| refused)
}

/// The header in block `lba`, with the checksum of its array checked.
fn read_header<D: BlockDevice>(
    device: &D,
    lba: u64,
    sectors: u64,
    sector: &mut [u8; SECTOR],
) -> Result<Header, Error> {
    read_block(device, lba, sector)?;
    let header = Header::parse(sector, lba, sectors)?;
    check_array_checksum(device, &header)?;
    Ok(header)
}

/// That the checksum the header carries covers the bytes of the array.
///
/// The checksum covers the count times the size and no more, so what is
/// left over in the last block is read past rather than fed in (UEFI
/// 2.11, section 5.3.1).
fn check_array_checksum<D: BlockDevice>(device: &D, header: &Header) -> Result<(), Error> {
    let mut crc = Crc32::new();
    let mut left = header.array_len();
    let mut lba = header.entry_lba;
    let mut sector = [0u8; SECTOR];
    while left > 0 {
        read_block(device, lba, &mut sector)?;
        let take = usize::try_from(left).unwrap_or(SECTOR).min(SECTOR);
        crc.update(sector.get(..take).unwrap_or(&[]));
        left = left.saturating_sub(u64::try_from(take).unwrap_or(0));
        lba = lba.saturating_add(1);
    }
    if crc.finish() == header.array_crc {
        Ok(())
    } else {
        Err(Error::ArrayChecksum)
    }
}

/// Where a walk of the entry array stands, and the block it last read.
///
/// The block is kept because an entry is smaller than one: a walk that
/// read a block per entry would read every block of the array as many
/// times as it holds entries.
#[derive(Clone, Debug)]
pub struct Cursor {
    /// The next entry to look at.
    index: u32,
    /// The block [`Cursor::sector`] holds, if any.
    loaded: Option<u64>,
    /// The block last read.
    sector: [u8; SECTOR],
}

impl Default for Cursor {
    fn default() -> Self {
        Self::new()
    }
}

impl Cursor {
    /// A walk that has looked at nothing.
    #[must_use]
    pub const fn new() -> Cursor {
        Cursor {
            index: 0,
            loaded: None,
            sector: [0u8; SECTOR],
        }
    }
}

/// The next entry of the array that is in use, or `None` at its end.
///
/// The whole walk costs O(A) block reads, where A is the blocks of the
/// array, because the cursor keeps the block it is inside.
///
/// # Errors
///
/// [`Error::Device`] for a block the device would not move, and
/// [`Error::Partition`] for an entry that lies outside the range the
/// header calls usable, which a table that was written correctly has none
/// of (UEFI 2.11, section 5.3.3).
pub fn next_entry<D: BlockDevice>(
    device: &D,
    header: &Header,
    cursor: &mut Cursor,
) -> Result<Option<Entry>, Error> {
    let per_block = per_block(header)?;
    while cursor.index < header.entry_count {
        let index = cursor.index;
        cursor.index = cursor.index.saturating_add(1);
        let lba = header
            .entry_lba
            .saturating_add(u64::from(index.checked_div(per_block).unwrap_or(0)));
        if cursor.loaded != Some(lba) {
            read_block(device, lba, &mut cursor.sector)?;
            cursor.loaded = Some(lba);
        }
        let offset = usize::try_from(index.checked_rem(per_block).unwrap_or(0))
            .unwrap_or(0)
            .saturating_mul(usize::try_from(header.entry_len).unwrap_or(ENTRY_LEN));
        let bytes = cursor
            .sector
            .get(offset..offset.saturating_add(ENTRY_LEN))
            .unwrap_or(&[]);
        let entry = Entry::parse(bytes);
        if !entry.is_used() {
            continue;
        }
        if entry.first_lba < header.first_usable
            || entry.last_lba > header.last_usable
            || entry.last_lba < entry.first_lba
        {
            return Err(Error::Partition(entry.first_lba));
        }
        return Ok(Some(entry));
    }
    Ok(None)
}

/// The first partition of `type_guid`, or `None` where the table has
/// none.
///
/// # Errors
///
/// Those of [`next_entry`].
pub fn find<D: BlockDevice>(
    device: &D,
    header: &Header,
    type_guid: &[u8; GUID_LEN],
) -> Result<Option<Entry>, Error> {
    let mut cursor = Cursor::new();
    while let Some(entry) = next_entry(device, header, &mut cursor)? {
        if entry.type_guid == *type_guid {
            return Ok(Some(entry));
        }
    }
    Ok(None)
}

/// Writes a fresh table onto `device`: the protective record, both entry
/// arrays, and both headers.
///
/// The order is what a write cut short leaves behind. The protective
/// record goes down first, because a reader that does not find one takes
/// the device for one partitioned the legacy way and reads no table at
/// all; then the arrays; then the backup header, before the primary, so
/// that the copy a reader falls back to is whole before the copy it
/// reads first names an array nobody wrote (UEFI 2.11, section 5.3.2).
///
/// Checking that no two partitions overlap is O(N²) in the number of
/// entries, which is at most [`ENTRY_COUNT`].
///
/// # Errors
///
/// [`Error::TooSmall`] for a device below [`MIN_SECTORS`] blocks,
/// [`Error::Space`] for more entries than the array holds, and
/// [`Error::Partition`] for one that lies outside the usable range or
/// over another.
pub fn write<D: BlockDevice>(
    device: &mut D,
    disk_guid: &[u8; GUID_LEN],
    entries: &[Entry],
) -> Result<(), Error> {
    let sectors = u64::from(device.sectors());
    if sectors < MIN_SECTORS {
        return Err(Error::TooSmall(sectors));
    }
    let count = usize::try_from(ENTRY_COUNT).unwrap_or(0);
    if entries.len() > count {
        return Err(Error::Space(entries.len()));
    }
    let last = sectors.saturating_sub(1);
    let last_usable = last_usable(sectors);
    check_entries(entries, FIRST_USABLE, last_usable)?;

    write_block(device, PROTECTIVE_LBA, &protective(sectors))?;
    let backup_array = last.saturating_sub(ARRAY_SECTORS);
    // Both copies hold the same entries and so carry the same checksum.
    // The backup goes down whole before the primary begins, which is why
    // this is two passes and not one that writes each block twice.
    let array_crc = write_array(device, entries, backup_array)?;
    write_array(device, entries, ARRAY_LBA)?;

    let mut header = Header {
        my_lba: last,
        alternate_lba: HEADER_LBA,
        first_usable: FIRST_USABLE,
        last_usable,
        disk_guid: *disk_guid,
        entry_lba: backup_array,
        entry_count: ENTRY_COUNT,
        entry_len: u32::try_from(ENTRY_LEN).unwrap_or(0),
        array_crc,
    };
    write_block(device, last, &header.write())?;
    header.my_lba = HEADER_LBA;
    header.alternate_lba = last;
    header.entry_lba = ARRAY_LBA;
    write_block(device, HEADER_LBA, &header.write())
}

/// That every entry lies inside the usable range and over no other.
fn check_entries(entries: &[Entry], first_usable: u64, last_usable: u64) -> Result<(), Error> {
    for (index, entry) in entries.iter().enumerate() {
        if !entry.is_used() {
            continue;
        }
        if entry.first_lba < first_usable
            || entry.last_lba > last_usable
            || entry.last_lba < entry.first_lba
        {
            return Err(Error::Partition(entry.first_lba));
        }
        let rest = entries.get(index.saturating_add(1)..).unwrap_or(&[]);
        for other in rest.iter().filter(|other| other.is_used()) {
            if entry.first_lba <= other.last_lba && other.first_lba <= entry.last_lba {
                return Err(Error::Partition(other.first_lba));
            }
        }
    }
    Ok(())
}

/// Writes the entry array at `lba` and answers its checksum, which is the
/// same for both copies because both hold the same entries.
fn write_array<D: BlockDevice>(device: &mut D, entries: &[Entry], lba: u64) -> Result<u32, Error> {
    let per_block = SECTOR.checked_div(ENTRY_LEN).unwrap_or(0);
    let mut crc = Crc32::new();
    for block in 0..ARRAY_SECTORS {
        let mut sector = [0u8; SECTOR];
        let first = usize::try_from(block)
            .unwrap_or(0)
            .saturating_mul(per_block);
        for slot in 0..per_block {
            let Some(entry) = entries.get(first.saturating_add(slot)) else {
                break;
            };
            let offset = slot.saturating_mul(ENTRY_LEN);
            entry.write(sector.get_mut(offset..).unwrap_or(&mut []));
        }
        crc.update(&sector);
        write_block(device, lba.saturating_add(block), &sector)?;
    }
    Ok(crc.finish())
}

/// How many entries a block holds, which the header's entry size settles.
fn per_block(header: &Header) -> Result<u32, Error> {
    let block = u32::try_from(SECTOR).unwrap_or(0);
    block
        .checked_div(header.entry_len)
        .filter(|count| *count > 0)
        .ok_or(Error::EntrySize(header.entry_len))
}

/// Reads block `lba`, which a device that counts its blocks in thirty-two
/// bits may not have.
fn read_block<D: BlockDevice>(device: &D, lba: u64, into: &mut [u8; SECTOR]) -> Result<(), Error> {
    let block = u32::try_from(lba).map_err(|_| Error::Lba(lba))?;
    device.read(block, into).map_err(|_| Error::Device(block))
}

/// Writes block `lba`.
fn write_block<D: BlockDevice>(device: &mut D, lba: u64, from: &[u8; SECTOR]) -> Result<(), Error> {
    let block = u32::try_from(lba).map_err(|_| Error::Lba(lba))?;
    device.write(block, from).map_err(|_| Error::Device(block))
}
