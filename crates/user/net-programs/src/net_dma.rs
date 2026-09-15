// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The region the network device reads and writes: the three rings of each
//! of its two queues, and the frame buffers of each side.
//!
//! The driver asks the memory server for the region like any other program
//! and calls `memory_info` on what comes back. A memory object is one
//! contiguous physical range (D-115), so an offset into the mapping is the
//! same offset into physical memory, and that one addition is the whole of
//! the address arithmetic here.
//!
//! The layout is fixed at compile time, each piece at the alignment virtio
//! 2.7 asks for, and the region is cut into its pieces once: the two queues
//! and the two frame areas are four values that borrow disjoint halves of
//! one slice, so the driver can hold all four at once.
//! [`REGION_BYTES`] is what it comes to.
//!
//! Invariant: every accessor answers a slice inside the region or an empty
//! one; nothing here computes an address the caller could pass on.

use core::sync::atomic::{Ordering, fence};

use driver_virtio_net::frames::Frames;
use driver_virtio_net::queues::{Queues, Rings};
use virtio_queue::memory::{
    AVAILABLE_RING_ALIGN, DESCRIPTOR_TABLE_ALIGN, QueueMemory, USED_RING_ALIGN,
    available_ring_bytes, descriptor_table_bytes, used_ring_bytes,
};

/// Descriptors of each queue, and buffers of each side.
///
/// Eight of each: a receive queue of eight buffers holds a burst of eight
/// frames between two rounds of the server, and a transmit queue of eight
/// lets the stack write while the device is still reading.
pub const QUEUE_SIZE: u16 = 8;

/// Bytes of one frame buffer: the twelve-byte header and the largest frame
/// `net-eth` writes, rounded up to the alignment of a descriptor table.
pub const BUFFER_BYTES: usize = 2048;

/// How many buffers each side has.
#[expect(
    clippy::as_conversions,
    reason = "eight widened to the type a length has, in a const"
)]
pub const BUFFERS: usize = QUEUE_SIZE as usize;

/// Bytes of the frame area of one side.
const AREA_BYTES: usize = BUFFER_BYTES * BUFFERS;

/// Bytes the three rings of one queue occupy, the alignment of the next
/// queue included.
const QUEUE_BYTES: usize = align_up(
    align_up(
        align_up(descriptor_table_bytes(QUEUE_SIZE), AVAILABLE_RING_ALIGN)
            + available_ring_bytes(QUEUE_SIZE),
        USED_RING_ALIGN,
    ) + used_ring_bytes(QUEUE_SIZE),
    DESCRIPTOR_TABLE_ALIGN,
);

/// Bytes the whole region occupies.
pub const REGION_BYTES: usize = 2 * QUEUE_BYTES + 2 * AREA_BYTES;

/// Where the available ring of a queue begins inside its own piece.
const AVAILABLE_AT: usize = align_up(descriptor_table_bytes(QUEUE_SIZE), AVAILABLE_RING_ALIGN);

/// Where the used ring begins inside that piece.
const USED_AT: usize = align_up(
    AVAILABLE_AT + available_ring_bytes(QUEUE_SIZE),
    USED_RING_ALIGN,
);

/// Where the used ring ends inside that piece.
const USED_END: usize = USED_AT + used_ring_bytes(QUEUE_SIZE);

/// `value` rounded up to a multiple of `align`.
const fn align_up(value: usize, align: usize) -> usize {
    value.next_multiple_of(align)
}

/// The three rings of one queue, over the bytes of its piece.
pub struct Ring<'a> {
    /// The bytes of the piece.
    bytes: &'a mut [u8],
    /// The physical address the piece begins at.
    physical: u64,
}

impl Ring<'_> {
    /// Where the three rings are, as `initialize` is told them.
    #[must_use]
    #[expect(
        clippy::as_conversions,
        reason = "two offsets of a few hundred bytes widened to an address, in a const fn"
    )]
    pub const fn rings(&self) -> Rings {
        Rings {
            descriptor_table: self.physical,
            available_ring: self.physical.wrapping_add(AVAILABLE_AT as u64),
            used_ring: self.physical.wrapping_add(USED_AT as u64),
            size: QUEUE_SIZE,
        }
    }

    /// Every byte of the piece, which the driver zeroes before it tells
    /// the device where the rings are (virtio 2.7.10.1).
    pub fn clear(&mut self) {
        self.bytes.fill(0);
    }
}

impl QueueMemory for Ring<'_> {
    fn descriptor_table(&self) -> &[u8] {
        region(self.bytes, 0, AVAILABLE_AT)
    }

    fn descriptor_table_mut(&mut self) -> &mut [u8] {
        region_mut(self.bytes, 0, AVAILABLE_AT)
    }

    fn available_ring_mut(&mut self) -> &mut [u8] {
        region_mut(self.bytes, AVAILABLE_AT, USED_AT)
    }

    fn used_ring(&self) -> &[u8] {
        region(self.bytes, USED_AT, USED_END)
    }

    /// The ordering virtio 2.7.13.3.1 and 2.7.13.4.1 require.
    ///
    /// A processor fence is enough and a device fence is not required: the
    /// device is software on the processor this driver runs on, which is
    /// the case virtio 6 describes without `VIRTIO_F_ORDER_PLATFORM`.
    fn barrier(&self) {
        fence(Ordering::SeqCst);
    }
}

/// The frame buffers of one side, over the bytes of its area.
pub struct Area<'a> {
    /// The bytes of the area.
    bytes: &'a mut [u8],
    /// The physical address the area begins at.
    physical: u64,
}

impl Area<'_> {
    /// Where buffer `index` begins in the area.
    fn at(index: u16) -> Option<usize> {
        (usize::from(index) < BUFFERS).then(|| usize::from(index).saturating_mul(BUFFER_BYTES))
    }
}

impl Frames for Area<'_> {
    fn count(&self) -> u16 {
        QUEUE_SIZE
    }

    fn len(&self) -> u32 {
        u32::try_from(BUFFER_BYTES).unwrap_or(0)
    }

    fn address(&self, index: u16) -> Option<u64> {
        let at = Self::at(index)?;
        self.physical.checked_add(u64::try_from(at).ok()?)
    }

    fn bytes(&self, index: u16) -> Option<&[u8]> {
        let at = Self::at(index)?;
        self.bytes.get(at..at.checked_add(BUFFER_BYTES)?)
    }

    fn bytes_mut(&mut self, index: u16) -> Option<&mut [u8]> {
        let at = Self::at(index)?;
        self.bytes.get_mut(at..at.checked_add(BUFFER_BYTES)?)
    }
}

/// The region of one network device, cut into its four pieces.
pub struct NetDma<'a> {
    /// The rings of the receive queue.
    pub receive: Ring<'a>,
    /// The rings of the transmit queue.
    pub transmit: Ring<'a>,
    /// The buffers the device writes frames into.
    pub taken: Area<'a>,
    /// The buffers the driver writes frames into.
    pub given: Area<'a>,
}

impl<'a> NetDma<'a> {
    /// The region over `bytes`, which begin at physical address
    /// `physical`.
    ///
    /// Answers `None` for fewer than [`REGION_BYTES`] bytes and for a
    /// physical start that is not aligned to the descriptor table, which
    /// virtio 2.7 requires of the table and this layout puts first.
    #[must_use]
    pub fn new(bytes: &'a mut [u8], physical: u64) -> Option<NetDma<'a>> {
        let align = u64::try_from(DESCRIPTOR_TABLE_ALIGN).unwrap_or(1);
        if bytes.len() < REGION_BYTES || !physical.is_multiple_of(align) {
            return None;
        }
        let step = u64::try_from(QUEUE_BYTES).ok()?;
        let area = u64::try_from(AREA_BYTES).ok()?;
        let (receive, rest) = bytes.split_at_mut(QUEUE_BYTES);
        let (transmit, rest) = rest.split_at_mut(QUEUE_BYTES);
        let (taken, rest) = rest.split_at_mut(AREA_BYTES);
        let (given, _over) = rest.split_at_mut(AREA_BYTES);
        Some(NetDma {
            receive: Ring {
                bytes: receive,
                physical,
            },
            transmit: Ring {
                bytes: transmit,
                physical: physical.wrapping_add(step),
            },
            taken: Area {
                bytes: taken,
                physical: physical.wrapping_add(step.wrapping_mul(2)),
            },
            given: Area {
                bytes: given,
                physical: physical
                    .wrapping_add(step.wrapping_mul(2))
                    .wrapping_add(area),
            },
        })
    }

    /// Where the rings of both queues are, as `initialize` is told them.
    #[must_use]
    pub const fn queues(&self) -> Queues {
        Queues {
            receive: self.receive.rings(),
            transmit: self.transmit.rings(),
        }
    }

    /// Zeroes both queues, which the driver does before it tells the
    /// device where the rings are.
    pub fn clear(&mut self) {
        self.receive.clear();
        self.transmit.clear();
    }
}

/// The bytes from `from` to `to`, or none when the region is shorter.
fn region(bytes: &[u8], from: usize, to: usize) -> &[u8] {
    bytes.get(from..to).unwrap_or(&[])
}

/// The same, for writing.
fn region_mut(bytes: &mut [u8], from: usize, to: usize) -> &mut [u8] {
    bytes.get_mut(from..to).unwrap_or(&mut [])
}
