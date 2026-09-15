// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Which queue is which, where its three rings are, and which message
//! interrupt vector it raises.

use core::fmt;

use crate::common;

/// One of the two queues of a device without `VIRTIO_NET_F_MQ`
/// (virtio 5.1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// The queue the device writes frames into, which is queue zero.
    Receive,
    /// The queue the driver writes frames into, which is queue one.
    Transmit,
}

impl Side {
    /// Both, in the order the device numbers them.
    pub const ALL: [Side; 2] = [Side::Receive, Side::Transmit];

    /// The index the queue selector takes.
    #[must_use]
    pub const fn index(self) -> u16 {
        match self {
            Side::Receive => common::RECEIVE_QUEUE,
            Side::Transmit => common::TRANSMIT_QUEUE,
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Side::Receive => formatter.write_str("receive"),
            Side::Transmit => formatter.write_str("transmit"),
        }
    }
}

/// Where the three areas of one queue are, and how large the caller made
/// them.
///
/// The addresses are the device's view of memory, which the caller gets
/// from the object it mapped; nothing here derives an address.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rings {
    /// Physical address of the descriptor table.
    pub descriptor_table: u64,
    /// Physical address of the available ring.
    pub available_ring: u64,
    /// Physical address of the used ring.
    pub used_ring: u64,
    /// Descriptors the caller made room for. A device that offers fewer
    /// settles it; one that offers more is held to this.
    pub size: u16,
}

/// The rings of both queues.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Queues {
    /// The receive queue.
    pub receive: Rings,
    /// The transmit queue.
    pub transmit: Rings,
}

impl Queues {
    /// The rings of one side.
    #[must_use]
    pub const fn of(&self, side: Side) -> &Rings {
        match side {
            Side::Receive => &self.receive,
            Side::Transmit => &self.transmit,
        }
    }
}

/// The message interrupt vectors of one device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Vectors {
    /// The vector a configuration change is reported on, or
    /// [`common::NO_VECTOR`].
    pub config: u16,
    /// The vector the receive queue is reported on, or
    /// [`common::NO_VECTOR`].
    pub receive: u16,
    /// The vector the transmit queue is reported on, or
    /// [`common::NO_VECTOR`].
    pub transmit: u16,
}

impl Vectors {
    /// Neither a queue nor a configuration vector, which is a device
    /// driven without message interrupts.
    #[must_use]
    pub const fn none() -> Vectors {
        Vectors {
            config: common::NO_VECTOR,
            receive: common::NO_VECTOR,
            transmit: common::NO_VECTOR,
        }
    }

    /// One vector for both queues, which is what a driver that waits on
    /// one notification asks for.
    #[must_use]
    pub const fn shared(vector: u16) -> Vectors {
        Vectors {
            config: common::NO_VECTOR,
            receive: vector,
            transmit: vector,
        }
    }

    /// The vector of one side.
    #[must_use]
    pub const fn of(&self, side: Side) -> u16 {
        match side {
            Side::Receive => self.receive,
            Side::Transmit => self.transmit,
        }
    }
}

/// What one queue was configured with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Configured {
    /// Descriptors the queue was given.
    pub size: u16,
    /// Where in the notification structure this queue is notified, the
    /// multiplier of the capability already applied.
    pub notify: u16,
}
