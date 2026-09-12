// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

/// Bytes in one block. The format calls it a logical block and allows
/// others; this crate reads 512, which is what every offset here counts
/// in and what the trait it reads through moves.
pub const SECTOR: usize = fs_fat::SECTOR;

pub mod bytes;
pub mod crc32;
pub mod entry;
pub mod error;
pub mod header;
pub mod mbr;
pub mod table;

pub use crc32::{Crc32, crc32};
pub use entry::{ENTRY_LEN, ESP_TYPE_GUID, Entry, GUID_LEN, NAME_UNITS, UNUSED_TYPE_GUID};
pub use error::Error;
pub use header::{
    ARRAY_SECTORS, ENTRY_COUNT, FIRST_USABLE, HEADER_LBA, HEADER_LEN, HEADER_REVISION,
    HEADER_SIGNATURE, Header, MIN_SECTORS, last_usable,
};
pub use mbr::{PROTECTIVE_TYPE, is_protective, protective};
pub use table::{Cursor, find, next_entry, read, write};

#[cfg(test)]
mod tests;
