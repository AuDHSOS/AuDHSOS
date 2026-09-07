// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The boot sector: the parameter block as numbers, read from a volume
//! and written onto one.

use crate::device::{BlockDevice, SECTOR, put, read_u16, read_u32};
use crate::error::Error;

/// The first cluster of the data region, and the one a fresh volume gives
/// its root directory.
pub const ROOT_CLUSTER: u32 = 2;

/// Fewer data clusters than this and the volume is a FAT16 one. The
/// number is what draws the line between the two, and it is why a small
/// device cannot hold a FAT32 file system at all.
pub const MIN_CLUSTERS: u32 = 65525;

/// The largest cluster this crate reads, in sectors. A cluster of more
/// than 128 sectors is above the 64 kibibytes the format settled on.
pub const MAX_SECTORS_PER_CLUSTER: u32 = 128;

/// Sector of the file system information structure on a volume this crate
/// writes.
pub const FSINFO_SECTOR: u32 = 1;

/// Sector of the copy of the boot sector on a volume this crate writes.
pub const BACKUP_BOOT_SECTOR: u32 = 6;

/// Media descriptor of a fixed disk.
pub const MEDIA: u8 = 0xF8;

/// Sectors a fresh volume keeps before the first table.
pub const DEFAULT_RESERVED_SECTORS: u32 = 32;

/// Tables a fresh volume carries. Two is what every FAT volume has.
pub const DEFAULT_FAT_COUNT: u32 = 2;

/// The first signature of the file system information sector.
const FSINFO_LEAD: u32 = 0x4161_5252;

/// The second signature of the file system information sector.
const FSINFO_STRUCT: u32 = 0x6141_7272;

/// The trailing signature of the file system information sector.
const FSINFO_TRAIL: u32 = 0xAA55_0000;

/// What a fresh volume is made of. Everything here is a choice rather
/// than a rule of the format, which is why it is the caller's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FormatOptions {
    /// Sectors in one cluster; a power of two up to
    /// [`MAX_SECTORS_PER_CLUSTER`].
    pub sectors_per_cluster: u32,
    /// Sectors before the first table.
    pub reserved_sectors: u32,
    /// How many copies of the table the volume carries.
    pub fat_count: u32,
    /// The media descriptor, which is also the low byte of the first
    /// table entry.
    pub media: u8,
    /// The serial number of the volume.
    pub volume_id: u32,
    /// The label, exactly eleven bytes, space-padded.
    pub volume_label: [u8; 11],
}

impl Default for FormatOptions {
    /// One sector per cluster, the two tables and the thirty-two reserved
    /// sectors every FAT32 volume has, and an unnamed volume.
    fn default() -> FormatOptions {
        FormatOptions {
            sectors_per_cluster: 1,
            reserved_sectors: DEFAULT_RESERVED_SECTORS,
            fat_count: DEFAULT_FAT_COUNT,
            media: MEDIA,
            volume_id: 0,
            volume_label: *b"NO NAME    ",
        }
    }
}

/// Where everything is on one volume: the parameter block, as numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    /// Sectors the volume claims.
    pub sectors: u32,
    /// Sectors in one cluster.
    pub sectors_per_cluster: u32,
    /// Sectors before the first table.
    pub reserved_sectors: u32,
    /// How many copies of the table there are.
    pub fat_count: u32,
    /// Sectors in one table.
    pub fat_sectors: u32,
    /// Clusters in the data region.
    pub clusters: u32,
    /// The cluster the root directory begins at.
    pub root_cluster: u32,
}

impl Geometry {
    /// The first sector of the data region.
    #[must_use]
    pub const fn data_start(&self) -> u32 {
        self.reserved_sectors
            .saturating_add(self.fat_count.saturating_mul(self.fat_sectors))
    }

    /// The first sector of `cluster`. A cluster below [`ROOT_CLUSTER`]
    /// answers the start of the data region, which is where cluster two
    /// is; callers check the number against [`Geometry::holds`] first.
    #[must_use]
    pub const fn cluster_sector(&self, cluster: u32) -> u32 {
        self.data_start().saturating_add(
            cluster
                .saturating_sub(ROOT_CLUSTER)
                .saturating_mul(self.sectors_per_cluster),
        )
    }

    /// The bytes one cluster holds.
    #[must_use]
    pub const fn cluster_bytes(&self) -> u32 {
        self.sectors_per_cluster.saturating_mul(SECTOR_U32)
    }

    /// The highest cluster number the data region has.
    #[must_use]
    pub const fn last_cluster(&self) -> u32 {
        self.clusters.saturating_add(ROOT_CLUSTER).saturating_sub(1)
    }

    /// Whether `cluster` names a cluster of the data region.
    #[must_use]
    pub const fn holds(&self, cluster: u32) -> bool {
        cluster >= ROOT_CLUSTER && cluster <= self.last_cluster()
    }

    /// The first sector of the table copy `copy`.
    #[must_use]
    pub const fn fat_start(&self, copy: u32) -> u32 {
        self.reserved_sectors
            .saturating_add(copy.saturating_mul(self.fat_sectors))
    }
}

/// [`SECTOR`] as the number the geometry counts in.
pub(crate) const SECTOR_U32: u32 = 512;

/// The geometry a device of `sectors` sectors gets under `options`.
///
/// The size of the table is found rather than computed, because a table
/// has to describe the clusters that are left once it is there: the
/// smallest size that describes what remains beside it is the answer, and
/// each try that is too small names a larger one.
///
/// # Errors
///
/// [`Error::ClusterSize`] for a cluster size that is not a power of two,
/// [`Error::Layout`] for no table at all, [`Error::TooSmall`] for a device
/// that cannot hold the tables, and [`Error::NotFat32`] for one that holds
/// fewer than [`MIN_CLUSTERS`] clusters.
pub fn geometry_for(sectors: u32, options: &FormatOptions) -> Result<Geometry, Error> {
    check_cluster_size(options.sectors_per_cluster)?;
    if options.fat_count == 0 || options.reserved_sectors == 0 {
        return Err(Error::Layout);
    }
    let available = sectors
        .checked_sub(options.reserved_sectors)
        .ok_or(Error::TooSmall(sectors))?;
    let mut fat_sectors = 1u32;
    loop {
        let data = available
            .checked_sub(options.fat_count.saturating_mul(fat_sectors))
            .ok_or(Error::TooSmall(sectors))?;
        let clusters = data.checked_div(options.sectors_per_cluster).unwrap_or(0);
        let needed = clusters
            .saturating_add(ROOT_CLUSTER)
            .saturating_mul(4)
            .div_ceil(SECTOR_U32);
        if needed <= fat_sectors {
            if clusters < MIN_CLUSTERS {
                return Err(Error::NotFat32(clusters));
            }
            return Ok(Geometry {
                sectors,
                sectors_per_cluster: options.sectors_per_cluster,
                reserved_sectors: options.reserved_sectors,
                fat_count: options.fat_count,
                fat_sectors,
                clusters,
                root_cluster: ROOT_CLUSTER,
            });
        }
        fat_sectors = needed;
    }
}

/// The geometry the boot sector of a mounted volume describes.
///
/// # Errors
///
/// [`Error::Signature`], [`Error::SectorSize`], [`Error::ClusterSize`],
/// [`Error::Layout`], [`Error::NotFat32`] for a FAT12 or FAT16 volume, and
/// [`Error::Cluster`] for a root directory outside the data region.
pub fn parse(boot: &[u8; SECTOR]) -> Result<Geometry, Error> {
    if boot.get(510..512) != Some(&[0x55, 0xAA]) {
        return Err(Error::Signature);
    }
    let bytes_per_sector = read_u16(boot, 11);
    if usize::from(bytes_per_sector) != SECTOR {
        return Err(Error::SectorSize(bytes_per_sector));
    }
    let sectors_per_cluster = u32::from(*boot.get(13).unwrap_or(&0));
    check_cluster_size(sectors_per_cluster)?;
    let reserved_sectors = u32::from(read_u16(boot, 14));
    let fat_count = u32::from(*boot.get(16).unwrap_or(&0));
    if reserved_sectors == 0 || fat_count == 0 {
        return Err(Error::Layout);
    }
    // Three fields say FAT12 or FAT16 by being used at all: the count of
    // the fixed root directory, the sixteen-bit table size, and the
    // sixteen-bit total. FAT32 leaves all three at zero.
    if read_u16(boot, 17) != 0 || read_u16(boot, 22) != 0 || read_u16(boot, 19) != 0 {
        return Err(Error::NotFat32(0));
    }
    let sectors = read_u32(boot, 32);
    let fat_sectors = read_u32(boot, 36);
    if fat_sectors == 0 {
        return Err(Error::Layout);
    }
    let root_cluster = read_u32(boot, 44);
    let overhead = reserved_sectors.saturating_add(fat_count.saturating_mul(fat_sectors));
    let clusters = sectors
        .saturating_sub(overhead)
        .checked_div(sectors_per_cluster)
        .unwrap_or(0);
    if clusters < MIN_CLUSTERS {
        return Err(Error::NotFat32(clusters));
    }
    let geometry = Geometry {
        sectors,
        sectors_per_cluster,
        reserved_sectors,
        fat_count,
        fat_sectors,
        clusters,
        root_cluster,
    };
    if !geometry.holds(root_cluster) {
        return Err(Error::Cluster(root_cluster));
    }
    Ok(geometry)
}

/// A cluster size the format allows: a power of two, at least one sector
/// and at most [`MAX_SECTORS_PER_CLUSTER`].
const fn check_cluster_size(sectors_per_cluster: u32) -> Result<(), Error> {
    if sectors_per_cluster == 0
        || !sectors_per_cluster.is_power_of_two()
        || sectors_per_cluster > MAX_SECTORS_PER_CLUSTER
    {
        return Err(Error::ClusterSize(sectors_per_cluster));
    }
    Ok(())
}

/// The boot sector `geometry` and `options` describe.
pub(crate) fn boot_sector(geometry: &Geometry, options: &FormatOptions) -> [u8; SECTOR] {
    let mut boot = [0u8; SECTOR];
    put(&mut boot, 0, &[0xEB, 0x58, 0x90]);
    put(&mut boot, 3, b"MSWIN4.1");
    put(&mut boot, 11, &SECTOR_U16.to_le_bytes());
    put(
        &mut boot,
        13,
        &[u8::try_from(geometry.sectors_per_cluster).unwrap_or(1)],
    );
    put(
        &mut boot,
        14,
        &u16::try_from(geometry.reserved_sectors)
            .unwrap_or(u16::MAX)
            .to_le_bytes(),
    );
    put(
        &mut boot,
        16,
        &[u8::try_from(geometry.fat_count).unwrap_or(2)],
    );
    put(&mut boot, 21, &[options.media]);
    put(&mut boot, 24, &32u16.to_le_bytes());
    put(&mut boot, 26, &8u16.to_le_bytes());
    put(&mut boot, 32, &geometry.sectors.to_le_bytes());
    put(&mut boot, 36, &geometry.fat_sectors.to_le_bytes());
    put(&mut boot, 44, &geometry.root_cluster.to_le_bytes());
    put(
        &mut boot,
        48,
        &u16::try_from(FSINFO_SECTOR).unwrap_or(1).to_le_bytes(),
    );
    put(
        &mut boot,
        50,
        &u16::try_from(BACKUP_BOOT_SECTOR).unwrap_or(6).to_le_bytes(),
    );
    put(&mut boot, 64, &[0x80]);
    put(&mut boot, 66, &[0x29]);
    put(&mut boot, 67, &options.volume_id.to_le_bytes());
    put(&mut boot, 71, &options.volume_label);
    put(&mut boot, 82, b"FAT32   ");
    put(&mut boot, 510, &[0x55, 0xAA]);
    boot
}

/// [`SECTOR`] as the boot sector writes it.
const SECTOR_U16: u16 = 512;

/// The file system information sector, which records what a mount would
/// otherwise count.
pub(crate) fn info_sector(free: u32, next_free: u32) -> [u8; SECTOR] {
    let mut info = [0u8; SECTOR];
    put(&mut info, 0, &FSINFO_LEAD.to_le_bytes());
    put(&mut info, 484, &FSINFO_STRUCT.to_le_bytes());
    put(&mut info, 488, &free.to_le_bytes());
    put(&mut info, 492, &next_free.to_le_bytes());
    put(&mut info, 508, &FSINFO_TRAIL.to_le_bytes());
    info
}

/// Reads the boot sector of `device`.
///
/// # Errors
///
/// The errors of [`parse`], and [`Error::Device`] for a device that will
/// not give up its first sector.
pub fn read_geometry<D: BlockDevice>(device: &D) -> Result<Geometry, Error> {
    let mut boot = [0u8; SECTOR];
    device.read(0, &mut boot)?;
    parse(&boot)
}
