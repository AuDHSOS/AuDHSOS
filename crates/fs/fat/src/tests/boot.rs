// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The boot parameter block: what is read out of a good one, and what
//! every bad one is refused for.

use crate::boot::{
    FormatOptions, Geometry, MIN_CLUSTERS, ROOT_CLUSTER, geometry_for, parse, read_geometry,
};
use crate::device::{BlockDevice, SECTOR, put};
use crate::doubles::RamDisk;
use crate::error::Error;
use crate::fs::FileSystem;
use crate::tests::support::{SECTORS, options, volume};

/// The boot sector of a volume this crate wrote.
fn good() -> [u8; SECTOR] {
    let disk = volume().into_device();
    let mut boot = [0u8; SECTOR];
    disk.read(0, &mut boot).expect("read");
    boot
}

/// The boot sector with `bytes` put at `offset`.
fn patched(offset: usize, bytes: &[u8]) -> [u8; SECTOR] {
    let mut boot = good();
    put(&mut boot, offset, bytes);
    boot
}

#[test]
fn a_volume_this_crate_wrote_reads_back_as_the_geometry_it_was_given() {
    let disk = volume().into_device();
    let geometry = read_geometry(&disk).expect("geometry");
    assert_eq!(geometry.sectors, SECTORS);
    assert_eq!(geometry.sectors_per_cluster, 1);
    assert_eq!(geometry.reserved_sectors, 32);
    assert_eq!(geometry.fat_count, 2);
    assert_eq!(geometry.root_cluster, ROOT_CLUSTER);
    assert!(geometry.clusters >= MIN_CLUSTERS);
    assert_eq!(
        geometry.data_start(),
        geometry.reserved_sectors + geometry.fat_count * geometry.fat_sectors
    );
    assert_eq!(geometry.cluster_sector(ROOT_CLUSTER), geometry.data_start());
    assert_eq!(geometry.cluster_bytes(), 512);
    assert_eq!(geometry.last_cluster(), geometry.clusters + 1);
    assert!(geometry.holds(ROOT_CLUSTER) && geometry.holds(geometry.last_cluster()));
    assert!(!geometry.holds(1) && !geometry.holds(geometry.last_cluster() + 1));
    assert_eq!(
        geometry.fat_start(1),
        geometry.fat_start(0) + geometry.fat_sectors
    );
}

#[test]
fn the_backup_boot_sector_is_the_boot_sector() {
    let disk = volume().into_device();
    let mut backup = [0u8; SECTOR];
    disk.read(crate::boot::BACKUP_BOOT_SECTOR, &mut backup)
        .expect("read");
    assert_eq!(backup, good());
}

#[test]
fn a_sector_size_other_than_512_is_refused() {
    assert_eq!(
        parse(&patched(11, &1024u16.to_le_bytes())),
        Err(Error::SectorSize(1024))
    );
}

#[test]
fn a_cluster_size_that_is_not_a_power_of_two_is_refused() {
    assert_eq!(parse(&patched(13, &[3])), Err(Error::ClusterSize(3)));
    assert_eq!(parse(&patched(13, &[0])), Err(Error::ClusterSize(0)));
    assert_eq!(parse(&patched(13, &[255])), Err(Error::ClusterSize(255)));
    // A power of two above the largest cluster the format settled on.
    assert_eq!(
        geometry_for(
            SECTORS,
            &FormatOptions {
                sectors_per_cluster: 256,
                ..options()
            }
        ),
        Err(Error::ClusterSize(256))
    );
}

#[test]
fn a_volume_with_no_table_at_all_is_refused() {
    assert_eq!(parse(&patched(16, &[0])), Err(Error::Layout));
    assert_eq!(parse(&patched(14, &0u16.to_le_bytes())), Err(Error::Layout));
    assert_eq!(parse(&patched(36, &0u32.to_le_bytes())), Err(Error::Layout));
    assert_eq!(
        geometry_for(
            SECTORS,
            &FormatOptions {
                fat_count: 0,
                ..options()
            }
        ),
        Err(Error::Layout)
    );
    assert_eq!(
        geometry_for(
            SECTORS,
            &FormatOptions {
                reserved_sectors: 0,
                ..options()
            }
        ),
        Err(Error::Layout)
    );
}

#[test]
fn a_fat12_or_fat16_block_is_refused_by_the_fields_it_uses() {
    assert_eq!(
        parse(&patched(17, &512u16.to_le_bytes())),
        Err(Error::NotFat32(0))
    );
    assert_eq!(
        parse(&patched(22, &200u16.to_le_bytes())),
        Err(Error::NotFat32(0))
    );
    assert_eq!(
        parse(&patched(19, &1000u16.to_le_bytes())),
        Err(Error::NotFat32(0))
    );
}

#[test]
fn a_block_with_no_signature_is_refused() {
    assert_eq!(parse(&patched(510, &[0, 0])), Err(Error::Signature));
}

#[test]
fn a_volume_of_too_few_clusters_is_not_a_fat32_one() {
    let mut boot = good();
    put(&mut boot, 32, &40_000u32.to_le_bytes());
    match parse(&boot) {
        Err(Error::NotFat32(clusters)) => assert!(clusters < MIN_CLUSTERS),
        other => panic!("a small volume parsed as {other:?}"),
    }
    assert!(matches!(
        geometry_for(1024, &options()),
        Err(Error::NotFat32(_))
    ));
}

#[test]
fn a_device_too_small_for_the_tables_is_refused() {
    assert_eq!(geometry_for(0, &options()), Err(Error::TooSmall(0)));
    assert_eq!(geometry_for(31, &options()), Err(Error::TooSmall(31)));
    assert_eq!(geometry_for(33, &options()), Err(Error::TooSmall(33)));
}

#[test]
fn a_root_directory_outside_the_data_region_is_refused() {
    assert_eq!(
        parse(&patched(44, &1u32.to_le_bytes())),
        Err(Error::Cluster(1))
    );
    assert_eq!(
        parse(&patched(44, &9_000_000u32.to_le_bytes())),
        Err(Error::Cluster(9_000_000))
    );
}

#[test]
fn a_device_that_will_not_give_up_its_first_sector_says_so() {
    let mut disk = RamDisk::new(SECTORS);
    disk.refuse(0);
    assert_eq!(read_geometry(&disk), Err(Error::Device(0)));
    assert_eq!(FileSystem::mount(disk).err(), Some(Error::Device(0)));
}

#[test]
fn a_larger_cluster_needs_a_smaller_table() {
    let small = geometry_for(SECTORS, &options()).expect("one sector per cluster");
    let large = geometry_for(
        SECTORS * 8,
        &FormatOptions {
            sectors_per_cluster: 8,
            ..options()
        },
    )
    .expect("eight sectors per cluster");
    assert!(large.clusters > small.clusters);
    assert!(large.clusters < small.clusters + 1000);
    assert!(large.fat_sectors < small.fat_sectors * 8);
    assert_eq!(large.cluster_bytes(), 4096);
}

#[test]
fn the_default_options_are_a_volume_without_a_name() {
    let plain = FormatOptions::default();
    assert_eq!(&plain.volume_label, b"NO NAME    ");
    assert_eq!(plain.volume_id, 0);
    assert_eq!(plain.sectors_per_cluster, 1);
    let geometry: Geometry = geometry_for(SECTORS, &plain).expect("geometry");
    assert_eq!(geometry.sectors, SECTORS);
}
