// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why an initialization, a receive or a send was refused.

use core::fmt;

use virtio_queue::{DeviceError, QueueError};

use crate::queues::Side;

/// What a call of this crate refused, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NetError {
    /// The device status did not read back as zero, which virtio 4.1.4.3.2
    /// requires before a device is initialized again.
    NotReset(u8),
    /// The device has no queue of this side: `queue_size` read zero, which
    /// virtio 4.1.4.3 gives as the value of a queue that does not exist.
    NoQueue(Side),
    /// A queue size that is zero, not a power of two, or larger than the
    /// rings the caller prepared. The number is what was asked for.
    QueueSize(u16),
    /// A queue or frame area larger than the driver has slots for.
    /// The number is the size the caller supplied.
    Slots(u16),
    /// The device would not take the interrupt vector: it read back as
    /// `NO_VECTOR` after a vector was written (virtio 4.1.5.1.3, step 5).
    NoVector(u16),
    /// The notification offset of a queue, times the multiplier of the
    /// capability, does not fit the sixteen bits a structure offset has.
    /// The number is the queue's own offset.
    NotifyOffset(u32),
    /// The configuration space kept changing under the read. The number
    /// is how many attempts were made.
    Generation(u32),
    /// A frame longer than one buffer holds behind the header.
    FrameTooLong {
        /// Bytes the caller offered.
        len: usize,
        /// Bytes a buffer holds behind the header.
        capacity: usize,
    },
    /// A used element of the receive queue that does not even hold the
    /// header. The number is what the device reported.
    ShortFrame(u32),
    /// A receive buffer shorter than the 1526 bytes required without
    /// merged receive buffers or guest offloads.
    BufferTooShort(u32),
    /// A used element naming a buffer this driver did not post, or a
    /// buffer index outside the area. The number is the descriptor.
    UnknownBuffer(u16),
    /// A send with no free transmit buffer. Nothing was written, so the
    /// caller may drain the completions and try again.
    NoBuffer,
    /// A buffer the memory would not hand over, or one too short for the
    /// header. The number is the buffer.
    Buffer(u16),
    /// The initialization state machine refused a step.
    Device(DeviceError),
    /// A queue refused an operation.
    Queue(QueueError),
}

impl From<DeviceError> for NetError {
    fn from(error: DeviceError) -> NetError {
        NetError::Device(error)
    }
}

impl From<QueueError> for NetError {
    fn from(error: QueueError) -> NetError {
        NetError::Queue(error)
    }
}

impl fmt::Display for NetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            NetError::NotReset(status) => {
                write!(formatter, "the device status reads {status:#04x}, not zero")
            }
            NetError::NoQueue(side) => write!(formatter, "the device has no {side} queue"),
            NetError::QueueSize(size) => write!(formatter, "a queue of {size} descriptors"),
            NetError::Slots(size) => {
                write!(formatter, "{size} slots are more than this driver holds")
            }
            NetError::NoVector(vector) => {
                write!(formatter, "the device would not take vector {vector}")
            }
            NetError::NotifyOffset(offset) => {
                write!(formatter, "a notification offset of {offset} is too far")
            }
            NetError::Generation(attempts) => write!(
                formatter,
                "the configuration changed under {attempts} reads of it"
            ),
            NetError::FrameTooLong { len, capacity } => {
                write!(formatter, "{len} bytes of frame where {capacity} fit")
            }
            NetError::ShortFrame(length) => {
                write!(
                    formatter,
                    "the device wrote {length} bytes, less than a header"
                )
            }
            NetError::BufferTooShort(length) => {
                write!(formatter, "a receive buffer of {length} bytes is too short")
            }
            NetError::UnknownBuffer(descriptor) => {
                write!(
                    formatter,
                    "descriptor {descriptor} names no buffer of this driver"
                )
            }
            NetError::NoBuffer => formatter.write_str("no transmit buffer is free"),
            NetError::Buffer(index) => write!(formatter, "buffer {index} is not there"),
            NetError::Device(error) => write!(formatter, "{error}"),
            NetError::Queue(error) => write!(formatter, "{error}"),
        }
    }
}

impl core::error::Error for NetError {}
