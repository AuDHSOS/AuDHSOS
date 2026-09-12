// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::header`.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use crate::SECTOR;
use crate::crc32::crc32;
use crate::entry::ENTRY_LEN;
use crate::error::Error;
use crate::header::{
    ARRAY_SECTORS, ENTRY_COUNT, FIRST_USABLE, HEADER_LBA, HEADER_LEN, HEADER_REVISION,
    HEADER_SIGNATURE, Header, MIN_SECTORS, last_usable,
};

use super::support::{DISK_GUID, SECTORS};

/// The blocks of the device every header here is read against.
const BLOCKS: u64 = SECTORS as u64;

fn header() -> Header {
    Header {
        my_lba: HEADER_LBA,
        alternate_lba: BLOCKS - 1,
        first_usable: FIRST_USABLE,
        last_usable: last_usable(BLOCKS),
        disk_guid: DISK_GUID,
        entry_lba: 2,
        entry_count: ENTRY_COUNT,
        entry_len: ENTRY_LEN as u32,
        array_crc: 0x1234_5678,
    }
}

/// The header's block with `bytes` put at `offset`, and the header
/// checksum recomputed so that only the change under test is wrong.
fn patched(offset: usize, bytes: &[u8]) -> [u8; SECTOR] {
    let mut sector = header().write();
    sector[offset..offset + bytes.len()].copy_from_slice(bytes);
    sector[16..20].copy_from_slice(&[0, 0, 0, 0]);
    let len = u32::from_le_bytes(sector[12..16].try_into().unwrap()) as usize;
    let checksum = crc32(&sector[..len.min(SECTOR)]);
    sector[16..20].copy_from_slice(&checksum.to_le_bytes());
    sector
}

/// The block with its header checksum recomputed, for a test that
/// changed more than one field.
fn patched_from(mut sector: [u8; SECTOR]) -> [u8; SECTOR] {
    sector[16..20].copy_from_slice(&[0, 0, 0, 0]);
    let len = u32::from_le_bytes(sector[12..16].try_into().unwrap()) as usize;
    let checksum = crc32(&sector[..len.min(SECTOR)]);
    sector[16..20].copy_from_slice(&checksum.to_le_bytes());
    sector
}

fn parse(sector: &[u8; SECTOR]) -> Result<Header, Error> {
    Header::parse(sector, HEADER_LBA, BLOCKS)
}

#[test]
fn a_header_written_and_read_back_is_the_header() {
    let written = header().write();
    assert_eq!(parse(&written), Ok(header()));
    assert_eq!(&written[..8], &HEADER_SIGNATURE);
    assert_eq!(
        u32::from_le_bytes(written[8..12].try_into().unwrap()),
        HEADER_REVISION
    );
    assert_eq!(
        u32::from_le_bytes(written[12..16].try_into().unwrap()) as usize,
        HEADER_LEN
    );
    assert!(
        written[HEADER_LEN..].iter().all(|byte| *byte == 0),
        "the rest of the block is reserved"
    );
}

#[test]
fn the_checksum_covers_the_header_with_its_own_field_read_as_zero() {
    let written = header().write();
    let mut zeroed = written;
    zeroed[16..20].copy_from_slice(&[0, 0, 0, 0]);
    assert_eq!(
        u32::from_le_bytes(written[16..20].try_into().unwrap()),
        crc32(&zeroed[..HEADER_LEN])
    );
}

#[test]
fn a_header_whose_checksum_does_not_cover_its_bytes_is_refused() {
    let mut sector = header().write();
    sector[56] ^= 0xFF;
    assert_eq!(parse(&sector), Err(Error::HeaderChecksum));
}

#[test]
fn a_signature_or_a_revision_the_format_does_not_have_is_refused() {
    assert_eq!(parse(&[0u8; SECTOR]), Err(Error::Signature));
    assert_eq!(
        parse(&patched(8, &0x0002_0000u32.to_le_bytes())),
        Err(Error::Revision(0x0002_0000))
    );
}

#[test]
fn a_header_below_the_fields_or_above_its_block_is_refused() {
    assert_eq!(
        parse(&patched(12, &91u32.to_le_bytes())),
        Err(Error::HeaderSize(91))
    );
    assert_eq!(
        parse(&patched(12, &513u32.to_le_bytes())),
        Err(Error::HeaderSize(513))
    );
    assert!(parse(&patched(12, &92u32.to_le_bytes())).is_ok());
    assert!(parse(&patched(12, &512u32.to_le_bytes())).is_ok());
}

#[test]
fn a_header_that_does_not_lie_where_it_says_is_refused() {
    let sector = header().write();
    assert_eq!(Header::parse(&sector, 2, BLOCKS), Err(Error::MyLba(1)));
}

#[test]
fn an_entry_size_that_would_straddle_a_block_is_refused() {
    for len in [0u32, 64, 96, 192, 1024] {
        assert_eq!(
            parse(&patched(84, &len.to_le_bytes())),
            Err(Error::EntrySize(len)),
            "{len}"
        );
    }
    // A larger entry makes a larger array, so the count comes down with
    // the size to keep the array where it fits.
    for (len, count) in [(128u32, 128u32), (256, 64), (512, 32)] {
        let mut sector = patched(84, &len.to_le_bytes());
        sector[80..84].copy_from_slice(&count.to_le_bytes());
        let sector = patched_from(sector);
        let header = parse(&sector).unwrap_or_else(|error| panic!("{len}: {error}"));
        assert_eq!(header.array_len(), 16384);
    }
}

#[test]
fn a_usable_range_that_is_empty_is_refused() {
    let sector = patched(48, &(FIRST_USABLE - 1).to_le_bytes());
    assert_eq!(
        parse(&sector),
        Err(Error::Usable(FIRST_USABLE, FIRST_USABLE - 1))
    );
}

#[test]
fn an_array_that_does_not_lie_on_the_device_is_refused() {
    // Past the end of the device.
    assert_eq!(
        parse(&patched(72, &(BLOCKS - 1).to_le_bytes())),
        Err(Error::ArrayRange)
    );
    // Inside the range it describes as usable.
    assert_eq!(
        parse(&patched(72, &FIRST_USABLE.to_le_bytes())),
        Err(Error::ArrayRange)
    );
    // An array of nothing.
    assert_eq!(
        parse(&patched(80, &0u32.to_le_bytes())),
        Err(Error::ArrayRange)
    );
    // The backup array, which lies above the usable range.
    let above = BLOCKS - 1 - ARRAY_SECTORS;
    assert!(parse(&patched(72, &above.to_le_bytes())).is_ok());
}

#[test]
fn an_array_whose_length_overflows_is_refused() {
    let sector = patched(72, &u64::MAX.to_le_bytes());
    assert_eq!(parse(&sector), Err(Error::ArrayRange));
}

#[test]
fn the_array_of_the_header_this_crate_writes_is_the_size_the_format_asks_for() {
    let header = header();
    assert_eq!(header.array_len(), 16384, "the format asks for 16384 bytes");
    assert_eq!(header.array_blocks(), ARRAY_SECTORS);
}

#[test]
fn an_array_that_does_not_fill_its_last_block_takes_one_more() {
    let header = Header {
        entry_count: 3,
        ..header()
    };
    assert_eq!(header.array_len(), 384);
    assert_eq!(header.array_blocks(), 1);
    let header = Header {
        entry_count: 5,
        ..header
    };
    assert_eq!(header.array_len(), 640);
    assert_eq!(header.array_blocks(), 2);
}

#[test]
fn the_usable_range_leaves_room_for_the_backup_at_the_end() {
    assert_eq!(last_usable(BLOCKS), BLOCKS - ARRAY_SECTORS - 2);
    assert_eq!(last_usable(MIN_SECTORS), FIRST_USABLE);
    assert_eq!(last_usable(0), 0);
}
