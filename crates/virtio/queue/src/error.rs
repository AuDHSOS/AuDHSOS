// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a queue operation or an initialization step was refused.

use core::fmt;

/// One of the three memory regions a virtqueue occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Area {
    /// The descriptor table, written by the driver.
    DescriptorTable,
    /// The available ring, written by the driver.
    AvailableRing,
    /// The used ring, written by the device.
    UsedRing,
}

impl fmt::Display for Area {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Area::DescriptorTable => f.write_str("descriptor table"),
            Area::AvailableRing => f.write_str("available ring"),
            Area::UsedRing => f.write_str("used ring"),
        }
    }
}

/// What a queue operation refused, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QueueError {
    /// The queue size is zero, is not a power of two, is above
    /// [`MAX_QUEUE_SIZE`](crate::memory::MAX_QUEUE_SIZE), or is more
    /// descriptors than the free set of this queue has bits for.
    Size(u16),
    /// A region the memory handed over is shorter than the access needs.
    Region {
        /// Which region.
        area: Area,
        /// The byte the access ends at.
        needed: usize,
        /// How many bytes the region has.
        given: usize,
    },
    /// A request without buffers. Virtio has no chain of no descriptors.
    EmptyChain,
    /// A buffer the device writes stands before one it reads. A chain
    /// puts everything the device reads first, and this is refused
    /// rather than reordered: the order of the buffers is the order of
    /// the bytes, and moving them would change the request.
    BufferOrder,
    /// More buffers than the queue has descriptors. No number of
    /// completions makes this request fit; it is refused rather than
    /// retried.
    ChainTooLong {
        /// How many buffers were offered.
        buffers: usize,
        /// How many descriptors the queue has in total.
        size: u16,
    },
    /// Not enough free descriptors for this chain right now. The chain
    /// that was half built has been given back, and the request fits once
    /// enough completions have been taken.
    QueueFull {
        /// How many descriptors are free now, which is after the
        /// part-built chain came back and therefore the number a retry
        /// has to fit into.
        free: u16,
        /// How many the chain needed.
        needed: usize,
    },
    /// The device is not in a phase where a buffer may go into a queue:
    /// it has not settled its feature set, or it has failed since.
    NotConfigured,
    /// The device has not reached `DRIVER_OK`, or it has failed since.
    NotLive,
    /// A used element names a descriptor this queue never handed out:
    /// one outside the table, or one that is free.
    UnknownDescriptor(u32),
    /// The used index moved backwards, or forwards past the number of
    /// chains the driver has outstanding.
    UsedIndex {
        /// The index the driver had reached.
        last: u16,
        /// The index the device now shows.
        now: u16,
        /// How many chains are outstanding.
        in_flight: u16,
    },
    /// A used element reports more bytes written than the chain gave the
    /// device to write into.
    ///
    /// Section 2.7.8.2 lets a device report fewer bytes than it wrote,
    /// because a device that failed part way may not know how far it
    /// got, and reporting too few is what keeps a driver from handing
    /// out memory that was never overwritten. It does not let a device
    /// report more than it was given room for: that number is true in no
    /// failure case, and a caller that indexed with it would leave the
    /// buffers it handed in. The chain is freed either way; what is
    /// refused is the number.
    UsedLength {
        /// What the used element said.
        reported: u32,
        /// How many bytes the device-writable descriptors of the chain
        /// add up to.
        writable: u32,
    },
    /// A chain stepped onto a descriptor outside the table, or onto one
    /// that is already free, instead of ending.
    ///
    /// The descriptors the walk reached before that step are back in the
    /// free set; the step that made no sense is where it stopped, so
    /// anything behind it is lost. Only the driver writes the descriptor
    /// table, so this says that this crate or its caller has a fault,
    /// not that the device misbehaved, and the queue is worth rebuilding
    /// rather than carrying on.
    CorruptChain(u16),
}

impl fmt::Display for QueueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueueError::Size(size) => write!(f, "{size} is not a usable queue size"),
            QueueError::Region {
                area,
                needed,
                given,
            } => write!(f, "the {area} needs {needed} bytes and has {given}"),
            QueueError::EmptyChain => f.write_str("a chain needs at least one buffer"),
            QueueError::BufferOrder => {
                f.write_str("a buffer the device writes stands before one it reads")
            }
            QueueError::ChainTooLong { buffers, size } => write!(
                f,
                "{buffers} buffers do not fit a queue of {size} descriptors"
            ),
            QueueError::QueueFull { free, needed } => {
                write!(f, "{needed} descriptors are needed and {free} are free")
            }
            QueueError::NotConfigured => f.write_str("the device has not settled its feature set"),
            QueueError::NotLive => f.write_str("the device is not live"),
            QueueError::UnknownDescriptor(id) => {
                write!(f, "the used ring names the unknown descriptor {id}")
            }
            QueueError::UsedIndex {
                last,
                now,
                in_flight,
            } => write!(
                f,
                "the used index went from {last} to {now} with {in_flight} chains outstanding"
            ),
            QueueError::UsedLength { reported, writable } => write!(
                f,
                "the device reports {reported} bytes written where {writable} were writable"
            ),
            QueueError::CorruptChain(head) => {
                write!(f, "the chain at descriptor {head} does not end")
            }
        }
    }
}

/// What an initialization step refused, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceError {
    /// The step does not follow the one the device is in. The device is
    /// left as it was; the way out is a reset.
    OutOfOrder,
    /// The driver asked for a feature this crate does not implement
    /// (D-52). Nothing was written; the request itself is the fault.
    UnsupportedFeature(u64),
    /// The device does not offer `VIRTIO_F_VERSION_1`, so it is a legacy
    /// device and this crate cannot drive it.
    Legacy,
    /// The device cleared `FEATURES_OK` after the driver set it: it does
    /// not accept the negotiated set.
    FeaturesRejected,
    /// The device set `DEVICE_NEEDS_RESET`.
    NeedsReset,
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceError::OutOfOrder => f.write_str("the step does not follow the device state"),
            DeviceError::UnsupportedFeature(bits) => {
                write!(f, "the features {bits:#x} are not implemented")
            }
            DeviceError::Legacy => f.write_str("the device does not offer VIRTIO_F_VERSION_1"),
            DeviceError::FeaturesRejected => f.write_str("the device rejected the feature set"),
            DeviceError::NeedsReset => f.write_str("the device asks for a reset"),
        }
    }
}
