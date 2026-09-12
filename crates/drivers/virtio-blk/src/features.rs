// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The feature bits of a block device (virtio 5.2.3), what this driver
//! takes, and what it refuses.
//!
//! Every bit the format has is named here even where the driver refuses
//! it, so that an omission reads as a decision and a refusal can say
//! which feature it turned down.

use virtio_queue::F_VERSION_1;

/// `VIRTIO_BLK_F_BARRIER`, of the legacy interface only (virtio 5.2.3.1).
pub const F_BARRIER: u64 = 1 << 0;

/// `VIRTIO_BLK_F_SIZE_MAX`: the largest one segment may be.
pub const F_SIZE_MAX: u64 = 1 << 1;

/// `VIRTIO_BLK_F_SEG_MAX`: the most segments one request may have.
pub const F_SEG_MAX: u64 = 1 << 2;

/// `VIRTIO_BLK_F_GEOMETRY`: cylinders, heads and sectors.
pub const F_GEOMETRY: u64 = 1 << 4;

/// `VIRTIO_BLK_F_RO`: the device is read-only.
pub const F_RO: u64 = 1 << 5;

/// `VIRTIO_BLK_F_BLK_SIZE`: the block size that suits the device best.
pub const F_BLK_SIZE: u64 = 1 << 6;

/// `VIRTIO_BLK_F_SCSI`, of the legacy interface only (virtio 5.2.3.1).
pub const F_SCSI: u64 = 1 << 7;

/// `VIRTIO_BLK_F_FLUSH`: the device takes a cache flush.
pub const F_FLUSH: u64 = 1 << 9;

/// `VIRTIO_BLK_F_TOPOLOGY`: what the device's alignment is.
pub const F_TOPOLOGY: u64 = 1 << 10;

/// `VIRTIO_BLK_F_CONFIG_WCE`: the cache mode can be switched.
pub const F_CONFIG_WCE: u64 = 1 << 11;

/// `VIRTIO_BLK_F_MQ`: more than one request queue.
pub const F_MQ: u64 = 1 << 12;

/// `VIRTIO_BLK_F_DISCARD`: the discard command.
pub const F_DISCARD: u64 = 1 << 13;

/// `VIRTIO_BLK_F_WRITE_ZEROES`: the write zeroes command.
pub const F_WRITE_ZEROES: u64 = 1 << 14;

/// `VIRTIO_BLK_F_LIFETIME`: how worn the storage is.
pub const F_LIFETIME: u64 = 1 << 15;

/// `VIRTIO_BLK_F_SECURE_ERASE`: the secure erase command.
pub const F_SECURE_ERASE: u64 = 1 << 16;

/// `VIRTIO_BLK_F_ZONED`: the device is a zoned one.
pub const F_ZONED: u64 = 1 << 17;

/// What this driver asks the device for.
///
/// `VIRTIO_F_VERSION_1` because nothing here reads a legacy device.
/// `VIRTIO_BLK_F_FLUSH` because a system that writes a file system and
/// cannot ask for the write to reach the disk is telling the caller
/// something it does not know. `VIRTIO_BLK_F_RO` because virtio 5.2.6.1
/// asks a driver to accept it if offered, and because a write refused
/// here says more than a status byte does later.
pub const WANTED: u64 = F_VERSION_1 | F_FLUSH | F_RO;

/// Every bit this module has a name for.
const NAMED: [(u64, &str); 16] = [
    (F_BARRIER, "VIRTIO_BLK_F_BARRIER"),
    (F_SIZE_MAX, "VIRTIO_BLK_F_SIZE_MAX"),
    (F_SEG_MAX, "VIRTIO_BLK_F_SEG_MAX"),
    (F_GEOMETRY, "VIRTIO_BLK_F_GEOMETRY"),
    (F_RO, "VIRTIO_BLK_F_RO"),
    (F_BLK_SIZE, "VIRTIO_BLK_F_BLK_SIZE"),
    (F_SCSI, "VIRTIO_BLK_F_SCSI"),
    (F_FLUSH, "VIRTIO_BLK_F_FLUSH"),
    (F_TOPOLOGY, "VIRTIO_BLK_F_TOPOLOGY"),
    (F_CONFIG_WCE, "VIRTIO_BLK_F_CONFIG_WCE"),
    (F_MQ, "VIRTIO_BLK_F_MQ"),
    (F_DISCARD, "VIRTIO_BLK_F_DISCARD"),
    (F_WRITE_ZEROES, "VIRTIO_BLK_F_WRITE_ZEROES"),
    (F_LIFETIME, "VIRTIO_BLK_F_LIFETIME"),
    (F_SECURE_ERASE, "VIRTIO_BLK_F_SECURE_ERASE"),
    (F_ZONED, "VIRTIO_BLK_F_ZONED"),
];

/// What `bit` is called, for a bit of the block device's own range.
///
/// A bit of the transport's range, which virtio 6 reserves from 24 up,
/// has no name here: those belong to no device type and this driver takes
/// none of them beyond `VIRTIO_F_VERSION_1`.
#[must_use]
pub fn name(bit: u64) -> Option<&'static str> {
    NAMED
        .iter()
        .find(|(value, _)| *value == bit)
        .map(|(_, name)| *name)
}

/// The bits `offered` carries that this driver does not take.
///
/// What comes back is a set, and a caller that reports it walks the bits
/// with [`name`]. It is not an error: virtio 2.2.1 leaves a feature the
/// driver did not want to the driver's judgement, and the judgement here
/// is that the request queue of 5.2.6 needs none of them.
#[must_use]
pub const fn refused(offered: u64) -> u64 {
    offered & !WANTED
}
