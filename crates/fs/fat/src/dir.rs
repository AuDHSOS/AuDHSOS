// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The thirty-two bytes of a directory entry, and what a slot of a
//! directory can be.

use audhsos_time::UnixTime;

use crate::device::{put, read_u16, read_u32};
use crate::error::Error;
use crate::name::{NAME_LEN, Name};
use crate::time;

/// Bytes of one directory entry.
pub const ENTRY_LEN: usize = 32;

/// The entry is read-only.
pub const ATTR_READ_ONLY: u8 = 0x01;

/// The entry is hidden.
pub const ATTR_HIDDEN: u8 = 0x02;

/// The entry belongs to the system.
pub const ATTR_SYSTEM: u8 = 0x04;

/// The entry is the label of the volume rather than a file.
pub const ATTR_VOLUME_ID: u8 = 0x08;

/// The entry is a directory.
pub const ATTR_DIRECTORY: u8 = 0x10;

/// The entry is a file that has been written since it was last archived,
/// which is what a file gets when it is made.
pub const ATTR_ARCHIVE: u8 = 0x20;

/// The four low attribute bits together mark a long file name, which this
/// crate skips rather than reads (D-09).
pub const ATTR_LONG_NAME: u8 = ATTR_READ_ONLY | ATTR_HIDDEN | ATTR_SYSTEM | ATTR_VOLUME_ID;

/// The first byte of a deleted entry.
pub const DELETED: u8 = 0xE5;

/// A directory, named by the cluster it begins at. The root of a volume
/// begins at [`Geometry::root_cluster`](crate::boot::Geometry).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Dir {
    /// First cluster of the directory.
    pub(crate) cluster: u32,
}

impl Dir {
    /// The directory that begins at `cluster`.
    #[must_use]
    pub const fn at(cluster: u32) -> Dir {
        Dir { cluster }
    }

    /// The cluster the directory begins at.
    #[must_use]
    pub const fn cluster(&self) -> u32 {
        self.cluster
    }
}

/// Where one entry stands: the cluster of the directory that holds it and
/// which slot of that cluster it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    /// The cluster the slot is in.
    pub(crate) cluster: u32,
    /// The slot within that cluster, counted in entries.
    pub(crate) slot: u32,
}

impl Location {
    /// The cluster the slot is in.
    #[must_use]
    pub const fn cluster(&self) -> u32 {
        self.cluster
    }

    /// The slot within that cluster, counted in entries.
    #[must_use]
    pub const fn slot(&self) -> u32 {
        self.slot
    }
}

/// What one directory entry says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The name in its 8.3 form.
    pub name: Name,
    /// The attribute byte.
    pub attributes: u8,
    /// The first cluster of the contents, zero for an empty file.
    pub first_cluster: u32,
    /// The size in bytes; zero for a directory, whose size is its chain.
    pub size: u32,
    /// When the entry was last written.
    pub modified: UnixTime,
    /// Where the entry stands.
    pub location: Location,
}

impl Entry {
    /// Whether the entry is a directory.
    #[must_use]
    pub const fn is_directory(&self) -> bool {
        self.attributes & ATTR_DIRECTORY != 0
    }
}

/// What one slot of a directory holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    /// Nothing, and nothing after it either: the directory ends here.
    End,
    /// A deleted entry, which a new name may take.
    Free,
    /// Something this crate walks past: the volume label, a long file
    /// name, or one of the two entries a directory keeps for itself.
    Skip,
    /// An entry.
    Used(Entry),
}

/// Reads one slot.
///
/// # Errors
///
/// [`Error::EntryName`] where the eleven bytes are not a name this crate
/// would have written, and [`Error::Time`] where the date and the time
/// name no moment.
pub(crate) fn decode(bytes: &[u8], location: Location) -> Result<Slot, Error> {
    let first = *bytes.first().unwrap_or(&0);
    if first == 0 {
        return Ok(Slot::End);
    }
    if first == DELETED {
        return Ok(Slot::Free);
    }
    let attributes = *bytes.get(11).unwrap_or(&0);
    if attributes & ATTR_LONG_NAME == ATTR_LONG_NAME
        || attributes & ATTR_VOLUME_ID != 0
        || first == b'.'
    {
        return Ok(Slot::Skip);
    }
    let mut raw = [0u8; NAME_LEN];
    let source = bytes.get(..NAME_LEN).ok_or(Error::EntryName)?;
    raw.copy_from_slice(source);
    let name = Name::from_entry(raw)?;
    let first_cluster = (u32::from(read_u16(bytes, 20)) << 16) | u32::from(read_u16(bytes, 26));
    Ok(Slot::Used(Entry {
        name,
        attributes,
        first_cluster,
        size: read_u32(bytes, 28),
        modified: time::from_entry(read_u16(bytes, 24), read_u16(bytes, 22))?,
        location,
    }))
}

/// The thirty-two bytes of an entry.
///
/// # Errors
///
/// [`Error::Time`] for a moment a directory entry cannot carry.
pub(crate) fn encode(
    name: &Name,
    attributes: u8,
    first_cluster: u32,
    size: u32,
    modified: UnixTime,
) -> Result<[u8; ENTRY_LEN], Error> {
    let (date, clock) = time::to_entry(modified)?;
    let mut bytes = [0u8; ENTRY_LEN];
    put(&mut bytes, 0, name.as_bytes());
    put(&mut bytes, 11, &[attributes]);
    put(&mut bytes, 14, &clock.to_le_bytes());
    put(&mut bytes, 16, &date.to_le_bytes());
    put(&mut bytes, 18, &date.to_le_bytes());
    put(
        &mut bytes,
        20,
        &u16::try_from(first_cluster >> 16)
            .unwrap_or(0)
            .to_le_bytes(),
    );
    put(&mut bytes, 22, &clock.to_le_bytes());
    put(&mut bytes, 24, &date.to_le_bytes());
    put(
        &mut bytes,
        26,
        &u16::try_from(first_cluster & 0xFFFF)
            .unwrap_or(0)
            .to_le_bytes(),
    );
    put(&mut bytes, 28, &size.to_le_bytes());
    Ok(bytes)
}

/// Writes the first cluster and the size into an entry that is already
/// there, leaving everything else as it stands.
pub(crate) fn patch(bytes: &mut [u8], first_cluster: u32, size: u32) {
    put(
        bytes,
        20,
        &u16::try_from(first_cluster >> 16)
            .unwrap_or(0)
            .to_le_bytes(),
    );
    put(
        bytes,
        26,
        &u16::try_from(first_cluster & 0xFFFF)
            .unwrap_or(0)
            .to_le_bytes(),
    );
    put(bytes, 28, &size.to_le_bytes());
}
