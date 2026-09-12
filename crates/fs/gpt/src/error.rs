// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a table was refused.

use core::fmt;

/// What a call of this crate refused, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Error {
    /// A sector outside the device, or one the device would not move.
    Device(u32),
    /// A block above the count this device addresses, which is what a
    /// table written for a larger one names.
    Lba(u64),
    /// The first block is not a protective record: it has no `0x55 0xAA`
    /// at its end, or no partition of type `0xEE`. The disk is then one
    /// partitioned the legacy way, and a table found behind it is stale
    /// (UEFI 2.11, section 5.3.2).
    NotProtective,
    /// The header does not begin with `EFI PART`.
    Signature,
    /// A header revision this crate does not read. Only 1.0 is read.
    Revision(u32),
    /// The header claims a size below the 92 bytes the fields need, or
    /// one above the block it lies in.
    HeaderSize(u32),
    /// The header's own checksum does not cover its bytes.
    HeaderChecksum,
    /// The header lies in a block other than the one it names.
    MyLba(u64),
    /// An entry size that is not 128 times a power of two, or one that
    /// does not divide a block, so that an entry would straddle two.
    EntrySize(u32),
    /// The entry array does not lie on this device, or it runs into the
    /// usable range it describes.
    ArrayRange,
    /// The checksum of the entry array does not cover its bytes.
    ArrayChecksum,
    /// The first usable block lies behind the last usable one.
    Usable(u64, u64),
    /// The device has fewer blocks than a table needs.
    TooSmall(u64),
    /// A partition name longer than the 36 code units an entry holds.
    Name,
    /// A partition that lies outside the usable range, or one that
    /// overlaps another.
    Partition(u64),
    /// More entries than the array this crate writes holds.
    Space(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Error::Device(sector) => write!(formatter, "the device refused sector {sector}"),
            Error::Lba(lba) => write!(formatter, "block {lba} is above this device"),
            Error::NotProtective => write!(formatter, "the first block is not a protective record"),
            Error::Signature => write!(formatter, "the header does not begin with `EFI PART`"),
            Error::Revision(revision) => write!(formatter, "header revision {revision:#010x}"),
            Error::HeaderSize(len) => write!(formatter, "a header of {len} bytes"),
            Error::HeaderChecksum => write!(formatter, "the header checksum does not match"),
            Error::MyLba(lba) => write!(formatter, "the header names block {lba} as its own"),
            Error::EntrySize(len) => write!(formatter, "an entry of {len} bytes"),
            Error::ArrayRange => write!(formatter, "the entry array does not lie on the device"),
            Error::ArrayChecksum => write!(formatter, "the array checksum does not match"),
            Error::Usable(first, last) => {
                write!(formatter, "the usable range {first} to {last} is empty")
            }
            Error::TooSmall(sectors) => write!(formatter, "a device of {sectors} blocks"),
            Error::Name => write!(formatter, "a partition name of more than 36 code units"),
            Error::Partition(first) => {
                write!(formatter, "the partition at block {first} does not fit")
            }
            Error::Space(count) => write!(formatter, "{count} entries do not fit the array"),
        }
    }
}

impl core::error::Error for Error {}
