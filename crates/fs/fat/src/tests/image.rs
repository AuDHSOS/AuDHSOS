// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A volume laid out the way the image writer lays one out — nested
//! directories, files of every awkward length — written, unmounted,
//! mounted again, and read back.

use std::collections::BTreeMap;

use crate::dir::Dir;
use crate::doubles::RamDisk;
use crate::error::Error;
use crate::fs::FileSystem;
use crate::name::Name;
use crate::tests::support::{SECTORS, moment, options};

/// The files an image of this project carries, with the lengths that sit
/// on the boundaries: nothing, less than a cluster, exactly one, more
/// than one, and a part of one after several whole ones.
fn files() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("EFI/BOOT/BOOTX64.EFI", vec![0xAA; 4096]),
        ("AUDHSOS/KERNEL.ELF", vec![0xBB; 5000]),
        ("AUDHSOS/BOOT.IMG", Vec::new()),
        ("AUDHSOS/EXACT.BIN", vec![0xCC; 512]),
        ("AUDHSOS/MANY.BIN", vec![0xDD; 512 * 40 + 7]),
        ("README.TXT", vec![0xEE; 1]),
    ]
}

/// Writes `files` onto a fresh volume, by their `/`-separated paths.
fn write_all(volume: &mut FileSystem<RamDisk>, files: &[(&str, Vec<u8>)]) -> Result<(), Error> {
    for (path, data) in files {
        let mut directory = volume.root();
        let mut components = path.split('/').peekable();
        while let Some(component) = components.next() {
            let name = Name::new(component)?;
            if components.peek().is_some() {
                directory = volume.open_or_create_dir(directory, &name, moment())?;
                continue;
            }
            let mut file = volume.create(directory, &name, moment())?;
            volume.write(&mut file, 0, data)?;
        }
    }
    volume.flush()
}

/// Every file of the volume, by its `/`-separated path.
fn read_all(volume: &FileSystem<RamDisk>) -> Result<BTreeMap<String, Vec<u8>>, Error> {
    let mut found = BTreeMap::new();
    collect(volume, volume.root(), "", &mut found)?;
    Ok(found)
}

/// Puts every file of `directory` and of what is below it into `found`.
fn collect(
    volume: &FileSystem<RamDisk>,
    directory: Dir,
    prefix: &str,
    found: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), Error> {
    let mut cursor = volume.entries(directory);
    while let Some(entry) = volume.next_entry(&mut cursor)? {
        let path = if prefix.is_empty() {
            std::format!("{}", entry.name)
        } else {
            std::format!("{prefix}/{}", entry.name)
        };
        if entry.is_directory() {
            collect(volume, Dir::at(entry.first_cluster), &path, found)?;
            continue;
        }
        let mut file = volume.open(directory, &entry.name)?;
        let mut bytes = vec![0u8; usize::try_from(entry.size).unwrap_or(0)];
        volume.read(&mut file, 0, &mut bytes)?;
        found.insert(path, bytes);
    }
    Ok(())
}

#[test]
fn an_image_written_and_mounted_again_holds_every_file_it_was_given() {
    let mut volume = FileSystem::format(RamDisk::new(SECTORS), &options()).expect("format");
    let files = files();
    write_all(&mut volume, &files).expect("write");
    let free = volume.free_clusters();
    let volume = FileSystem::mount(volume.into_device()).expect("mount");
    assert_eq!(volume.free_clusters(), free, "the count survives a mount");
    let found = read_all(&volume).expect("read");
    assert_eq!(found.len(), files.len());
    for (path, bytes) in &files {
        assert_eq!(found.get(*path), Some(bytes), "{path}");
    }
}

#[test]
fn the_same_files_written_twice_make_the_same_bytes() {
    let files = files();
    let mut first = FileSystem::format(RamDisk::new(SECTORS), &options()).expect("format");
    write_all(&mut first, &files).expect("write");
    let mut second = FileSystem::format(RamDisk::new(SECTORS), &options()).expect("format");
    write_all(&mut second, &files).expect("write");
    assert_eq!(first.into_device(), second.into_device());
}

#[test]
fn every_copy_of_the_table_says_the_same_thing() {
    let mut volume = FileSystem::format(RamDisk::new(SECTORS), &options()).expect("format");
    write_all(&mut volume, &files()).expect("write");
    let geometry = *volume.geometry();
    let disk = volume.into_device();
    let bytes = disk.bytes();
    let sector = usize::try_from(geometry.fat_sectors).expect("sectors") * crate::SECTOR;
    let first = usize::try_from(geometry.fat_start(0)).expect("start") * crate::SECTOR;
    let second = usize::try_from(geometry.fat_start(1)).expect("start") * crate::SECTOR;
    assert_eq!(
        bytes.get(first..first + sector),
        bytes.get(second..second + sector)
    );
}
