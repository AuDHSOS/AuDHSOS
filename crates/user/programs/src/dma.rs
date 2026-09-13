// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The region the device reads and writes: the three rings of one
//! virtqueue and the slots one request occupies.
//!
//! The driver asks the memory server for the region like any other
//! program and calls `memory_info` on what comes back. A memory object is
//! one contiguous physical range (D-115), so an offset into the mapping is
//! the same offset into physical memory, and that one addition is the
//! whole of the address arithmetic here.
//!
//! The layout is fixed at compile time: the descriptor table, the
//! available ring, the used ring, and then the request slots, each at the
//! alignment virtio 2.7 asks for. [`REGION_BYTES`] is what it comes to.
//!
//! Invariant: every accessor answers a slice inside the region or an empty
//! one; nothing here computes an address the caller could pass on.

use core::sync::atomic::{Ordering, fence};

use driver_virtio_blk::blk::{Chain, Rings, Segment};
use driver_virtio_blk::config::SECTOR_LEN;
use driver_virtio_blk::request::HEADER_LEN;
use virtio_queue::memory::{
    AVAILABLE_RING_ALIGN, DESCRIPTOR_TABLE_ALIGN, QueueMemory, USED_RING_ALIGN,
    available_ring_bytes, descriptor_table_bytes, used_ring_bytes,
};

/// Descriptors of the request queue.
///
/// Three describe one request, so eight carry two of them with room over.
/// A power of two is what virtio 2.7 requires of a queue size.
pub const QUEUE_SIZE: u16 = 8;

/// Requests that can be in flight at once.
///
/// Two, because one sector at a time is what `fs-fat` asks for and a
/// second slot is what lets a request be built while one stands.
pub const SLOTS: usize = 2;

/// Bytes of data one slot carries: one sector (virtio 5.2.5).
#[expect(
    clippy::as_conversions,
    reason = "five hundred and twelve is the whole range of both types, in a const"
)]
pub const SECTOR_BYTES: usize = SECTOR_LEN as usize;

/// How far into a slot its sector begins. The header takes sixteen bytes
/// and the status byte one; the sector begins at the next multiple of the
/// descriptor alignment, which leaves the three parts in three different
/// sixteen-byte pieces.
const SECTOR_AT: usize = 32;

/// How far into a slot its status byte is.
#[expect(
    clippy::as_conversions,
    reason = "sixteen is the whole range of both types, in a const"
)]
const STATUS_AT: usize = HEADER_LEN as usize;

/// Bytes one request slot occupies: the header the device reads, the
/// status byte it writes, and the sector.
const SLOT_BYTES: usize = SECTOR_AT + SECTOR_BYTES;

/// Where the descriptor table begins.
const DESCRIPTORS_AT: usize = 0;

/// Where the available ring begins.
const AVAILABLE_AT: usize = align_up(
    DESCRIPTORS_AT + descriptor_table_bytes(QUEUE_SIZE),
    AVAILABLE_RING_ALIGN,
);

/// Where the used ring begins.
const USED_AT: usize = align_up(
    AVAILABLE_AT + available_ring_bytes(QUEUE_SIZE),
    USED_RING_ALIGN,
);

/// Where the request slots begin.
const SLOTS_AT: usize = align_up(
    USED_AT + used_ring_bytes(QUEUE_SIZE),
    DESCRIPTOR_TABLE_ALIGN,
);

/// Bytes the whole region occupies.
pub const REGION_BYTES: usize = SLOTS_AT + SLOT_BYTES * SLOTS;

/// `value` rounded up to a multiple of `align`.
const fn align_up(value: usize, align: usize) -> usize {
    value.next_multiple_of(align)
}

/// The region of one device, over the bytes of its mapping.
pub struct Dma<'a> {
    bytes: &'a mut [u8],
    physical: u64,
}

impl<'a> Dma<'a> {
    /// The region over `bytes`, which begin at physical address
    /// `physical`.
    ///
    /// Answers `None` for fewer than [`REGION_BYTES`] bytes and for a
    /// physical start that is not aligned to the descriptor table, which
    /// virtio 2.7 requires of the table and this layout puts first.
    #[must_use]
    pub fn new(bytes: &'a mut [u8], physical: u64) -> Option<Self> {
        let align = u64::try_from(DESCRIPTOR_TABLE_ALIGN).unwrap_or(1);
        let aligned = physical.is_multiple_of(align);
        (bytes.len() >= REGION_BYTES && aligned).then_some(Dma { bytes, physical })
    }

    /// Every byte of the region, which the driver zeroes before it tells
    /// the device where the rings are (virtio 2.7.10.1).
    #[must_use]
    pub fn whole(&mut self) -> &mut [u8] {
        region_mut(self.bytes, 0, REGION_BYTES)
    }

    /// The physical address of the byte at `offset`.
    fn address(&self, offset: usize) -> u64 {
        self.physical
            .wrapping_add(u64::try_from(offset).unwrap_or(0))
    }

    /// Where the three rings are, as `initialize` is told them.
    #[must_use]
    pub fn rings(&self) -> Rings {
        Rings {
            descriptor_table: self.address(DESCRIPTORS_AT),
            available_ring: self.address(AVAILABLE_AT),
            used_ring: self.address(USED_AT),
            size: QUEUE_SIZE,
        }
    }

    /// Where slot `slot` begins.
    const fn slot_at(slot: usize) -> usize {
        SLOTS_AT.saturating_add(SLOT_BYTES.saturating_mul(slot % SLOTS))
    }

    /// The three parts of slot `slot`, as `submit` is told them.
    ///
    /// `data` says whether the request carries a sector; a flush does not.
    #[must_use]
    pub fn chain(&self, slot: usize, data: bool) -> Chain {
        let at = Self::slot_at(slot);
        Chain {
            header: self.address(at),
            data: data.then(|| Segment {
                address: self.address(at.saturating_add(SECTOR_AT)),
                length: SECTOR_LEN,
            }),
            status: self.address(at.saturating_add(STATUS_AT)),
        }
    }

    /// The sector of slot `slot`, for the caller to fill or to read.
    #[must_use]
    pub fn sector(&mut self, slot: usize) -> &mut [u8] {
        let at = Self::slot_at(slot).saturating_add(SECTOR_AT);
        region_mut(self.bytes, at, at.saturating_add(SECTOR_BYTES))
    }

    /// The header of slot `slot`, for `Request::write_header`.
    #[must_use]
    pub fn header(&mut self, slot: usize) -> &mut [u8] {
        let at = Self::slot_at(slot);
        region_mut(self.bytes, at, at.saturating_add(STATUS_AT))
    }

    /// The status byte the device wrote into slot `slot`.
    #[must_use]
    pub fn status(&self, slot: usize) -> u8 {
        let at = Self::slot_at(slot).saturating_add(STATUS_AT);
        self.bytes.get(at).copied().unwrap_or(NO_STATUS)
    }

    /// Puts a byte no device writes into the status of slot `slot`, so
    /// that a status read after a request is one the device wrote for it.
    pub fn clear_status(&mut self, slot: usize) {
        let at = Self::slot_at(slot).saturating_add(STATUS_AT);
        if let Some(byte) = self.bytes.get_mut(at) {
            *byte = NO_STATUS;
        }
    }
}

impl QueueMemory for Dma<'_> {
    fn descriptor_table(&self) -> &[u8] {
        region(self.bytes, DESCRIPTORS_AT, AVAILABLE_AT)
    }

    fn descriptor_table_mut(&mut self) -> &mut [u8] {
        region_mut(self.bytes, DESCRIPTORS_AT, AVAILABLE_AT)
    }

    fn available_ring_mut(&mut self) -> &mut [u8] {
        region_mut(self.bytes, AVAILABLE_AT, USED_AT)
    }

    fn used_ring(&self) -> &[u8] {
        region(self.bytes, USED_AT, SLOTS_AT)
    }

    /// The ordering virtio 2.7.13.3.1 and 2.7.13.4.1 require.
    ///
    /// A processor fence is enough and a device fence is not required:
    /// the device is software on the processor this driver runs on, which
    /// is the case virtio 6 describes without `VIRTIO_F_ORDER_PLATFORM`.
    fn barrier(&self) {
        fence(Ordering::SeqCst);
    }
}

/// The byte a status holds before the device writes one. The three the
/// specification defines are 0, 1 and 2 (virtio 5.2.6), so this is none of
/// them and a request whose status still reads it was not answered.
pub const NO_STATUS: u8 = 0xFF;

/// The bytes from `from` to `to`, or none when the region is shorter.
fn region(bytes: &[u8], from: usize, to: usize) -> &[u8] {
    bytes.get(from..to).unwrap_or(&[])
}

/// The same, for writing.
fn region_mut(bytes: &mut [u8], from: usize, to: usize) -> &mut [u8] {
    bytes.get_mut(from..to).unwrap_or(&mut [])
}
