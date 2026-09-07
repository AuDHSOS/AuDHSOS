// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(any(test, feature = "test-doubles")), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod boot;
pub mod device;
pub mod dir;
#[cfg(any(test, feature = "test-doubles"))]
pub mod doubles;
pub mod error;
pub mod fs;
pub mod name;
pub mod table;
pub mod time;

pub use boot::{
    BACKUP_BOOT_SECTOR, DEFAULT_FAT_COUNT, DEFAULT_RESERVED_SECTORS, FSINFO_SECTOR, FormatOptions,
    Geometry, MAX_SECTORS_PER_CLUSTER, MEDIA, MIN_CLUSTERS, ROOT_CLUSTER, geometry_for, parse,
    read_geometry,
};
pub use device::{BlockDevice, SECTOR};
pub use dir::{
    ATTR_ARCHIVE, ATTR_DIRECTORY, ATTR_HIDDEN, ATTR_LONG_NAME, ATTR_READ_ONLY, ATTR_SYSTEM,
    ATTR_VOLUME_ID, DELETED, Dir, ENTRY_LEN, Entry, Location,
};
pub use error::Error;
pub use fs::{Entries, File, FileSystem, info};
pub use name::{NAME_LEN, Name};
pub use table::{BAD_CLUSTER, END_MARK, END_OF_CHAIN, ENTRY_MASK};

#[cfg(test)]
mod tests;
