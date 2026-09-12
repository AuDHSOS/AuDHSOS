// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The device configuration of a block device (virtio 5.2.4), and the
//! rule for reading a field of it that is wider than one access.

use crate::common::CONFIG_GENERATION;
use crate::error::BlkError;
use crate::registers::{Registers, Structure, Width};

/// How many 512-byte sectors the device has. The one field that is there
/// whatever was negotiated (virtio 5.2.4).
pub const CAPACITY: u16 = 0x00;

/// The largest one segment may be, with `VIRTIO_BLK_F_SIZE_MAX`.
pub const SIZE_MAX: u16 = 0x08;

/// The most segments a request may have, with `VIRTIO_BLK_F_SEG_MAX`.
pub const SEG_MAX: u16 = 0x0C;

/// Cylinders, heads and sectors, with `VIRTIO_BLK_F_GEOMETRY`.
pub const GEOMETRY: u16 = 0x10;

/// The block size that suits the device, with `VIRTIO_BLK_F_BLK_SIZE`.
pub const BLK_SIZE: u16 = 0x14;

/// Bytes of a sector, in every request whatever `BLK_SIZE` says
/// (virtio 5.2.5, point 2).
pub const SECTOR_LEN: u32 = 512;

/// How many times a read of the configuration is attempted before it is
/// refused.
///
/// Virtio 2.5.1 has the driver read the generation, then the field, then
/// the generation again, and start over where the two disagree. The loop
/// is bounded rather than open: a device that changes its configuration
/// faster than it can be read is a device this driver reports instead of
/// spinning on.
pub const ATTEMPTS: u32 = 8;

/// The capacity in 512-byte sectors.
///
/// Eight bytes is wider than virtio 2.5.1 lets a driver assume is read at
/// once, so the read is the one that section prescribes.
///
/// # Errors
///
/// [`BlkError::Generation`] when the configuration changed under every
/// one of [`ATTEMPTS`] reads.
pub fn capacity(registers: &impl Registers) -> Result<u64, BlkError> {
    stable(registers, || {
        registers.read(Structure::Device, CAPACITY, Width::B64)
    })
}

/// The value `read` answers with the generation unchanged around it.
fn stable<T: PartialEq>(registers: &impl Registers, read: impl Fn() -> T) -> Result<T, BlkError> {
    for _ in 0..ATTEMPTS {
        let before = registers.read(Structure::Common, CONFIG_GENERATION, Width::B8);
        let value = read();
        let after = registers.read(Structure::Common, CONFIG_GENERATION, Width::B8);
        if before == after {
            return Ok(value);
        }
    }
    Err(BlkError::Generation(ATTEMPTS))
}
