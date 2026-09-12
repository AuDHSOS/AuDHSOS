// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::image`, covering the catalog items 6.6.15.

#![allow(clippy::arithmetic_side_effects)]

use audhsos_abi::boot_image::BootImageHeader;
use audhsos_abi::layout::PAGE_SIZE;

use crate::image::{boot_image, disk, fat32, gpt};
use fs_gpt::crc32;

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn sample_files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (disk::LOADER_PATH, vec![0xAA; 4096]),
        (disk::KERNEL_PATH, vec![0xBB; 5000]),
        (disk::BOOT_IMAGE_PATH, vec![0xCC; 512]),
    ]
}

#[test]
fn the_protective_record_covers_the_whole_disk() {
    let image = disk::try_build(&sample_files(), 64 * 1024 * 1024).unwrap();
    let sectors = u64::try_from(image.len() / gpt::SECTOR).unwrap();
    assert_eq!(&image[510..512], &[0x55, 0xAA]);
    let entry = &image[446..462];
    assert_eq!(entry[0], 0x00, "not bootable");
    assert_eq!(entry[4], 0xEE, "the protective type");
    assert_eq!(read_u32(entry, 8), 1, "it starts behind the record");
    assert_eq!(
        u64::from(read_u32(entry, 12)),
        sectors - 1,
        "and covers the rest"
    );
}

#[test]
fn the_two_headers_name_each_other_and_their_checksums_verify() {
    let image = disk::try_build(&sample_files(), 64 * 1024 * 1024).unwrap();
    let sectors = u64::try_from(image.len() / gpt::SECTOR).unwrap();
    let last = sectors - 1;
    let primary = &image[gpt::sector_offset(1)..gpt::sector_offset(2)];
    let backup = &image[gpt::sector_offset(last)..gpt::sector_offset(last + 1)];

    for (header, current, other) in [(primary, 1, last), (backup, last, 1)] {
        assert_eq!(&header[..8], &fs_gpt::HEADER_SIGNATURE);
        assert_eq!(read_u32(header, 8), fs_gpt::HEADER_REVISION);
        assert_eq!(
            read_u32(header, 12),
            u32::try_from(fs_gpt::HEADER_LEN).unwrap()
        );
        assert_eq!(read_u64(header, 24), current);
        assert_eq!(read_u64(header, 32), other);
        assert_eq!(read_u64(header, 40), gpt::FIRST_USABLE);
        assert_eq!(read_u64(header, 48), last - fs_gpt::ARRAY_SECTORS - 1);
        assert_eq!(&header[56..72], &gpt::DISK_GUID);
        assert_eq!(read_u32(header, 80), fs_gpt::ENTRY_COUNT);
        assert_eq!(
            read_u32(header, 84),
            u32::try_from(fs_gpt::ENTRY_LEN).unwrap()
        );

        let mut zeroed = header[..fs_gpt::HEADER_LEN].to_vec();
        let stored = read_u32(&zeroed, 16);
        zeroed[16..20].fill(0);
        assert_eq!(crc32(&zeroed), stored, "the header checksum verifies");
    }
    assert_eq!(
        read_u64(primary, 72),
        2,
        "the primary array follows the primary header"
    );
    assert_eq!(
        read_u64(backup, 72),
        last - fs_gpt::ARRAY_SECTORS,
        "the backup array sits before the backup header"
    );
}

#[test]
fn the_partition_array_holds_one_entry_and_verifies() {
    let image = disk::try_build(&sample_files(), 64 * 1024 * 1024).unwrap();
    let sectors = u64::try_from(image.len() / gpt::SECTOR).unwrap();
    let last = sectors - 1;
    let array_len = fs_gpt::ENTRY_LEN * usize::try_from(fs_gpt::ENTRY_COUNT).unwrap();
    let primary = &image[gpt::sector_offset(2)..gpt::sector_offset(2) + array_len];
    let backup_start = gpt::sector_offset(last - fs_gpt::ARRAY_SECTORS);
    let backup = &image[backup_start..backup_start + array_len];
    assert_eq!(primary, backup, "both copies are identical");

    let stored = read_u32(&image[gpt::sector_offset(1)..], 88);
    assert_eq!(crc32(primary), stored, "the array checksum verifies");

    assert_eq!(&primary[..16], &gpt::ESP_TYPE_GUID);
    assert_eq!(&primary[16..32], &gpt::PARTITION_GUID);
    assert_eq!(read_u64(primary, 32), gpt::PARTITION_START);
    assert_eq!(read_u64(primary, 40), last - fs_gpt::ARRAY_SECTORS - 1);
    assert_eq!(read_u64(primary, 48), 0, "no attributes");
    let name: String = (0..gpt::PARTITION_NAME.len())
        .map(|index| char::from(u8::try_from(read_u16(primary, 56 + index * 2)).unwrap()))
        .collect();
    assert_eq!(name, gpt::PARTITION_NAME);
    assert!(
        primary[fs_gpt::ENTRY_LEN..].iter().all(|byte| *byte == 0),
        "exactly one entry is in use"
    );
}

#[test]
fn a_disk_too_small_for_the_structures_is_rejected() {
    let mut image = vec![0u8; gpt::SECTOR * 64];
    assert!(gpt::write(&mut image, 64).is_err());
    assert!(gpt::write(&mut image, gpt::MIN_SECTORS - 1).is_err());
    let mut large = vec![0u8; gpt::SECTOR * usize::try_from(gpt::MIN_SECTORS).unwrap()];
    assert!(gpt::write(&mut large, gpt::MIN_SECTORS).is_ok());
}

#[test]
fn the_boot_sector_carries_the_geometry_and_the_backup_equals_it() {
    let image = disk::try_build(&sample_files(), 64 * 1024 * 1024).unwrap();
    let partition = disk::partition(&image).unwrap();
    let boot = &partition[..fat32::SECTOR];
    assert_eq!(read_u16(boot, 11), 512, "bytes per sector");
    assert_eq!(boot[13], 1, "sectors per cluster");
    assert_eq!(read_u16(boot, 14), 32, "reserved sectors");
    assert_eq!(boot[16], 2, "two tables");
    assert_eq!(boot[21], fat32::MEDIA);
    assert_eq!(read_u16(boot, 22), 0, "no sixteen-bit table size");
    assert_eq!(read_u32(boot, 44), fat32::ROOT_CLUSTER);
    assert_eq!(read_u16(boot, 48), fat32::FSINFO_SECTOR);
    assert_eq!(read_u16(boot, 50), fat32::BACKUP_BOOT_SECTOR);
    assert_eq!(read_u32(boot, 67), fat32::VOLUME_ID);
    assert_eq!(&boot[71..82], fat32::VOLUME_LABEL);
    assert_eq!(&boot[82..90], b"FAT32   ");
    assert_eq!(&boot[510..512], &[0x55, 0xAA]);

    let backup_start = usize::from(fat32::BACKUP_BOOT_SECTOR) * fat32::SECTOR;
    assert_eq!(
        &partition[backup_start..backup_start + fat32::SECTOR],
        boot,
        "the backup boot sector equals the boot sector"
    );
}

#[test]
fn the_information_sector_reports_the_free_clusters() {
    let image = disk::try_build(&sample_files(), 64 * 1024 * 1024).unwrap();
    let partition = disk::partition(&image).unwrap();
    let start = usize::from(fat32::FSINFO_SECTOR) * fat32::SECTOR;
    let info = &partition[start..start + fat32::SECTOR];
    assert_eq!(read_u32(info, 0), 0x4161_5252);
    assert_eq!(read_u32(info, 484), 0x6141_7272);
    assert_eq!(read_u32(info, 508), 0xAA55_0000);
    let geometry =
        fat32::geometry(u32::try_from(partition.len() / fat32::SECTOR).unwrap()).unwrap();
    let free = read_u32(info, 488);
    let next = read_u32(info, 492);
    assert!(free < geometry.clusters && free > 0);
    assert_eq!(free + (next - fat32::ROOT_CLUSTER), geometry.clusters);
}

#[test]
fn the_two_tables_are_identical_and_start_with_the_media_marker() {
    let image = disk::try_build(&sample_files(), 64 * 1024 * 1024).unwrap();
    let partition = disk::partition(&image).unwrap();
    let geometry =
        fat32::geometry(u32::try_from(partition.len() / fat32::SECTOR).unwrap()).unwrap();
    let len = usize::try_from(geometry.fat_sectors).unwrap() * fat32::SECTOR;
    let first = usize::try_from(fat32::RESERVED_SECTORS).unwrap() * fat32::SECTOR;
    let second = first + len;
    assert_eq!(
        &partition[first..first + len],
        &partition[second..second + len]
    );
    let table = &partition[first..first + len];
    assert_eq!(read_u32(table, 0), 0x0FFF_FF00 | u32::from(fat32::MEDIA));
    assert_eq!(read_u32(table, 4), fat32::END_OF_CHAIN);
    assert_eq!(
        read_u32(table, usize::try_from(fat32::ROOT_CLUSTER).unwrap() * 4),
        fat32::END_OF_CHAIN,
        "the root directory fits into one cluster and ends there"
    );
}

#[test]
fn a_partition_with_too_few_clusters_is_rejected() {
    assert!(fat32::geometry(1024).is_err());
    assert!(fat32::geometry(fat32::MIN_CLUSTERS + fat32::RESERVED_SECTORS).is_err());
    let geometry = fat32::geometry(70_000).unwrap();
    assert!(geometry.clusters >= fat32::MIN_CLUSTERS);
    assert_eq!(
        geometry.data_start(),
        fat32::RESERVED_SECTORS + 2 * geometry.fat_sectors
    );
    assert_eq!(
        geometry.cluster_sector(fat32::ROOT_CLUSTER),
        geometry.data_start()
    );
    assert!(fat32::geometry(0).is_err());
}

#[test]
fn every_written_file_reads_back_unchanged() {
    let files = vec![
        (disk::LOADER_PATH, vec![0x11; 4096]),
        (disk::KERNEL_PATH, vec![0x22; 5000]),
        (disk::BOOT_IMAGE_PATH, Vec::new()),
        ("AUDHSOS/EXACT.BIN", vec![0x33; 512]),
        ("AUDHSOS/MANY.BIN", vec![0x44; 512 * 40 + 7]),
    ];
    let image = disk::build(&files).unwrap();
    let partition = disk::partition(&image).unwrap();
    let read = fat32::read(partition).unwrap();
    assert_eq!(read.len(), files.len());
    for (path, bytes) in &files {
        assert_eq!(read.get(*path), Some(bytes), "{path}");
    }
}

#[test]
fn a_directory_whose_entries_span_more_than_one_cluster_reads_back() {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for index in 0..40u32 {
        files.push((
            format!("AUDHSOS/F{index:03}.BIN"),
            vec![u8::try_from(index % 256).unwrap(); 8],
        ));
    }
    let owned: Vec<(&str, Vec<u8>)> = files
        .iter()
        .map(|(path, bytes)| (path.as_str(), bytes.clone()))
        .collect();
    let image = disk::build(&owned).unwrap();
    let partition = disk::partition(&image).unwrap();
    let read = fat32::read(partition).unwrap();
    assert_eq!(read.len(), 40);
    for (path, bytes) in &files {
        assert_eq!(read.get(path), Some(bytes), "{path}");
    }
}

#[test]
fn eight_three_names_are_converted_and_the_others_rejected() {
    assert_eq!(&fat32::short_name("KERNEL.ELF").unwrap(), b"KERNEL  ELF");
    assert_eq!(&fat32::short_name("kernel.elf").unwrap(), b"KERNEL  ELF");
    assert_eq!(&fat32::short_name("EFI").unwrap(), b"EFI        ");
    assert_eq!(&fat32::short_name("BOOTX64.EFI").unwrap(), b"BOOTX64 EFI");
    assert!(fat32::short_name("").is_err(), "an empty name");
    assert!(fat32::short_name("TOOLONGNAME.BIN").is_err(), "a long stem");
    assert!(fat32::short_name("NAME.LONG").is_err(), "a long extension");
    assert!(fat32::short_name("NAME.").is_err(), "an empty extension");
    assert!(fat32::short_name("NA ME").is_err(), "a space");
    assert!(
        fat32::short_name("NA+ME").is_err(),
        "a character not allowed"
    );
}

#[test]
fn the_nested_directories_carry_dot_and_dot_dot() {
    let image = disk::build(&sample_files()).unwrap();
    let partition = disk::partition(&image).unwrap();
    let geometry =
        fat32::geometry(u32::try_from(partition.len() / fat32::SECTOR).unwrap()).unwrap();
    // The root holds EFI and AUDHSOS; EFI holds BOOT; BOOT holds the loader.
    let root =
        usize::try_from(geometry.cluster_sector(fat32::ROOT_CLUSTER)).unwrap() * fat32::SECTOR;
    assert_eq!(&partition[root..root + 11], b"EFI        ");
    assert_eq!(partition[root + 11], fat32::ATTR_DIRECTORY);
    let efi_cluster = u32::from(read_u16(&partition[root..], 20)) << 16
        | u32::from(read_u16(&partition[root..], 26));
    let efi = usize::try_from(geometry.cluster_sector(efi_cluster)).unwrap() * fat32::SECTOR;
    assert_eq!(&partition[efi..efi + 11], b".          ");
    assert_eq!(
        &partition[efi + fat32::ENTRY_LEN..efi + fat32::ENTRY_LEN + 11],
        b"..         "
    );
    assert_eq!(read_u16(&partition[root..], 16), fat32::DATE);
    assert_eq!(read_u16(&partition[root..], 14), fat32::TIME);
}

#[test]
fn the_image_has_the_default_size_and_grows_in_whole_mebibytes() {
    let small = disk::build(&sample_files()).unwrap();
    assert_eq!(u64::try_from(small.len()).unwrap(), disk::DEFAULT_SIZE);

    let large = disk::build(&[(disk::KERNEL_PATH, vec![0x55; 80 * 1024 * 1024])]).unwrap();
    let size = u64::try_from(large.len()).unwrap();
    assert!(size > disk::DEFAULT_SIZE);
    assert_eq!(size % disk::GROWTH_STEP, 0);
    let partition = disk::partition(&large).unwrap();
    assert_eq!(
        fat32::read(partition)
            .unwrap()
            .get(disk::KERNEL_PATH)
            .map(Vec::len),
        Some(80 * 1024 * 1024)
    );
}

#[test]
fn files_that_exceed_the_partition_are_rejected() {
    let too_large = fat32::write(
        &mut vec![0u8; 70_000 * fat32::SECTOR],
        &[("BIG.BIN", vec![0u8; 70_000 * fat32::SECTOR])],
    );
    assert!(too_large.is_err());
    let duplicate = fat32::write(
        &mut vec![0u8; 70_000 * fat32::SECTOR],
        &[("A.BIN", vec![1]), ("A.BIN", vec![2])],
    );
    assert!(duplicate.is_err(), "the same path twice");
}

#[test]
fn the_boot_image_is_one_the_kernels_parser_accepts() {
    let root_task = boot_image::placeholder_root_task();
    let image = boot_image::build(&root_task, &[], 0).unwrap();
    let header = BootImageHeader::parse(
        &image,
        u64::try_from(image.len()).unwrap(),
        64 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(header.root_task_offset, boot_image::ROOT_TASK_OFFSET);
    assert_eq!(header.root_task_len, PAGE_SIZE);
    assert_eq!(header.archive_len, 0);
    assert_eq!(header.kernel_reserve_size, 0);
    assert_eq!(u64::try_from(image.len()).unwrap(), 2 * PAGE_SIZE);
    assert!(
        image[usize::try_from(PAGE_SIZE).unwrap()..]
            .iter()
            .all(|byte| *byte == boot_image::PLACEHOLDER_BYTE)
    );
}

#[test]
fn the_boot_image_carries_an_archive_behind_the_root_task() {
    let root_task = vec![0x90; 5000];
    let archive = vec![0x77; 1024];
    let image = boot_image::build(&root_task, &archive, 8 * 1024 * 1024).unwrap();
    let header = BootImageHeader::parse(
        &image,
        u64::try_from(image.len()).unwrap(),
        64 * 1024 * 1024,
    )
    .unwrap();
    assert_eq!(header.root_task_len, 5000);
    assert_eq!(header.archive_offset % PAGE_SIZE, 0);
    assert!(header.archive_offset >= header.root_task_offset + header.root_task_len);
    assert_eq!(header.archive_len, 1024);
    assert_eq!(header.kernel_reserve_size, 8 * 1024 * 1024);
    let start = usize::try_from(header.archive_offset).unwrap();
    assert_eq!(&image[start..start + 1024], archive.as_slice());
}

#[test]
fn an_empty_root_task_or_an_unaligned_reserve_is_rejected() {
    assert!(boot_image::build(&[], &[], 0).is_err());
    assert!(boot_image::build(&[0x90], &[], PAGE_SIZE + 1).is_err());
    assert!(boot_image::build(&[0x90], &[], PAGE_SIZE).is_ok());
}
