// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a request or an initialization was refused.

use core::fmt;

use virtio_queue::{DeviceError, QueueError};

/// What a call of this crate refused, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlkError {
    /// The device status did not read back as zero, which virtio 4.1.4.3.2
    /// requires before a device is initialized again.
    NotReset(u8),
    /// The device has no request queue: `queue_size` read zero, which
    /// virtio 4.1.4.3 gives as the value of a queue that does not exist.
    NoQueue,
    /// A queue size that is zero, not a power of two, or larger than the
    /// rings the caller prepared. The number is what was asked for.
    QueueSize(u16),
    /// The device would not take the interrupt vector: it read back as
    /// `NO_VECTOR` after a vector was written (virtio 4.1.5.1.3, step 5).
    NoVector(u16),
    /// The notification offset of the queue, times the multiplier of the
    /// capability, does not fit the sixteen bits a structure offset has.
    /// The number is the queue's own offset.
    NotifyOffset(u32),
    /// The configuration space kept changing under the read. The number
    /// is how many attempts were made.
    Generation(u32),
    /// A request beyond the last sector of the device.
    Capacity {
        /// First sector of the request.
        sector: u64,
        /// Sectors it covers.
        sectors: u64,
        /// Sectors the device has.
        capacity: u64,
    },
    /// A read or a write whose data is not a whole number of 512-byte
    /// sectors, or is nothing at all (virtio 5.2.6.1).
    DataLength(u32),
    /// A read or a write with no data, or a flush with some.
    Framing,
    /// A flush whose sector is not zero, which virtio 5.2.6.1 requires.
    FlushSector(u64),
    /// A write to a device that offered `VIRTIO_BLK_F_RO`.
    ReadOnly,
    /// A buffer too short for the header or the status byte it is to
    /// hold.
    Buffer(usize),
    /// The device reported an error for a request: the status byte was
    /// not `VIRTIO_BLK_S_OK`.
    Status(u8),
    /// The initialization state machine refused a step.
    Device(DeviceError),
    /// The queue refused an operation.
    Queue(QueueError),
}

impl From<DeviceError> for BlkError {
    fn from(error: DeviceError) -> BlkError {
        BlkError::Device(error)
    }
}

impl From<QueueError> for BlkError {
    fn from(error: QueueError) -> BlkError {
        BlkError::Queue(error)
    }
}

impl fmt::Display for BlkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            BlkError::NotReset(status) => {
                write!(formatter, "the device status reads {status:#04x}, not zero")
            }
            BlkError::NoQueue => write!(formatter, "the device has no request queue"),
            BlkError::QueueSize(size) => write!(formatter, "a queue of {size} descriptors"),
            BlkError::NoVector(vector) => {
                write!(formatter, "the device would not take vector {vector}")
            }
            BlkError::NotifyOffset(offset) => {
                write!(formatter, "a notification offset of {offset} is too far")
            }
            BlkError::Generation(attempts) => write!(
                formatter,
                "the configuration changed under {attempts} reads of it"
            ),
            BlkError::Capacity {
                sector,
                sectors,
                capacity,
            } => write!(
                formatter,
                "{sectors} sectors from {sector} reach past the {capacity} the device has"
            ),
            BlkError::DataLength(length) => {
                write!(formatter, "{length} bytes is not whole sectors")
            }
            BlkError::Framing => write!(formatter, "the request carries the wrong parts"),
            BlkError::FlushSector(sector) => {
                write!(formatter, "a flush of sector {sector} rather than zero")
            }
            BlkError::ReadOnly => write!(formatter, "the device is read-only"),
            BlkError::Buffer(len) => write!(formatter, "a buffer of {len} bytes is too short"),
            BlkError::Status(status) => write!(formatter, "the device answered {status}"),
            BlkError::Device(error) => write!(formatter, "{error}"),
            BlkError::Queue(error) => write!(formatter, "{error}"),
        }
    }
}

impl core::error::Error for BlkError {}
