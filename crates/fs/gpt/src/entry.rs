// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One partition entry: what it is, where it begins and ends, and what it
//! is called. UEFI 2.11, table 5.6.

use crate::bytes::{put, read_guid, read_u64};
use crate::error::Error;

/// Bytes in a GUID.
pub const GUID_LEN: usize = 16;

/// Bytes in one entry. The header may name a larger one, in which case
/// the bytes above these are reserved and read as nothing.
pub const ENTRY_LEN: usize = 128;

/// Code units in the name an entry carries: 72 bytes of UTF-16.
pub const NAME_UNITS: usize = 36;

/// The type of an entry that is in use by nobody.
pub const UNUSED_TYPE_GUID: [u8; GUID_LEN] = [0u8; GUID_LEN];

/// `C12A7328-F81F-11D2-BA4B-00A0C93EC93B`, the EFI system partition
/// (UEFI 2.11, table 5.7). The first three fields of a GUID are written
/// little-endian and the last two as they read, which is why the bytes
/// are not the text in order.
pub const ESP_TYPE_GUID: [u8; GUID_LEN] = [
    0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B,
];

/// A partition, as its entry describes it.
///
/// The two GUIDs are the sixteen bytes as they lie on the device, not a
/// parsed form: this crate compares them and copies them, and never needs
/// the fields the mixed-endian text form has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// What the partition holds. [`UNUSED_TYPE_GUID`] means the entry is
    /// in use by nobody.
    pub type_guid: [u8; GUID_LEN],
    /// What this partition is, as against every other one ever made.
    pub unique_guid: [u8; GUID_LEN],
    /// First block of the partition.
    pub first_lba: u64,
    /// Last block of the partition, itself included.
    pub last_lba: u64,
    /// Bits the format reserves; this crate carries them and reads none.
    pub attributes: u64,
    /// The name, as UTF-16 code units padded with zeros.
    pub name: [u16; NAME_UNITS],
}

impl Entry {
    /// An entry for a partition of `type_guid` over `first_lba` to
    /// `last_lba`, named `name`.
    ///
    /// # Errors
    ///
    /// [`Error::Name`] for a name of more than [`NAME_UNITS`] code units,
    /// [`Error::Partition`] for a last block before the first.
    pub fn new(
        type_guid: [u8; GUID_LEN],
        unique_guid: [u8; GUID_LEN],
        first_lba: u64,
        last_lba: u64,
        name: &str,
    ) -> Result<Entry, Error> {
        if last_lba < first_lba {
            return Err(Error::Partition(first_lba));
        }
        let mut units = [0u16; NAME_UNITS];
        let mut written = 0usize;
        for unit in name.encode_utf16() {
            let slot = units.get_mut(written).ok_or(Error::Name)?;
            *slot = unit;
            written = written.saturating_add(1);
        }
        Ok(Entry {
            type_guid,
            unique_guid,
            first_lba,
            last_lba,
            attributes: 0,
            name: units,
        })
    }

    /// Whether the entry describes a partition. An entry of no type is
    /// one no partition uses.
    #[must_use]
    pub fn is_used(&self) -> bool {
        self.type_guid != UNUSED_TYPE_GUID
    }

    /// How many blocks the partition covers.
    #[must_use]
    pub const fn blocks(&self) -> u64 {
        self.last_lba
            .saturating_sub(self.first_lba)
            .saturating_add(1)
    }

    /// The entry `bytes` describe. Bytes the entry does not reach read as
    /// zero, so a short slice yields an unused entry rather than a
    /// refusal: what decides whether an entry is there is its type.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Entry {
        let mut name = [0u16; NAME_UNITS];
        for (index, unit) in name.iter_mut().enumerate() {
            let offset = 56usize.saturating_add(index.saturating_mul(2));
            *unit = bytes
                .get(offset..offset.saturating_add(2))
                .and_then(|slice| slice.try_into().ok())
                .map_or(0, u16::from_le_bytes);
        }
        Entry {
            type_guid: read_guid(bytes, 0),
            unique_guid: read_guid(bytes, 16),
            first_lba: read_u64(bytes, 32),
            last_lba: read_u64(bytes, 40),
            attributes: read_u64(bytes, 48),
            name,
        }
    }

    /// Writes the entry into `bytes`, which the caller has zeroed. Bytes
    /// beyond what `bytes` holds are dropped, as everywhere in this crate.
    pub fn write(&self, bytes: &mut [u8]) {
        put(bytes, 0, &self.type_guid);
        put(bytes, 16, &self.unique_guid);
        put(bytes, 32, &self.first_lba.to_le_bytes());
        put(bytes, 40, &self.last_lba.to_le_bytes());
        put(bytes, 48, &self.attributes.to_le_bytes());
        for (index, unit) in self.name.iter().enumerate() {
            let offset = 56usize.saturating_add(index.saturating_mul(2));
            put(bytes, offset, &unit.to_le_bytes());
        }
    }
}
