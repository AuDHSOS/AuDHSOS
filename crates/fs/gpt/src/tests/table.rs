// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::table`.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::indexing_slicing
)]

use fs_fat::doubles::RamDisk;

use crate::SECTOR;
use crate::entry::{ENTRY_LEN, ESP_TYPE_GUID, Entry, GUID_LEN};
use crate::error::Error;
use crate::header::{ARRAY_SECTORS, ENTRY_COUNT, FIRST_USABLE, MIN_SECTORS, last_usable};
use crate::table::{Cursor, find, next_entry, read, write};

use super::support::{
    DISK_GUID, PARTITION_GUID, PARTITION_START, SECTORS, disk, esp, put_header, refusal,
};

/// The blocks of the devices these tests write on.
const BLOCKS: u64 = SECTORS as u64;

/// A type nothing on a test device carries.
const OTHER_TYPE: [u8; GUID_LEN] = [0x0A; GUID_LEN];

#[test]
fn a_table_written_and_read_back_is_the_table() {
    let device = disk();
    let header = read(&device).expect("a table");
    assert_eq!(header.my_lba, 1);
    assert_eq!(header.alternate_lba, BLOCKS - 1);
    assert_eq!(header.first_usable, FIRST_USABLE);
    assert_eq!(header.last_usable, last_usable(BLOCKS));
    assert_eq!(header.disk_guid, DISK_GUID);
    assert_eq!(header.entry_lba, 2);
    assert_eq!(header.entry_count, ENTRY_COUNT);
    assert_eq!(header.entry_len as usize, ENTRY_LEN);
    let found = find(&device, &header, &ESP_TYPE_GUID).expect("a walk");
    assert_eq!(found, Some(esp()));
    assert_eq!(found.map(|entry| entry.unique_guid), Some(PARTITION_GUID));
}

#[test]
fn the_backup_names_the_primary_and_lies_in_the_last_block() {
    let mut device = disk();
    let sectors = device.bytes().len() / SECTOR;
    assert_eq!(sectors, SECTORS as usize);
    // Tear the primary header, which sends the read to the backup.
    device.bytes_mut()[SECTOR..SECTOR + 8].copy_from_slice(b"XXXXXXXX");
    let header = read(&device).expect("the backup");
    assert_eq!(header.my_lba, BLOCKS - 1);
    assert_eq!(header.alternate_lba, 1);
    assert_eq!(header.entry_lba, BLOCKS - 1 - ARRAY_SECTORS);
    assert_eq!(
        find(&device, &header, &ESP_TYPE_GUID).expect("a walk"),
        Some(esp())
    );
}

#[test]
fn both_copies_torn_reports_what_the_primary_refused() {
    let mut device = disk();
    let last = device.bytes().len() - SECTOR;
    device.bytes_mut()[SECTOR + 8..SECTOR + 12].copy_from_slice(&0x0002_0000u32.to_le_bytes());
    device.bytes_mut()[last..last + 8].copy_from_slice(b"XXXXXXXX");
    assert_eq!(refusal(read(&device)), Error::Revision(0x0002_0000));
}

#[test]
fn a_torn_entry_array_is_refused_by_its_checksum() {
    let mut device = disk();
    let array = 2 * SECTOR;
    device.bytes_mut()[array] ^= 0xFF;
    // The backup array is still whole, so the read falls back to it.
    assert_eq!(read(&device).expect("the backup").my_lba, BLOCKS - 1);
    let backup = usize::try_from(BLOCKS - 1 - ARRAY_SECTORS).unwrap() * SECTOR;
    device.bytes_mut()[backup] ^= 0xFF;
    assert_eq!(refusal(read(&device)), Error::ArrayChecksum);
}

#[test]
fn a_device_partitioned_the_legacy_way_is_refused_before_anything_else() {
    let mut device = disk();
    device.bytes_mut()[450] = 0x0C;
    assert_eq!(refusal(read(&device)), Error::NotProtective);
}

#[test]
fn a_device_too_small_for_a_table_is_refused_by_both_calls() {
    let small = RamDisk::new(u32::try_from(MIN_SECTORS).unwrap() - 1);
    assert_eq!(refusal(read(&small)), Error::TooSmall(MIN_SECTORS - 1));
    let mut small = small;
    assert_eq!(
        refusal(write(&mut small, &DISK_GUID, &[])),
        Error::TooSmall(MIN_SECTORS - 1)
    );
    let mut smallest = RamDisk::new(u32::try_from(MIN_SECTORS).unwrap());
    let one = Entry::new(
        ESP_TYPE_GUID,
        PARTITION_GUID,
        FIRST_USABLE,
        FIRST_USABLE,
        "x",
    )
    .expect("an entry");
    write(&mut smallest, &DISK_GUID, &[one]).expect("a table");
    let header = read(&smallest).expect("a table");
    assert_eq!(header.first_usable, header.last_usable);
}

#[test]
fn a_device_the_walk_finds_nothing_on_answers_nothing() {
    let device = disk();
    let header = read(&device).expect("a table");
    assert_eq!(find(&device, &header, &OTHER_TYPE).expect("a walk"), None);
    let empty = {
        let mut device = RamDisk::new(SECTORS);
        write(&mut device, &DISK_GUID, &[]).expect("a table");
        device
    };
    let header = read(&empty).expect("a table");
    let mut cursor = Cursor::default();
    assert_eq!(
        next_entry(&empty, &header, &mut cursor).expect("a walk"),
        None
    );
}

#[test]
fn the_walk_yields_every_partition_in_order_and_reads_each_block_once() {
    let mut device = RamDisk::new(SECTORS);
    let entries: Vec<Entry> = (0..8u64)
        .map(|index| {
            let first = PARTITION_START + index * 64;
            Entry::new(
                OTHER_TYPE,
                [u8::try_from(index).unwrap(); GUID_LEN],
                first,
                first + 63,
                "part",
            )
            .expect("an entry")
        })
        .collect();
    write(&mut device, &DISK_GUID, &entries).expect("a table");
    let header = read(&device).expect("a table");
    let mut cursor = Cursor::new();
    let mut seen = Vec::new();
    while let Some(entry) = next_entry(&device, &header, &mut cursor).expect("a walk") {
        seen.push(entry);
    }
    assert_eq!(seen, entries);
}

#[test]
fn an_entry_outside_the_usable_range_is_refused_by_the_walk() {
    let mut device = disk();
    let mut header = read(&device).expect("a table");
    header.first_usable = PARTITION_START + 1;
    put_header(&mut device, &header);
    let header = read(&device).expect("a table");
    assert_eq!(
        refusal(find(&device, &header, &ESP_TYPE_GUID)),
        Error::Partition(PARTITION_START)
    );
}

#[test]
fn a_header_that_names_a_block_the_device_does_not_have_is_refused() {
    let device = disk();
    let mut header = read(&device).expect("a table");
    header.entry_lba = 1 << 33;
    let mut cursor = Cursor::new();
    assert_eq!(
        refusal(next_entry(&device, &header, &mut cursor)),
        Error::Lba(1 << 33)
    );
}

#[test]
fn a_device_that_refuses_a_block_refuses_the_table() {
    let mut device = disk();
    device.refuse(0);
    assert_eq!(refusal(read(&device)), Error::Device(0));
    let mut device = disk();
    device.refuse(1);
    // The primary cannot be read, so the backup answers instead.
    assert_eq!(read(&device).expect("the backup").my_lba, BLOCKS - 1);
    let mut device = disk();
    device.refuse(2);
    assert_eq!(read(&device).expect("the backup").my_lba, BLOCKS - 1);
}

#[test]
fn writing_more_entries_than_the_array_holds_is_refused() {
    let mut device = RamDisk::new(SECTORS);
    let count = usize::try_from(ENTRY_COUNT).unwrap() + 1;
    let entries = vec![esp(); count];
    assert_eq!(
        refusal(write(&mut device, &DISK_GUID, &entries)),
        Error::Space(count)
    );
}

#[test]
fn writing_a_partition_outside_the_usable_range_is_refused() {
    let mut device = RamDisk::new(SECTORS);
    let low = Entry::new(
        ESP_TYPE_GUID,
        PARTITION_GUID,
        FIRST_USABLE - 1,
        PARTITION_START,
        "low",
    )
    .expect("an entry");
    assert_eq!(
        refusal(write(&mut device, &DISK_GUID, &[low])),
        Error::Partition(FIRST_USABLE - 1)
    );
    let high = Entry::new(
        ESP_TYPE_GUID,
        PARTITION_GUID,
        PARTITION_START,
        last_usable(BLOCKS) + 1,
        "high",
    )
    .expect("an entry");
    assert_eq!(
        refusal(write(&mut device, &DISK_GUID, &[high])),
        Error::Partition(PARTITION_START)
    );
}

#[test]
fn writing_two_partitions_over_one_another_is_refused() {
    let mut device = RamDisk::new(SECTORS);
    let first = Entry::new(OTHER_TYPE, PARTITION_GUID, 1024, 1200, "a").expect("an entry");
    let touching = Entry::new(OTHER_TYPE, PARTITION_GUID, 1200, 1300, "b").expect("an entry");
    assert_eq!(
        refusal(write(&mut device, &DISK_GUID, &[first, touching])),
        Error::Partition(1200)
    );
    let beside = Entry::new(OTHER_TYPE, PARTITION_GUID, 1201, 1300, "b").expect("an entry");
    write(&mut device, &DISK_GUID, &[first, beside]).expect("a table");
}

#[test]
fn an_unused_entry_among_the_written_ones_is_no_partition() {
    let mut device = RamDisk::new(SECTORS);
    let unused = Entry::parse(&[0u8; ENTRY_LEN]);
    let used = esp();
    write(&mut device, &DISK_GUID, &[unused, used]).expect("a table");
    let header = read(&device).expect("a table");
    assert_eq!(
        find(&device, &header, &ESP_TYPE_GUID).expect("a walk"),
        Some(used)
    );
}

#[test]
fn a_device_that_refuses_a_write_refuses_the_table() {
    let mut device = RamDisk::new(SECTORS);
    device.refuse(BLOCKS as u32 - 1 - ARRAY_SECTORS as u32);
    assert_eq!(
        refusal(write(&mut device, &DISK_GUID, &[esp()])),
        Error::Device(BLOCKS as u32 - 1 - ARRAY_SECTORS as u32)
    );
}

#[test]
fn the_backup_goes_down_before_the_primary() {
    // A device that refuses the primary header still carries a whole
    // backup, which is what the order is for.
    let mut device = RamDisk::new(SECTORS);
    device.refuse(1);
    assert_eq!(
        refusal(write(&mut device, &DISK_GUID, &[esp()])),
        Error::Device(1)
    );
    let whole = RamDisk::from_bytes(device.bytes().to_vec());
    let header = read(&whole).expect("the backup");
    assert_eq!(header.my_lba, BLOCKS - 1);
}
