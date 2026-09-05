// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The types every UEFI structure is built from.

use core::ffi::c_void;

/// An opaque firmware handle.
pub type Handle = *mut c_void;

/// A globally unique identifier, in the mixed-endian form the
/// specification uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct Guid {
    /// First group, little-endian.
    pub data1: u32,
    /// Second group, little-endian.
    pub data2: u16,
    /// Third group, little-endian.
    pub data3: u16,
    /// Fourth and fifth group, byte order as written.
    pub data4: [u8; 8],
}

impl Guid {
    /// The identifier with the given groups.
    #[must_use]
    pub const fn new(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        Guid {
            data1,
            data2,
            data3,
            data4,
        }
    }
}

/// The header every UEFI table starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct TableHeader {
    /// Identifies the table.
    pub signature: u64,
    /// Version of the table.
    pub revision: u32,
    /// Size of the whole table in bytes.
    pub header_size: u32,
    /// Checksum over the table with this field zeroed.
    pub crc32: u32,
    /// Zero.
    pub reserved: u32,
}

/// One entry of the configuration table.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ConfigurationTable {
    /// Names what `vendor_table` points at.
    pub vendor_guid: Guid,
    /// The table itself.
    pub vendor_table: *mut c_void,
}
