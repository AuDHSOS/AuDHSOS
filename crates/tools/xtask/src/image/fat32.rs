// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The FAT32 file system inside the EFI system partition: one sector per
//! cluster, two identical file allocation tables, 8.3 names, and
//! deterministic timestamps, so that two runs produce the same image.
//!
//! Invariants: every chain ends with the end-of-chain marker; the two
//! tables are identical; the backup boot sector equals the boot sector;
//! the file system holds at least [`MIN_CLUSTERS`] clusters, which is what
//! makes it FAT32 rather than FAT16.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "the offsets are bounded by the sector and cluster sizes this module defines"
)]

#[cfg(test)]
use std::collections::BTreeMap;

use crate::error::Error;

/// Size of one sector in bytes.
pub(crate) const SECTOR: usize = 512;

/// Sectors per cluster.
pub(crate) const SECTORS_PER_CLUSTER: u32 = 1;

/// Sectors before the first file allocation table.
pub(crate) const RESERVED_SECTORS: u32 = 32;

/// Number of file allocation tables.
pub(crate) const FAT_COUNT: u32 = 2;

/// Cluster of the root directory.
pub(crate) const ROOT_CLUSTER: u32 = 2;

/// Media descriptor of a fixed disk.
pub(crate) const MEDIA: u8 = 0xF8;

/// The value that ends a cluster chain.
pub(crate) const END_OF_CHAIN: u32 = 0x0FFF_FFFF;

/// Fewer clusters than this would make the file system FAT16.
pub(crate) const MIN_CLUSTERS: u32 = 65525;

/// Sector of the file system information structure.
pub(crate) const FSINFO_SECTOR: u16 = 1;

/// Sector of the copy of the boot sector.
pub(crate) const BACKUP_BOOT_SECTOR: u16 = 6;

/// Serial number of the volume; fixed, so that images are reproducible.
pub(crate) const VOLUME_ID: u32 = 0xAD48_5305;

/// Label of the volume, exactly eleven bytes.
pub(crate) const VOLUME_LABEL: &[u8; 11] = b"AUDHSOS    ";

/// `2026-01-01` in the FAT date encoding.
pub(crate) const DATE: u16 = ((2026 - 1980) << 9) | (1 << 5) | 1;

/// Midnight in the FAT time encoding.
pub(crate) const TIME: u16 = 0;

/// Attribute of a directory.
pub(crate) const ATTR_DIRECTORY: u8 = 0x10;

/// Attribute of a file.
pub(crate) const ATTR_ARCHIVE: u8 = 0x20;

/// Length of one directory entry in bytes.
pub(crate) const ENTRY_LEN: usize = 32;

/// The geometry of one file system, derived from the size of the
/// partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Geometry {
    /// Total sectors of the partition.
    pub(crate) sectors: u32,
    /// Sectors per file allocation table.
    pub(crate) fat_sectors: u32,
    /// Number of clusters in the data region.
    pub(crate) clusters: u32,
}

impl Geometry {
    /// First sector of the data region.
    pub(crate) const fn data_start(self) -> u32 {
        RESERVED_SECTORS + FAT_COUNT * self.fat_sectors
    }

    /// First sector of a cluster.
    pub(crate) const fn cluster_sector(self, cluster: u32) -> u32 {
        self.data_start() + (cluster - ROOT_CLUSTER) * SECTORS_PER_CLUSTER
    }
}

/// The geometry of a partition of `sectors` sectors.
///
/// # Errors
///
/// [`Error::Usage`] if the partition is too small for a FAT32 file system.
pub(crate) fn geometry(sectors: u32) -> Result<Geometry, Error> {
    let available = sectors
        .checked_sub(RESERVED_SECTORS)
        .ok_or_else(|| too_small(sectors))?;
    // Each table entry is four bytes; the table has to describe itself, so
    // the size is found by trying the smallest one that still fits.
    let mut fat_sectors = 1u32;
    loop {
        let data = available
            .checked_sub(FAT_COUNT * fat_sectors)
            .ok_or_else(|| too_small(sectors))?;
        let clusters = data / SECTORS_PER_CLUSTER;
        let needed = ((clusters + 2) * 4).div_ceil(u32::try_from(SECTOR).unwrap_or(512));
        if needed <= fat_sectors {
            if clusters < MIN_CLUSTERS {
                return Err(Error::Usage(format!(
                    "a partition of {sectors} sectors holds {clusters} clusters, \
                     fewer than the {MIN_CLUSTERS} a FAT32 file system needs"
                )));
            }
            return Ok(Geometry {
                sectors,
                fat_sectors,
                clusters,
            });
        }
        fat_sectors = needed;
    }
}

fn too_small(sectors: u32) -> Error {
    Error::Usage(format!(
        "a partition of {sectors} sectors is too small for a FAT32 file system"
    ))
}

/// A name in the 8.3 form the image uses, as the eleven bytes a directory
/// entry carries.
///
/// # Errors
///
/// [`Error::Usage`] for an empty name, a name or extension that is too
/// long, or a character the short form does not allow.
pub(crate) fn short_name(name: &str) -> Result<[u8; 11], Error> {
    let (stem, extension) = match name.split_once('.') {
        Some((stem, extension)) => (stem, extension),
        None => (name, ""),
    };
    if stem.is_empty()
        || stem.len() > 8
        || extension.len() > 3
        || name.contains('.') && extension.is_empty()
    {
        return Err(Error::Usage(format!("`{name}` is not an 8.3 name")));
    }
    let mut bytes = [b' '; 11];
    for (slot, byte) in bytes.iter_mut().zip(stem.bytes()) {
        *slot = short_byte(byte, name)?;
    }
    for (slot, byte) in bytes.iter_mut().skip(8).zip(extension.bytes()) {
        *slot = short_byte(byte, name)?;
    }
    Ok(bytes)
}

fn short_byte(byte: u8, name: &str) -> Result<u8, Error> {
    let upper = byte.to_ascii_uppercase();
    let allowed = upper.is_ascii_uppercase()
        || upper.is_ascii_digit()
        || b"$%'-_@~`!(){}^#&".contains(&upper);
    if allowed {
        Ok(upper)
    } else {
        Err(Error::Usage(format!(
            "`{name}` holds a character an 8.3 name does not allow"
        )))
    }
}

/// One node of the directory tree the image holds.
#[derive(Clone, Debug)]
struct Node {
    name: [u8; 11],
    directory: bool,
    children: Vec<usize>,
    data: Vec<u8>,
    first_cluster: u32,
}

impl Node {
    const fn entry_count(&self, root: bool) -> usize {
        if root {
            self.children.len()
        } else {
            self.children.len().saturating_add(2)
        }
    }
}

/// Writes the file system into `partition` with the given files, whose
/// paths are `/`-separated 8.3 names.
///
/// # Errors
///
/// [`Error::Usage`] for a partition that is too small, a name that is not
/// an 8.3 name, or files that do not fit into the data region.
pub(crate) fn write(partition: &mut [u8], files: &[(&str, Vec<u8>)]) -> Result<Geometry, Error> {
    let sectors = u32::try_from(partition.len() / SECTOR).unwrap_or(u32::MAX);
    let geometry = geometry(sectors)?;
    let mut nodes = build_tree(files)?;
    let used = assign_clusters(&mut nodes, &geometry)?;
    write_boot_sectors(partition, &geometry, used);
    write_tables(partition, &geometry, &nodes);
    write_contents(partition, &geometry, &nodes);
    Ok(geometry)
}

/// Builds the tree; node zero is the root.
fn build_tree(files: &[(&str, Vec<u8>)]) -> Result<Vec<Node>, Error> {
    let mut nodes = vec![Node {
        name: [b' '; 11],
        directory: true,
        children: Vec::new(),
        data: Vec::new(),
        first_cluster: 0,
    }];
    for (path, data) in files {
        let mut parent = 0usize;
        let mut components = path.split('/').peekable();
        while let Some(component) = components.next() {
            let name = short_name(component)?;
            let last = components.peek().is_none();
            let existing = nodes
                .get(parent)
                .map(|node| node.children.clone())
                .unwrap_or_default()
                .into_iter()
                .find(|index| nodes.get(*index).is_some_and(|node| node.name == name));
            match existing {
                Some(index) if !last => parent = index,
                Some(_) => {
                    return Err(Error::Usage(format!("`{path}` is in the image twice")));
                }
                None => {
                    nodes.push(Node {
                        name,
                        directory: !last,
                        children: Vec::new(),
                        data: if last { data.clone() } else { Vec::new() },
                        first_cluster: 0,
                    });
                    let index = nodes.len() - 1;
                    if let Some(node) = nodes.get_mut(parent) {
                        node.children.push(index);
                    }
                    parent = index;
                }
            }
        }
    }
    Ok(nodes)
}

/// Gives every node a chain of clusters and reports how many are used.
fn assign_clusters(nodes: &mut [Node], geometry: &Geometry) -> Result<u32, Error> {
    let mut next = ROOT_CLUSTER;
    for index in 0..nodes.len() {
        let root = index == 0;
        let Some(node) = nodes.get(index) else { break };
        let bytes = if node.directory {
            node.entry_count(root) * ENTRY_LEN
        } else {
            node.data.len()
        };
        let clusters = u32::try_from(bytes.div_ceil(SECTOR))
            .unwrap_or(u32::MAX)
            .max(1);
        if next.saturating_sub(ROOT_CLUSTER).saturating_add(clusters) > geometry.clusters {
            return Err(Error::Usage(
                "the files do not fit into the partition".to_owned(),
            ));
        }
        if let Some(node) = nodes.get_mut(index) {
            node.first_cluster = next;
        }
        next += clusters;
    }
    Ok(next - ROOT_CLUSTER)
}

fn cluster_count(node: &Node, root: bool) -> u32 {
    let bytes = if node.directory {
        node.entry_count(root) * ENTRY_LEN
    } else {
        node.data.len()
    };
    u32::try_from(bytes.div_ceil(SECTOR))
        .unwrap_or(u32::MAX)
        .max(1)
}

fn put(image: &mut [u8], offset: usize, bytes: &[u8]) {
    if let Some(slot) = offset
        .checked_add(bytes.len())
        .and_then(|end| image.get_mut(offset..end))
    {
        slot.copy_from_slice(bytes);
    }
}

fn sector_offset(sector: u32) -> usize {
    usize::try_from(sector)
        .unwrap_or(usize::MAX)
        .saturating_mul(SECTOR)
}

/// The boot sector, the file system information sector, and the copy of
/// the boot sector.
fn write_boot_sectors(partition: &mut [u8], geometry: &Geometry, used: u32) {
    let mut boot = [0u8; SECTOR];
    put(&mut boot, 0, &[0xEB, 0x58, 0x90]);
    put(&mut boot, 3, b"MSWIN4.1");
    put(
        &mut boot,
        11,
        &u16::try_from(SECTOR).unwrap_or(512).to_le_bytes(),
    );
    put(
        &mut boot,
        13,
        &[u8::try_from(SECTORS_PER_CLUSTER).unwrap_or(1)],
    );
    put(
        &mut boot,
        14,
        &u16::try_from(RESERVED_SECTORS).unwrap_or(32).to_le_bytes(),
    );
    put(&mut boot, 16, &[u8::try_from(FAT_COUNT).unwrap_or(2)]);
    put(&mut boot, 21, &[MEDIA]);
    put(&mut boot, 24, &32u16.to_le_bytes());
    put(&mut boot, 26, &8u16.to_le_bytes());
    put(&mut boot, 32, &geometry.sectors.to_le_bytes());
    put(&mut boot, 36, &geometry.fat_sectors.to_le_bytes());
    put(&mut boot, 44, &ROOT_CLUSTER.to_le_bytes());
    put(&mut boot, 48, &FSINFO_SECTOR.to_le_bytes());
    put(&mut boot, 50, &BACKUP_BOOT_SECTOR.to_le_bytes());
    put(&mut boot, 64, &[0x80]);
    put(&mut boot, 66, &[0x29]);
    put(&mut boot, 67, &VOLUME_ID.to_le_bytes());
    put(&mut boot, 71, VOLUME_LABEL);
    put(&mut boot, 82, b"FAT32   ");
    put(&mut boot, 510, &[0x55, 0xAA]);
    put(partition, 0, &boot);
    put(
        partition,
        sector_offset(u32::from(BACKUP_BOOT_SECTOR)),
        &boot,
    );

    let mut info = [0u8; SECTOR];
    put(&mut info, 0, &0x4161_5252u32.to_le_bytes());
    put(&mut info, 484, &0x6141_7272u32.to_le_bytes());
    put(
        &mut info,
        488,
        &geometry.clusters.saturating_sub(used).to_le_bytes(),
    );
    put(&mut info, 492, &(ROOT_CLUSTER + used).to_le_bytes());
    put(&mut info, 508, &0xAA55_0000u32.to_le_bytes());
    put(partition, sector_offset(u32::from(FSINFO_SECTOR)), &info);
}

/// Both copies of the file allocation table.
fn write_tables(partition: &mut [u8], geometry: &Geometry, nodes: &[Node]) {
    let mut table = vec![0u8; sector_offset(geometry.fat_sectors)];
    put(
        &mut table,
        0,
        &(0x0FFF_FF00 | u32::from(MEDIA)).to_le_bytes(),
    );
    put(&mut table, 4, &END_OF_CHAIN.to_le_bytes());
    for (index, node) in nodes.iter().enumerate() {
        let count = cluster_count(node, index == 0);
        for step in 0..count {
            let cluster = node.first_cluster + step;
            let next = if step + 1 == count {
                END_OF_CHAIN
            } else {
                cluster + 1
            };
            put(
                &mut table,
                usize::try_from(cluster).unwrap_or(0) * 4,
                &next.to_le_bytes(),
            );
        }
    }
    for copy in 0..FAT_COUNT {
        let start = RESERVED_SECTORS + copy * geometry.fat_sectors;
        put(partition, sector_offset(start), &table);
    }
}

/// The directory entries and the file contents.
fn write_contents(partition: &mut [u8], geometry: &Geometry, nodes: &[Node]) {
    for (index, node) in nodes.iter().enumerate() {
        let offset = sector_offset(geometry.cluster_sector(node.first_cluster));
        if node.directory {
            put(partition, offset, &directory_bytes(nodes, node, index == 0));
        } else {
            put(partition, offset, &node.data);
        }
    }
}

/// The entries of one directory, `.` and `..` first for anything but the
/// root.
fn directory_bytes(nodes: &[Node], node: &Node, root: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    if !root {
        bytes.extend_from_slice(&entry(
            b".          ",
            ATTR_DIRECTORY,
            node.first_cluster,
            0,
        ));
        bytes.extend_from_slice(&entry(b"..         ", ATTR_DIRECTORY, 0, 0));
    }
    for index in &node.children {
        let Some(child) = nodes.get(*index) else {
            continue;
        };
        let (attributes, size) = if child.directory {
            (ATTR_DIRECTORY, 0)
        } else {
            (
                ATTR_ARCHIVE,
                u32::try_from(child.data.len()).unwrap_or(u32::MAX),
            )
        };
        bytes.extend_from_slice(&entry(&child.name, attributes, child.first_cluster, size));
    }
    bytes
}

/// One directory entry.
fn entry(name: &[u8; 11], attributes: u8, cluster: u32, size: u32) -> [u8; ENTRY_LEN] {
    let mut bytes = [0u8; ENTRY_LEN];
    put(&mut bytes, 0, name);
    put(&mut bytes, 11, &[attributes]);
    put(&mut bytes, 14, &TIME.to_le_bytes());
    put(&mut bytes, 16, &DATE.to_le_bytes());
    put(&mut bytes, 18, &DATE.to_le_bytes());
    put(
        &mut bytes,
        20,
        &u16::try_from(cluster >> 16).unwrap_or(0).to_le_bytes(),
    );
    put(&mut bytes, 22, &TIME.to_le_bytes());
    put(&mut bytes, 24, &DATE.to_le_bytes());
    put(
        &mut bytes,
        26,
        &u16::try_from(cluster & 0xFFFF).unwrap_or(0).to_le_bytes(),
    );
    put(&mut bytes, 28, &size.to_le_bytes());
    bytes
}

/// Reads the file system back, so that the tests compare what was written
/// with what a reader finds. Returns the files by their `/`-separated
/// paths. Only the tests read; the product only writes.
///
/// # Errors
///
/// [`Error::Parse`] if the boot sector does not describe a file system
/// this writer produced.
#[cfg(test)]
pub(crate) fn read(partition: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, Error> {
    let boot = partition
        .get(..SECTOR)
        .ok_or_else(|| Error::Parse("the partition holds no boot sector".to_owned()))?;
    if boot.get(510..512) != Some(&[0x55, 0xAA]) {
        return Err(Error::Parse("the boot sector has no signature".to_owned()));
    }
    let fat_sectors = read_u32(boot, 36);
    let geometry = Geometry {
        sectors: read_u32(boot, 32),
        fat_sectors,
        clusters: 0,
    };
    let mut files = BTreeMap::new();
    read_directory(partition, &geometry, ROOT_CLUSTER, "", &mut files)?;
    Ok(files)
}

#[cfg(test)]
fn read_directory(
    partition: &[u8],
    geometry: &Geometry,
    cluster: u32,
    prefix: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), Error> {
    let bytes = read_chain(partition, geometry, cluster);
    for chunk in bytes.chunks(ENTRY_LEN) {
        let Some(name) = chunk.get(..11) else {
            continue;
        };
        if name.first() == Some(&0) || name.first() == Some(&b'.') {
            continue;
        }
        let attributes = chunk.get(11).copied().unwrap_or(0);
        let first = u32::from(read_u16(chunk, 20)) << 16 | u32::from(read_u16(chunk, 26));
        let long_name = display_name(name);
        let path = if prefix.is_empty() {
            long_name
        } else {
            format!("{prefix}/{long_name}")
        };
        if attributes & ATTR_DIRECTORY != 0 {
            read_directory(partition, geometry, first, &path, files)?;
        } else {
            let size = usize::try_from(read_u32(chunk, 28)).unwrap_or(0);
            let mut data = read_chain(partition, geometry, first);
            data.truncate(size);
            files.insert(path, data);
        }
    }
    Ok(())
}

/// The bytes of a cluster chain, following the first file allocation
/// table.
#[cfg(test)]
fn read_chain(partition: &[u8], geometry: &Geometry, first: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut cluster = first;
    let mut steps = 0u32;
    while (ROOT_CLUSTER..END_OF_CHAIN).contains(&cluster) && steps < 1 << 20 {
        let offset = sector_offset(geometry.cluster_sector(cluster));
        if let Some(slot) = partition.get(offset..offset.saturating_add(SECTOR)) {
            bytes.extend_from_slice(slot);
        }
        let entry_offset =
            sector_offset(RESERVED_SECTORS) + usize::try_from(cluster).unwrap_or(0) * 4;
        cluster = read_u32(partition, entry_offset) & 0x0FFF_FFFF;
        steps += 1;
    }
    bytes
}

/// The 8.3 name of an entry as `NAME.EXT`.
#[cfg(test)]
fn display_name(name: &[u8]) -> String {
    let stem: String = name
        .get(..8)
        .unwrap_or_default()
        .iter()
        .map(|byte| char::from(*byte))
        .collect();
    let extension: String = name
        .get(8..11)
        .unwrap_or_default()
        .iter()
        .map(|byte| char::from(*byte))
        .collect();
    let stem = stem.trim_end().to_owned();
    let extension = extension.trim_end();
    if extension.is_empty() {
        stem
    } else {
        format!("{stem}.{extension}")
    }
}

#[cfg(test)]
fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    bytes
        .get(offset..offset.saturating_add(2))
        .and_then(|slice| slice.try_into().ok())
        .map_or(0, u16::from_le_bytes)
}

#[cfg(test)]
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    bytes
        .get(offset..offset.saturating_add(4))
        .and_then(|slice| slice.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}
