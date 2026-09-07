// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why an operation was refused.

use core::fmt;

/// What a call of this crate refused, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Error {
    /// A sector outside the device, or one the device would not move.
    Device(u32),
    /// The boot sector does not end in `0x55 0xAA`.
    Signature,
    /// The volume claims a sector size this crate does not read; only 512
    /// bytes is read, because that is what the layout constants assume.
    SectorSize(u16),
    /// The cluster size is zero, is not a power of two, or is more than
    /// [`MAX_SECTORS_PER_CLUSTER`](crate::boot::MAX_SECTORS_PER_CLUSTER)
    /// sectors.
    ClusterSize(u32),
    /// The volume claims no file allocation table, or more than one
    /// table's worth of reserved sectors is missing.
    Layout,
    /// The volume is a FAT12 or a FAT16 one: it has a fixed root
    /// directory, a sixteen-bit table size, or fewer clusters than the
    /// 65525 that make a volume FAT32. The number is what it holds.
    NotFat32(u32),
    /// The device is too small for the tables and one data cluster.
    TooSmall(u32),
    /// A cluster number outside the data region of this volume.
    Cluster(u32),
    /// A chain step reached a free cluster: the table and the entry that
    /// named the chain disagree about what is in use.
    FreeInChain(u32),
    /// A chain step reached the bad-cluster marker.
    BadCluster(u32),
    /// A chain was walked for as many steps as the volume has clusters
    /// without reaching its end, so it points back into itself.
    ChainLoop(u32),
    /// No free cluster is left.
    Full,
    /// The name is not one the 8.3 form can carry: it is empty, its stem
    /// or its extension is too long, or it holds a character the form
    /// does not allow.
    Name,
    /// A directory entry holds eleven bytes that are not a name this
    /// crate would write.
    EntryName,
    /// The name is already in the directory.
    Exists,
    /// The name is not in the directory.
    NotFound,
    /// The entry is a directory where a file was wanted, or the other way
    /// round.
    Kind,
    /// A read or a write beyond the end of the file. A write may begin at
    /// the end and extend it; it may not begin past it, because the bytes
    /// in between were never written.
    Offset(u32),
    /// A file that would be larger than the four gigabytes a directory
    /// entry can record.
    TooLarge,
    /// A time outside what a directory entry carries: before 1980, after
    /// 2107, or with an odd second, FAT counting seconds in twos.
    Time,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Error::Device(sector) => write!(f, "the device would not move sector {sector}"),
            Error::Signature => f.write_str("the boot sector carries no signature"),
            Error::SectorSize(size) => {
                write!(f, "a sector of {size} bytes, where 512 is what is read")
            }
            Error::ClusterSize(sectors) => {
                write!(f, "{sectors} sectors per cluster is not a power of two")
            }
            Error::Layout => f.write_str("the volume declares no file allocation table"),
            Error::NotFat32(clusters) => {
                write!(f, "a volume of {clusters} clusters is not a FAT32 one")
            }
            Error::TooSmall(sectors) => {
                write!(f, "a device of {sectors} sectors is too small for FAT32")
            }
            Error::Cluster(cluster) => write!(f, "the cluster {cluster} is not in the data region"),
            Error::FreeInChain(cluster) => {
                write!(f, "the chain reached the free cluster {cluster}")
            }
            Error::BadCluster(cluster) => write!(f, "the chain reached the bad cluster {cluster}"),
            Error::ChainLoop(first) => write!(f, "the chain at {first} points back into itself"),
            Error::Full => f.write_str("no free cluster is left"),
            Error::Name => f.write_str("the name is not an 8.3 name"),
            Error::EntryName => f.write_str("a directory entry carries a name that is not one"),
            Error::Exists => f.write_str("the name is already in the directory"),
            Error::NotFound => f.write_str("the name is not in the directory"),
            Error::Kind => f.write_str("the entry is not of the kind that was wanted"),
            Error::Offset(offset) => write!(f, "the offset {offset} is past the end of the file"),
            Error::TooLarge => f.write_str("a file cannot be larger than four gigabytes"),
            Error::Time => f.write_str("the time is not one a directory entry carries"),
        }
    }
}
