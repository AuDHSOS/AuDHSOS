// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What the tests of this crate build on: a formatted volume in memory.

use audhsos_time::UnixTime;
use fs_fat::doubles::RamDisk;
use fs_fat::{FileSystem, FormatOptions};

/// Sectors of the volume every test works on. FAT32 wants at least 65525
/// clusters, and one sector per cluster is what the format writes by
/// default, so this is the smallest volume that is one.
pub(super) const SECTORS: u32 = 70_000;

/// A volume nothing has been written to.
pub(super) fn volume() -> FileSystem<RamDisk> {
    FileSystem::format(RamDisk::new(SECTORS), &FormatOptions::default())
        .expect("a blank disk of this size formats")
}

/// The moment every entry a test makes carries: an even second in 2026,
/// which is what a directory entry can hold.
pub(super) fn now() -> UnixTime {
    crate::moment(1_767_225_600_000_000)
}
