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
//! 2.7 asks for. The device-written pieces remain raw pointers; the driver
//! borrows only the descriptor tables, available rings and transmit area.
//! [`REGION_BYTES`] is what it comes to.
//!
//! Invariant: every access stays inside its disjoint piece.

use core::sync::atomic::{Ordering, fence};

use driver_virtio_net::frames::Frames;
use driver_virtio_net::queues::{Queues, Rings};
use virtio_queue::error::{Area as QueueArea, QueueError};
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

/// `value` rounded up to a multiple of `align`.
const fn align_up(value: usize, align: usize) -> usize {
    value.next_multiple_of(align)
}

/// The three rings of one queue, over the bytes of its piece.
pub struct Ring<'a> {
    descriptors: &'a mut [u8],
    available: &'a mut [u8],
    used: *mut u8,
    /// The physical address the piece begins at.
    physical: u64,
}

impl Ring<'_> {
    fn check_used(at: usize, width: usize) -> Result<(), QueueError> {
        let len = used_ring_bytes(QUEUE_SIZE);
        if at.checked_add(width).is_none_or(|end| end > len) {
            Err(QueueError::Region {
                area: QueueArea::UsedRing,
                needed: at.saturating_add(width),
                given: len,
            })
        } else {
            Ok(())
        }
    }

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
        self.descriptors.fill(0);
        self.available.fill(0);
        for at in 0..used_ring_bytes(QUEUE_SIZE) {
            let pointer = self.used.wrapping_add(at);
            // SAFETY: construction bounds the used ring; the device is not
            // configured when clear is called.
            unsafe { pointer.write_volatile(0) };
        }
    }
}

impl QueueMemory for Ring<'_> {
    fn descriptor_table(&self) -> &[u8] {
        self.descriptors
    }

    fn descriptor_table_mut(&mut self) -> &mut [u8] {
        self.descriptors
    }

    fn available_ring_mut(&mut self) -> &mut [u8] {
        self.available
    }

    fn read_used(&self, at: usize, out: &mut [u8]) -> Result<(), QueueError> {
        Self::check_used(at, out.len())?;
        for (offset, byte) in out.iter_mut().enumerate() {
            let pointer = self.used.wrapping_add(at.saturating_add(offset));
            // SAFETY: the range check bounds every offset; the mapping is live.
            *byte = unsafe { pointer.read_volatile() };
        }
        Ok(())
    }

    fn read_used_u16(&self, at: usize) -> Result<u16, QueueError> {
        Self::check_used(at, 2)?;
        if !at.is_multiple_of(2) {
            let mut bytes = [0; 2];
            self.read_used(at, &mut bytes)?;
            return Ok(u16::from_le_bytes(bytes));
        }
        // new checks base alignment; at and the width are checked.
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "base and offset alignment checked"
        )]
        let pointer = self.used.wrapping_add(at).cast::<u16>();
        // SAFETY: the pointer is aligned, in bounds and device-shared.
        Ok(u16::from_le(unsafe { pointer.read_volatile() }))
    }

    fn read_used_u32(&self, at: usize) -> Result<u32, QueueError> {
        Self::check_used(at, 4)?;
        if !at.is_multiple_of(4) {
            let mut bytes = [0; 4];
            self.read_used(at, &mut bytes)?;
            return Ok(u32::from_le_bytes(bytes));
        }
        // new checks base alignment; at and the width are checked.
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "base and offset alignment checked"
        )]
        let pointer = self.used.wrapping_add(at).cast::<u32>();
        // SAFETY: the pointer is aligned, in bounds and device-shared.
        Ok(u32::from_le(unsafe { pointer.read_volatile() }))
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
    bytes: AreaBytes<'a>,
    /// The physical address the area begins at.
    physical: u64,
}

enum AreaBytes<'a> {
    Shared(*const u8),
    Exclusive(&'a mut [u8]),
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

    fn copy(&self, index: u16, offset: usize, out: &mut [u8]) -> Option<()> {
        let at = Self::at(index)?;
        let end = offset.checked_add(out.len())?;
        if end > BUFFER_BYTES {
            return None;
        }
        match &self.bytes {
            AreaBytes::Shared(bytes) => {
                let start = at.checked_add(offset)?;
                for (step, byte) in out.iter_mut().enumerate() {
                    let pointer = bytes.wrapping_add(start.checked_add(step)?);
                    // SAFETY: at and end bound the access inside AREA_BYTES.
                    *byte = unsafe { pointer.read_volatile() };
                }
            }
            AreaBytes::Exclusive(bytes) => {
                out.copy_from_slice(bytes.get(at.checked_add(offset)?..at.checked_add(end)?)?);
            }
        }
        Some(())
    }

    fn bytes_mut(&mut self, index: u16) -> Option<&mut [u8]> {
        let at = Self::at(index)?;
        match &mut self.bytes {
            AreaBytes::Shared(_) => None,
            AreaBytes::Exclusive(bytes) => bytes.get_mut(at..at.checked_add(BUFFER_BYTES)?),
        }
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
    /// The region over `bytes`, which begin at physical address `physical`.
    ///
    /// Answers `None` for fewer than [`REGION_BYTES`] bytes and for a
    /// physical start that is not aligned to the descriptor table, which
    /// virtio 2.7 requires of the table and this layout puts first.
    /// # Safety
    ///
    /// `bytes` must point to a live, writable mapping of `len` bytes for
    /// the entire lifetime. The driver clears used rings before device
    /// configuration; afterward only the device writes the used rings and
    /// receive area. No other CPU access may alias
    /// the descriptor tables, available rings or transmit area.
    #[must_use]
    pub unsafe fn new(bytes: *mut u8, len: usize, physical: u64) -> Option<NetDma<'a>> {
        let align = u64::try_from(DESCRIPTOR_TABLE_ALIGN).unwrap_or(1);
        if bytes.is_null()
            || len < REGION_BYTES
            || len > isize::MAX.unsigned_abs()
            || !physical.is_multiple_of(align)
            || !bytes.addr().is_multiple_of(DESCRIPTOR_TABLE_ALIGN)
        {
            return None;
        }
        let step = u64::try_from(QUEUE_BYTES).ok()?;
        let area = u64::try_from(AREA_BYTES).ok()?;
        // SAFETY: the caller guarantees the mapping and exclusive
        // driver-written parts; the length check bounds both queues.
        let receive = unsafe { Self::ring(bytes, physical) };
        let transmit_at = bytes.wrapping_add(QUEUE_BYTES);
        // SAFETY: the second queue is disjoint from the first.
        let transmit = unsafe { Self::ring(transmit_at, physical.wrapping_add(step)) };
        let taken = AreaBytes::Shared(bytes.wrapping_add(2 * QUEUE_BYTES));
        let given_at = bytes.wrapping_add(2 * QUEUE_BYTES + AREA_BYTES);
        // SAFETY: the transmit area is disjoint and driver-written.
        let given =
            AreaBytes::Exclusive(unsafe { core::slice::from_raw_parts_mut(given_at, AREA_BYTES) });
        Some(NetDma {
            receive,
            transmit,
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

    const unsafe fn ring(bytes: *mut u8, physical: u64) -> Ring<'a> {
        // SAFETY: new bounds and reserves the descriptor table.
        let descriptors = unsafe { core::slice::from_raw_parts_mut(bytes, AVAILABLE_AT) };
        let available_at = bytes.wrapping_add(AVAILABLE_AT);
        // SAFETY: the available ring is disjoint from the descriptor table.
        let available =
            unsafe { core::slice::from_raw_parts_mut(available_at, USED_AT - AVAILABLE_AT) };
        Ring {
            descriptors,
            available,
            used: bytes.wrapping_add(USED_AT),
            physical,
        }
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
