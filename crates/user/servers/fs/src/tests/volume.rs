// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::volume` and `crate::partition`.

use audhsos_abi::Error;
use audhsos_time::UnixTime;
use fs_fat::doubles::RamDisk;
use fs_fat::{BlockDevice, FileSystem, FormatOptions, Name, SECTOR};
use fs_gpt::{ESP_TYPE_GUID, Entry, UNUSED_TYPE_GUID};

use crate::partition::Partition;
use crate::tests::support::SECTORS;
use crate::volume::mount;

/// A disk of `SECTORS` sectors with nothing on it.
fn blank() -> RamDisk {
    RamDisk::new(SECTORS)
}

/// The same disk with a partition table naming one partition of `kind`.
fn partitioned(kind: [u8; 16]) -> RamDisk {
    let mut disk = blank();
    let first = 2048u64;
    let last = u64::from(SECTORS).saturating_sub(40);
    let entry = Entry::new(kind, [7u8; 16], first, last, "esp").unwrap();
    fs_gpt::write(&mut disk, &[3u8; 16], &[entry]).expect("a table of one partition is written");
    disk
}

#[test]
fn a_disk_nothing_partitioned_is_formatted_and_then_mounted() {
    let volume = mount(blank()).expect("a blank disk carries a volume once it is formatted");
    let fresh = volume.free_clusters();
    let disk = volume.into_device().device().clone();
    let again = mount(disk).expect("the volume that was written is mounted, not written again");
    assert_eq!(
        again.free_clusters(),
        fresh,
        "a second mount formatted the disk over"
    );
}

#[test]
fn a_disk_that_carries_a_table_is_never_formatted() {
    let disk = partitioned(ESP_TYPE_GUID);
    let before = disk.bytes().to_vec();
    // The partition holds no volume, so the mount is refused — and the
    // bytes of the disk are what they were.
    let outcome = mount(disk);
    assert!(outcome.is_err(), "a partition with no volume is no volume");
    let after = partitioned(ESP_TYPE_GUID);
    assert_eq!(before, after.bytes(), "the disk was written to");
}

#[test]
fn a_table_that_names_no_system_partition_is_refused() {
    let outcome = mount(partitioned(UNUSED_TYPE_GUID));
    assert_eq!(outcome.unwrap_err(), Error::NotFound);
}

/// A partitioned disk whose system partition carries a volume.
fn formatted() -> RamDisk {
    let mut disk = partitioned(ESP_TYPE_GUID);
    let last = u64::from(SECTORS.saturating_sub(40));
    {
        let window = Partition::new(&mut disk, 2048, last).unwrap();
        let _made = FileSystem::format(window, &FormatOptions::default())
            .expect("the partition takes a volume");
    }
    disk
}

/// `disk` with `change` made to its primary header, the array checksum
/// taken again, and the backup header zeroed so that a read has no copy
/// to fall back to.
fn tampered(mut disk: RamDisk, change: impl Fn(&mut fs_gpt::Header)) -> RamDisk {
    let mut header = fs_gpt::read(&disk).expect("a table");
    change(&mut header);
    let start = usize::try_from(header.entry_lba)
        .unwrap()
        .saturating_mul(SECTOR);
    let len = usize::try_from(header.array_len()).unwrap();
    let array = disk.bytes().get(start..start.saturating_add(len)).unwrap();
    header.array_crc = fs_gpt::crc32(array);
    let sector = |lba: u64| {
        let offset = usize::try_from(lba).unwrap().saturating_mul(SECTOR);
        offset..offset.saturating_add(SECTOR)
    };
    let bytes = disk.bytes_mut();
    bytes
        .get_mut(sector(header.my_lba))
        .unwrap()
        .copy_from_slice(&header.write());
    bytes.get_mut(sector(header.alternate_lba)).unwrap().fill(0);
    disk
}

#[test]
fn a_table_whose_usable_range_reaches_past_the_disk_is_a_wrong_table() {
    assert!(mount(tampered(formatted(), |_| {})).is_ok());
    let disk = tampered(formatted(), |header| header.last_usable = u64::MAX);
    assert_eq!(mount(disk).unwrap_err(), Error::InvalidState);
}

#[test]
fn a_table_that_names_too_many_entries_is_a_wrong_table() {
    let count = fs_gpt::MAX_ENTRY_COUNT.saturating_add(1);
    let disk = tampered(formatted(), |header| {
        header.entry_count = count;
        // The usable range starts behind the larger array, so only the
        // count decides.
        header.first_usable = header.entry_lba.saturating_add(header.array_blocks());
    });
    assert_eq!(mount(disk).unwrap_err(), Error::InvalidState);
}

#[test]
fn the_volume_of_a_partitioned_disk_is_the_partition_and_not_the_disk() {
    let mut disk = partitioned(ESP_TYPE_GUID);
    let first = 2048u32;
    let last = SECTORS.saturating_sub(40);
    {
        let window = Partition::new(&mut disk, u64::from(first), u64::from(last)).unwrap();
        let _made = FileSystem::format(window, &FormatOptions::default())
            .expect("the partition takes a volume");
    }
    let volume = mount(disk).expect("the partition carries a volume now");
    assert_eq!(
        volume.device().sectors(),
        last.saturating_sub(first).saturating_add(1),
        "the volume covers the partition and not the whole disk"
    );
}

#[test]
fn a_partition_refuses_a_sector_it_does_not_hold() {
    let mut disk = blank();
    let mut window = Partition::new(&mut disk, 10, 19).unwrap();
    assert_eq!(window.sectors(), 10);
    let mut into = [0u8; SECTOR];
    assert!(window.read(9, &mut into).is_ok());
    assert_eq!(window.read(10, &mut into), Err(fs_fat::Error::Device(10)));
    let from = [1u8; SECTOR];
    assert_eq!(window.write(10, &from), Err(fs_fat::Error::Device(10)));
}

#[test]
fn a_partition_that_runs_past_the_disk_is_no_partition() {
    let mut disk = blank();
    assert!(Partition::new(&mut disk, 0, u64::from(SECTORS)).is_none());
    assert!(Partition::new(&mut disk, 20, 10).is_none());
    assert!(Partition::new(&mut disk, 0, u64::from(u32::MAX).saturating_add(1)).is_none());
}

#[test]
fn a_partition_reads_and_writes_the_sectors_of_the_disk_it_stands_on() {
    let mut disk = blank();
    let from = [0xABu8; SECTOR];
    {
        let mut window = Partition::new(&mut disk, 4, 8).unwrap();
        window.write(0, &from).unwrap();
    }
    let mut into = [0u8; SECTOR];
    disk.read(4, &mut into).unwrap();
    assert_eq!(into, from, "sector zero of the partition is sector four");
}

/// A disk whose first sector is a legacy master boot record: the boot
/// signature at its end and one record of type `0x0C`, a FAT32 partition.
fn legacy_mbr() -> RamDisk {
    let mut disk = blank();
    let mut sector = [0u8; SECTOR];
    sector[446] = 0x80;
    sector[450] = 0x0C;
    sector[454..458].copy_from_slice(&2048u32.to_le_bytes());
    sector[458..462].copy_from_slice(&SECTORS.saturating_sub(2048).to_le_bytes());
    sector[510] = 0x55;
    sector[511] = 0xAA;
    disk.write(0, &sector).unwrap();
    disk
}

#[test]
fn a_disk_with_a_legacy_partition_table_is_refused_and_not_formatted() {
    let disk = legacy_mbr();
    let before = disk.bytes().to_vec();
    let outcome = mount(disk);
    assert_eq!(
        outcome.unwrap_err(),
        Error::InvalidState,
        "a first sector this system does not read is no blank disk"
    );
    assert_eq!(
        before,
        legacy_mbr().bytes(),
        "the records of the legacy table were written over"
    );
}

#[test]
fn a_volume_whose_information_sector_was_damaged_is_mounted_and_not_formatted() {
    let name = Name::new("KEEP.ME").unwrap();
    let mut volume = mount(blank()).expect("a blank disk carries a volume once it is formatted");
    let root = volume.root();
    let _made = volume
        .create(root, &name, UnixTime::from_seconds(315_532_800))
        .unwrap();
    let mut disk = volume.into_device().device().clone();
    // The information sector carries a hint and no volume: the boot
    // sector is what says a volume is there.
    disk.write(1, &[0u8; SECTOR]).unwrap();
    let again = mount(disk).expect("the boot sector still names the volume");
    let root = again.root();
    assert!(
        again.find(root, &name).unwrap().is_some(),
        "the volume was formatted over and its files are gone"
    );
}
