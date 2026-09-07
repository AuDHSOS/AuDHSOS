// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The FAT32 file system inside the EFI system partition.
//!
//! The structure is `fs-fat`'s (D-53); what is here is the image's own:
//! the partition as a block device, the choices a boot volume is made
//! with, and the fixed moment every entry is stamped with, so that two
//! runs with the same files produce the same bytes.

#[cfg(test)]
use std::collections::BTreeMap;

use audhsos_time::UnixTime;
use fs_fat::{BlockDevice, FileSystem, FormatOptions, Name};

use crate::error::Error;

/// Size of one sector in bytes.
pub(crate) const SECTOR: usize = fs_fat::SECTOR;

/// Sectors per cluster.
pub(crate) const SECTORS_PER_CLUSTER: u32 = 1;

/// Sectors before the first file allocation table.
pub(crate) const RESERVED_SECTORS: u32 = fs_fat::DEFAULT_RESERVED_SECTORS;

/// Number of file allocation tables.
pub(crate) const FAT_COUNT: u32 = fs_fat::DEFAULT_FAT_COUNT;

#[cfg(test)]
/// Cluster of the root directory.
pub(crate) const ROOT_CLUSTER: u32 = fs_fat::ROOT_CLUSTER;

/// Media descriptor of a fixed disk.
pub(crate) const MEDIA: u8 = fs_fat::MEDIA;

#[cfg(test)]
/// The value that ends a cluster chain.
pub(crate) const END_OF_CHAIN: u32 = fs_fat::END_OF_CHAIN;

/// Fewer clusters than this would make the file system FAT16.
pub(crate) const MIN_CLUSTERS: u32 = fs_fat::MIN_CLUSTERS;

#[cfg(test)]
/// Sector of the file system information structure.
pub(crate) const FSINFO_SECTOR: u16 = 1;

#[cfg(test)]
/// Sector of the copy of the boot sector.
pub(crate) const BACKUP_BOOT_SECTOR: u16 = 6;

/// Serial number of the volume; fixed, so that images are reproducible.
pub(crate) const VOLUME_ID: u32 = 0xAD48_5305;

/// Label of the volume, exactly eleven bytes.
pub(crate) const VOLUME_LABEL: &[u8; 11] = b"AUDHSOS    ";

/// `2026-01-01T00:00:00Z`, the moment every entry of an image carries.
pub(crate) const TIMESTAMP: UnixTime = UnixTime::from_seconds(1_767_225_600);

#[cfg(test)]
/// `2026-01-01` in the FAT date encoding, which is what [`TIMESTAMP`]
/// comes out as.
pub(crate) const DATE: u16 = ((2026 - 1980) << 9) | (1 << 5) | 1;

#[cfg(test)]
/// Midnight in the FAT time encoding.
pub(crate) const TIME: u16 = 0;

#[cfg(test)]
/// Attribute of a directory.
pub(crate) const ATTR_DIRECTORY: u8 = fs_fat::ATTR_DIRECTORY;

#[cfg(test)]
/// Length of one directory entry in bytes.
pub(crate) const ENTRY_LEN: usize = fs_fat::ENTRY_LEN;

/// The geometry of one file system, derived from the size of the
/// partition.
pub(crate) type Geometry = fs_fat::Geometry;

/// What a boot volume of this project is made of.
const fn options() -> FormatOptions {
    FormatOptions {
        sectors_per_cluster: SECTORS_PER_CLUSTER,
        reserved_sectors: RESERVED_SECTORS,
        fat_count: FAT_COUNT,
        media: MEDIA,
        volume_id: VOLUME_ID,
        volume_label: *VOLUME_LABEL,
    }
}

/// The partition as a device of sectors.
struct Partition<'a> {
    /// The bytes of the partition, one sector after another.
    bytes: &'a mut [u8],
}

/// The partition of an image that is only read.
#[cfg(test)]
struct View<'a> {
    /// The bytes of the partition.
    bytes: &'a [u8],
}

/// Where a sector begins, where the bytes hold it.
fn offset(len: usize, sector: u32) -> Option<usize> {
    let start = usize::try_from(sector).ok()?.checked_mul(SECTOR)?;
    if start.checked_add(SECTOR)? <= len {
        Some(start)
    } else {
        None
    }
}

impl BlockDevice for Partition<'_> {
    fn sectors(&self) -> u32 {
        u32::try_from(self.bytes.len() / SECTOR).unwrap_or(u32::MAX)
    }

    fn read(&self, sector: u32, into: &mut [u8; SECTOR]) -> Result<(), fs_fat::Error> {
        let start = offset(self.bytes.len(), sector).ok_or(fs_fat::Error::Device(sector))?;
        let source = self
            .bytes
            .get(start..start.saturating_add(SECTOR))
            .ok_or(fs_fat::Error::Device(sector))?;
        into.copy_from_slice(source);
        Ok(())
    }

    fn write(&mut self, sector: u32, from: &[u8; SECTOR]) -> Result<(), fs_fat::Error> {
        let start = offset(self.bytes.len(), sector).ok_or(fs_fat::Error::Device(sector))?;
        let target = self
            .bytes
            .get_mut(start..start.saturating_add(SECTOR))
            .ok_or(fs_fat::Error::Device(sector))?;
        target.copy_from_slice(from);
        Ok(())
    }
}

#[cfg(test)]
impl BlockDevice for View<'_> {
    fn sectors(&self) -> u32 {
        u32::try_from(self.bytes.len() / SECTOR).unwrap_or(u32::MAX)
    }

    fn read(&self, sector: u32, into: &mut [u8; SECTOR]) -> Result<(), fs_fat::Error> {
        let start = offset(self.bytes.len(), sector).ok_or(fs_fat::Error::Device(sector))?;
        let source = self
            .bytes
            .get(start..start.saturating_add(SECTOR))
            .ok_or(fs_fat::Error::Device(sector))?;
        into.copy_from_slice(source);
        Ok(())
    }

    fn write(&mut self, sector: u32, _from: &[u8; SECTOR]) -> Result<(), fs_fat::Error> {
        Err(fs_fat::Error::Device(sector))
    }
}

/// The geometry of a partition of `sectors` sectors.
///
/// # Errors
///
/// [`Error::Usage`] if the partition is too small for a FAT32 file system.
pub(crate) fn geometry(sectors: u32) -> Result<Geometry, Error> {
    fs_fat::geometry_for(sectors, &options()).map_err(|error| match error {
        fs_fat::Error::NotFat32(clusters) => Error::Usage(format!(
            "a partition of {sectors} sectors holds {clusters} clusters, \
             fewer than the {MIN_CLUSTERS} a FAT32 file system needs"
        )),
        _ => Error::Usage(format!(
            "a partition of {sectors} sectors is too small for a FAT32 file system"
        )),
    })
}

/// A name in the 8.3 form the image uses, as the eleven bytes a directory
/// entry carries.
///
/// # Errors
///
/// [`Error::Usage`] for an empty name, a name or extension that is too
/// long, or a character the short form does not allow.
#[cfg(test)]
pub(crate) fn short_name(name: &str) -> Result<[u8; 11], Error> {
    Ok(*name_of(name)?.as_bytes())
}

/// The name `name` stands for.
fn name_of(name: &str) -> Result<Name, Error> {
    Name::new(name).map_err(|_| Error::Usage(format!("`{name}` is not an 8.3 name")))
}

/// Writes the file system into `partition` with the given files, whose
/// paths are `/`-separated 8.3 names.
///
/// # Errors
///
/// [`Error::Usage`] for a partition that is too small, a name that is not
/// an 8.3 name, a path that is in the image twice, or files that do not
/// fit into the data region.
pub(crate) fn write(partition: &mut [u8], files: &[(&str, Vec<u8>)]) -> Result<Geometry, Error> {
    let sectors = u32::try_from(partition.len() / SECTOR).unwrap_or(u32::MAX);
    let geometry = geometry(sectors)?;
    let mut volume = FileSystem::format(Partition { bytes: partition }, &options())
        .map_err(|error| Error::Usage(format!("the file system was refused: {error}")))?;
    for (path, data) in files {
        let mut directory = volume.root();
        let mut components = path.split('/').peekable();
        while let Some(component) = components.next() {
            let name = name_of(component)?;
            if components.peek().is_some() {
                directory = volume
                    .open_or_create_dir(directory, &name, TIMESTAMP)
                    .map_err(|error| no_room(path, error))?;
                continue;
            }
            let mut file =
                volume
                    .create(directory, &name, TIMESTAMP)
                    .map_err(|error| match error {
                        fs_fat::Error::Exists => {
                            Error::Usage(format!("`{path}` is in the image twice"))
                        }
                        other => no_room(path, other),
                    })?;
            volume
                .write(&mut file, 0, data)
                .map_err(|error| no_room(path, error))?;
        }
    }
    volume
        .flush()
        .map_err(|error| Error::Usage(format!("the file system was refused: {error}")))?;
    Ok(geometry)
}

/// What a refusal while writing `path` means to a caller of the xtask.
fn no_room(path: &str, error: fs_fat::Error) -> Error {
    match error {
        fs_fat::Error::Full => Error::Usage("the files do not fit into the partition".to_owned()),
        fs_fat::Error::Kind => Error::Usage(format!("`{path}` is in the image twice")),
        other => Error::Usage(format!("`{path}` was refused: {other}")),
    }
}

/// Reads the file system back, so that the tests compare what was written
/// with what a reader finds. Returns the files by their `/`-separated
/// paths. Only the tests read; the product only writes.
///
/// # Errors
///
/// [`Error::Parse`] if the partition does not hold a file system this
/// writer produced.
#[cfg(test)]
pub(crate) fn read(partition: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, Error> {
    let volume = FileSystem::mount(View { bytes: partition })
        .map_err(|error| Error::Parse(format!("the partition holds no file system: {error}")))?;
    let mut files = BTreeMap::new();
    let root = volume.root();
    collect(&volume, root, "", &mut files)?;
    Ok(files)
}

/// Puts every file of `directory` and of the directories below it into
/// `files`.
#[cfg(test)]
fn collect(
    volume: &FileSystem<View<'_>>,
    directory: fs_fat::Dir,
    prefix: &str,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(), Error> {
    let mut cursor = volume.entries(directory);
    while let Some(entry) = volume
        .next_entry(&mut cursor)
        .map_err(|error| Error::Parse(format!("a directory could not be read: {error}")))?
    {
        let path = if prefix.is_empty() {
            format!("{}", entry.name)
        } else {
            format!("{prefix}/{}", entry.name)
        };
        if entry.is_directory() {
            collect(volume, fs_fat::Dir::at(entry.first_cluster), &path, files)?;
            continue;
        }
        let mut file = volume
            .open(directory, &entry.name)
            .map_err(|error| Error::Parse(format!("`{path}` could not be opened: {error}")))?;
        let mut bytes = vec![0u8; usize::try_from(entry.size).unwrap_or(0)];
        volume
            .read(&mut file, 0, &mut bytes)
            .map_err(|error| Error::Parse(format!("`{path}` could not be read: {error}")))?;
        files.insert(path, bytes);
    }
    Ok(())
}
