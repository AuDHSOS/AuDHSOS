// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What every test file here needs: a volume small enough to be quick and
//! large enough to be FAT32, and a fixed moment to stamp entries with.

use audhsos_time::{CivilTime, UnixTime};

use crate::boot::FormatOptions;
use crate::doubles::RamDisk;
use crate::fs::FileSystem;

/// Sectors of the smallest volume these tests use. One sector per
/// cluster, so the data region has to hold more than the 65525 clusters
/// that make a volume FAT32, and the tables and the reserved sectors come
/// on top.
pub(crate) const SECTORS: u32 = 66_600;

/// A fresh volume on a device of [`SECTORS`] sectors.
pub(crate) fn volume() -> FileSystem<RamDisk> {
    FileSystem::format(RamDisk::new(SECTORS), &options()).expect("format")
}

/// The options these tests format with.
pub(crate) fn options() -> FormatOptions {
    FormatOptions {
        volume_id: 0x1234_5678,
        volume_label: *b"TEST       ",
        ..FormatOptions::default()
    }
}

/// A moment a directory entry can carry: 2026-01-01T00:00:00Z.
pub(crate) fn moment() -> UnixTime {
    let civil = CivilTime::new(2026, 1, 1, 0, 0, 0).expect("civil");
    UnixTime::from_civil(civil).expect("unix")
}
