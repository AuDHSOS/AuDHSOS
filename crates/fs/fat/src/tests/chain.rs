// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Cluster chains and what is done with them: walking one, taking
//! clusters, and giving them back.

use crate::boot::ROOT_CLUSTER;
use crate::device::{BlockDevice, SECTOR, put};
use crate::doubles::RamDisk;
use crate::error::Error;
use crate::fs::FileSystem;
use crate::name::Name;
use crate::table::{BAD_CLUSTER, END_OF_CHAIN};
use crate::tests::support::{SECTORS, moment, options, volume};

/// Writes `value` into the entry of `cluster`, in the first table only,
/// which is how a test makes a chain no writer of this crate would.
fn set_raw(disk: &mut RamDisk, fat_start: u32, cluster: u32, value: u32) {
    let sector = fat_start.saturating_add(cluster / 128);
    let mut buffer = [0u8; SECTOR];
    disk.read(sector, &mut buffer).expect("read");
    put(
        &mut buffer,
        usize::try_from(cluster % 128)
            .expect("offset")
            .saturating_mul(4),
        &value.to_le_bytes(),
    );
    disk.write(sector, &buffer).expect("write");
}

#[test]
fn a_file_of_one_cluster_is_a_chain_of_one() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("ONE.BIN").expect("name");
    let mut file = fs.create(root, &name, moment()).expect("create");
    fs.write(&mut file, 0, &[7u8; 100]).expect("write");
    assert_eq!(fs.chain_length(file.first_cluster()), Ok(1));
    assert_eq!(fs.next_cluster(file.first_cluster()), Ok(None));
}

#[test]
fn a_file_of_many_clusters_is_a_chain_of_many() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("MANY.BIN").expect("name");
    let mut file = fs.create(root, &name, moment()).expect("create");
    fs.write(&mut file, 0, &[7u8; 512 * 5 + 3]).expect("write");
    assert_eq!(fs.chain_length(file.first_cluster()), Ok(6));
    let mut cluster = file.first_cluster();
    let mut seen = 1;
    while let Some(following) = fs.next_cluster(cluster).expect("walk") {
        cluster = following;
        seen += 1;
    }
    assert_eq!(seen, 6);
}

#[test]
fn a_chain_that_points_back_into_itself_is_refused() {
    let fs = volume();
    let start = fs.geometry().fat_start(0);
    let clusters = fs.geometry().clusters;
    let root = fs.root();
    let mut disk = fs.into_device();
    set_raw(&mut disk, start, ROOT_CLUSTER, ROOT_CLUSTER + 1);
    set_raw(&mut disk, start, ROOT_CLUSTER + 1, ROOT_CLUSTER);
    let fs = FileSystem::mount(disk).expect("mount");
    assert_eq!(
        fs.chain_length(ROOT_CLUSTER),
        Err(Error::ChainLoop(ROOT_CLUSTER))
    );
    assert!(clusters > 0);
    let mut cursor = fs.entries(root);
    assert!(matches!(
        fs.next_entry(&mut cursor),
        Ok(_) | Err(Error::ChainLoop(_))
    ));
}

#[test]
fn a_chain_that_reaches_a_free_cluster_is_refused() {
    let fs = volume();
    let start = fs.geometry().fat_start(0);
    let mut disk = fs.into_device();
    set_raw(&mut disk, start, ROOT_CLUSTER, ROOT_CLUSTER + 5);
    let fs = FileSystem::mount(disk).expect("mount");
    assert_eq!(
        fs.chain_length(ROOT_CLUSTER),
        Err(Error::FreeInChain(ROOT_CLUSTER + 5))
    );
}

#[test]
fn a_chain_that_leaves_the_table_is_refused() {
    let fs = volume();
    let start = fs.geometry().fat_start(0);
    let last = fs.geometry().last_cluster();
    let mut disk = fs.into_device();
    set_raw(&mut disk, start, ROOT_CLUSTER, last + 1);
    let fs = FileSystem::mount(disk).expect("mount");
    assert_eq!(fs.chain_length(ROOT_CLUSTER), Err(Error::Cluster(last + 1)));
    assert_eq!(fs.next_cluster(1), Err(Error::Cluster(1)));
    assert_eq!(fs.next_cluster(last + 1), Err(Error::Cluster(last + 1)));
}

#[test]
fn a_chain_that_reaches_the_bad_cluster_marker_is_refused() {
    let fs = volume();
    let start = fs.geometry().fat_start(0);
    let mut disk = fs.into_device();
    set_raw(&mut disk, start, ROOT_CLUSTER, BAD_CLUSTER);
    let fs = FileSystem::mount(disk).expect("mount");
    assert_eq!(
        fs.chain_length(ROOT_CLUSTER),
        Err(Error::BadCluster(ROOT_CLUSTER))
    );
}

#[test]
fn the_end_of_a_chain_is_every_marker_the_format_allows() {
    let fs = volume();
    let start = fs.geometry().fat_start(0);
    let mut disk = fs.into_device();
    set_raw(&mut disk, start, ROOT_CLUSTER, crate::table::END_MARK);
    let fs = FileSystem::mount(disk).expect("mount");
    assert_eq!(fs.next_cluster(ROOT_CLUSTER), Ok(None));
}

#[test]
fn a_fresh_volume_has_every_cluster_but_the_root_free() {
    let fs = volume();
    assert_eq!(fs.free_clusters(), fs.geometry().clusters - 1);
    assert_eq!(fs.next_free(), ROOT_CLUSTER + 1);
}

#[test]
fn a_mounted_volume_counts_what_the_table_says_and_not_what_the_note_says() {
    let mut fs = volume();
    let name = Name::new("A.BIN").expect("name");
    let root = fs.root();
    let mut file = fs.create(root, &name, moment()).expect("create");
    fs.write(&mut file, 0, &[1u8; 2000]).expect("write");
    let free = fs.free_clusters();
    let mut disk = fs.into_device();
    // A note that lies: the mount counts instead of believing it.
    let mut info = [0u8; SECTOR];
    disk.read(crate::boot::FSINFO_SECTOR, &mut info)
        .expect("read");
    put(&mut info, 488, &7u32.to_le_bytes());
    disk.write(crate::boot::FSINFO_SECTOR, &info)
        .expect("write");
    let fs = FileSystem::mount(disk).expect("mount");
    assert_eq!(fs.free_clusters(), free);
    let note = crate::fs::info(&fs.into_device()).expect("info");
    assert_eq!(note.map(|(stored, _)| stored), Some(7));
}

#[test]
fn the_note_of_a_sector_that_is_not_one_is_no_note() {
    let mut disk = RamDisk::new(SECTORS);
    assert_eq!(crate::fs::info(&disk).expect("info"), None);
    disk.refuse(crate::boot::FSINFO_SECTOR);
    assert!(crate::fs::info(&disk).is_err());
}

#[test]
fn a_freed_chain_gives_back_every_cluster_it_had() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("GONE.BIN").expect("name");
    let before = fs.free_clusters();
    let mut file = fs.create(root, &name, moment()).expect("create");
    fs.write(&mut file, 0, &[9u8; 512 * 4]).expect("write");
    assert_eq!(fs.free_clusters(), before - 4);
    assert_eq!(fs.remove(root, &name), Ok(4));
    assert_eq!(fs.free_clusters(), before);
    assert_eq!(fs.find(root, &name), Ok(None));
}

#[test]
fn a_file_with_no_cluster_frees_none() {
    let mut fs = volume();
    let root = fs.root();
    let name = Name::new("EMPTY.BIN").expect("name");
    fs.create(root, &name, moment()).expect("create");
    let before = fs.free_clusters();
    assert_eq!(fs.remove(root, &name), Ok(0));
    assert_eq!(fs.free_clusters(), before);
}

#[test]
fn removing_what_is_not_there_and_what_is_a_directory_is_refused() {
    let mut fs = volume();
    let root = fs.root();
    let missing = Name::new("NONE.BIN").expect("name");
    assert_eq!(fs.remove(root, &missing), Err(Error::NotFound));
    let folder = Name::new("SUB").expect("name");
    fs.create_dir(root, &folder, moment()).expect("create dir");
    assert_eq!(fs.remove(root, &folder), Err(Error::Kind));
}

#[test]
fn allocating_in_a_full_table_is_an_error() {
    let mut fs = FileSystem::format(RamDisk::new(SECTORS), &options()).expect("format");
    let free = fs.free_clusters();
    for _ in 0..free {
        fs.allocate().expect("allocate");
    }
    assert_eq!(fs.free_clusters(), 0);
    assert_eq!(fs.allocate(), Err(Error::Full));
    let root = fs.root();
    let name = Name::new("NOROOM.BIN").expect("name");
    let mut file = fs.create(root, &name, moment()).expect("create");
    assert_eq!(fs.write(&mut file, 0, &[1]), Err(Error::Full));
}

#[test]
fn a_full_directory_that_cannot_grow_is_an_error() {
    let mut fs = FileSystem::format(RamDisk::new(SECTORS), &options()).expect("format");
    // Sixteen entries fit into the root cluster; the seventeenth needs a
    // cluster the table no longer has.
    for index in 0..16u32 {
        let root = fs.root();
        let name = Name::new(&alloc_name(index)).expect("name");
        fs.create(root, &name, moment()).expect("create");
    }
    let free = fs.free_clusters();
    for _ in 0..free {
        fs.allocate().expect("allocate");
    }
    let root = fs.root();
    let name = Name::new("LAST.BIN").expect("name");
    assert_eq!(fs.create(root, &name, moment()), Err(Error::Full));
}

/// A distinct 8.3 name for the file numbered `index`.
fn alloc_name(index: u32) -> String {
    format!("F{index:03}.BIN")
}

#[test]
fn a_table_entry_keeps_the_four_bits_that_are_not_the_crates() {
    let fs = volume();
    let start = fs.geometry().fat_start(0);
    let mut disk = fs.into_device();
    set_raw(&mut disk, start, ROOT_CLUSTER + 1, 0xF000_0000);
    let mut fs = FileSystem::mount(disk).expect("mount");
    let cluster = fs.allocate().expect("allocate");
    assert_eq!(cluster, ROOT_CLUSTER + 1, "the entry reads as free");
    let start = fs.geometry().fat_start(0);
    let disk = fs.into_device();
    let mut buffer = [0u8; SECTOR];
    disk.read(start, &mut buffer).expect("read");
    let raw = u32::from_le_bytes(
        buffer[usize::try_from(ROOT_CLUSTER + 1).expect("offset") * 4..][..4]
            .try_into()
            .expect("four bytes"),
    );
    assert_eq!(raw, 0xF000_0000 | END_OF_CHAIN);
}
