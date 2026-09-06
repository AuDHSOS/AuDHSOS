// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The memory a virtqueue occupies, and the seam through which this crate
//! reaches it.
//!
//! The sizes and alignments are the table of virtio 1.4, section 2.7.
//!
//! Invariants: every access is bounds-checked against the region it is in
//! and returns [`QueueError::Region`] rather than panicking; no access
//! leaves the region it was given.

use crate::error::{Area, QueueError};

/// The largest queue virtio allows, from the text under the table of
/// section 2.7: the size is always a power of two and at most 32768.
/// A `u16` bounds it from above on its own, 32768 being the largest power
/// of two it holds.
pub const MAX_QUEUE_SIZE: u16 = 32768;

/// Bytes of one descriptor: address, length, flags, next
/// (`struct virtq_desc`, section 2.7.5).
pub const DESCRIPTOR_BYTES: usize = 16;

/// Bytes of one used element: descriptor id and written length
/// (`struct virtq_used_elem`, section 2.7.8). Both are `le32`, the id
/// being that wide for padding and not for range.
pub const USED_ELEMENT_BYTES: usize = 8;

/// Bytes before the ring array of the available and the used ring: the
/// flags and the index.
pub const RING_HEADER_BYTES: usize = 4;

/// Bytes after the ring array of the available and the used ring. They
/// carry `used_event` and `avail_event`, which only `EVENT_IDX` gives a
/// meaning; the region has them either way, so an adapter that sizes by
/// these functions is right whether the feature is negotiated or not.
pub const EVENT_BYTES: usize = 2;

/// Byte alignment the descriptor table needs (section 2.7).
pub const DESCRIPTOR_TABLE_ALIGN: usize = 16;

/// Byte alignment the available ring needs (section 2.7).
pub const AVAILABLE_RING_ALIGN: usize = 2;

/// Byte alignment the used ring needs (section 2.7).
pub const USED_RING_ALIGN: usize = 4;

/// Bytes a descriptor table of `size` descriptors occupies.
#[must_use]
#[expect(clippy::as_conversions, reason = "widening a u16 in a const fn")]
pub const fn descriptor_table_bytes(size: u16) -> usize {
    (size as usize).saturating_mul(DESCRIPTOR_BYTES)
}

/// Bytes an available ring of `size` descriptors occupies.
#[must_use]
#[expect(clippy::as_conversions, reason = "widening a u16 in a const fn")]
pub const fn available_ring_bytes(size: u16) -> usize {
    RING_HEADER_BYTES
        .saturating_add((size as usize).saturating_mul(2))
        .saturating_add(EVENT_BYTES)
}

/// Bytes a used ring of `size` descriptors occupies.
#[must_use]
#[expect(clippy::as_conversions, reason = "widening a u16 in a const fn")]
pub const fn used_ring_bytes(size: u16) -> usize {
    RING_HEADER_BYTES
        .saturating_add((size as usize).saturating_mul(USED_ELEMENT_BYTES))
        .saturating_add(EVENT_BYTES)
}

/// The three regions a virtqueue occupies.
///
/// The driver writes the descriptor table and the available ring; the
/// device writes the used ring, which is why it has no mutable accessor.
/// The regions are ordinary memory the device reaches by bus master
/// access, not a register window: an implementation hands over the bytes
/// and this crate reads and writes the structures in them.
pub trait QueueMemory {
    /// The descriptor table, at least [`descriptor_table_bytes`] long.
    fn descriptor_table(&self) -> &[u8];

    /// The descriptor table for writing.
    fn descriptor_table_mut(&mut self) -> &mut [u8];

    /// The available ring for writing, at least
    /// [`available_ring_bytes`] long.
    fn available_ring_mut(&mut self) -> &mut [u8];

    /// The used ring, at least [`used_ring_bytes`] long.
    fn used_ring(&self) -> &[u8];

    /// Orders the accesses before this call against the accesses after
    /// it, as seen by the device.
    ///
    /// Three of the four places this is called are required by name.
    /// Section 2.7.13, steps 4 and 6, has the driver barrier after the
    /// ring entry and before the index that publishes it, and again
    /// after that index and before it reads the notification flag;
    /// sections 2.7.13.3.1 and 2.7.13.4.1 state both as MUST. The
    /// fourth is between the used index and the element it names, which
    /// section 2.7.14 does not name — it is thorough about the driver's
    /// writes and thin about its reads — but which is the same race the
    /// other way round: the device writes the element and then the
    /// index, so a driver that has seen the index and not the element
    /// reads what was there before.
    ///
    /// What the barrier costs is the adapter's business, and section 6
    /// says as much under `VIRTIO_F_ORDER_PLATFORM`: without it a device
    /// is software on a CPU like this one and a weaker form is enough,
    /// with it the platform's device barrier is required. So the default
    /// does nothing, which is what a host test wants.
    fn barrier(&self) {}
}

/// Reads a little-endian `u16` at `at`.
pub(crate) fn read_u16(region: &[u8], area: Area, at: usize) -> Result<u16, QueueError> {
    let given = region.len();
    let chunk = region
        .get(at..)
        .and_then(|rest| rest.first_chunk::<2>())
        .ok_or(QueueError::Region {
            area,
            needed: at.saturating_add(2),
            given,
        })?;
    Ok(u16::from_le_bytes(*chunk))
}

/// Reads a little-endian `u32` at `at`.
pub(crate) fn read_u32(region: &[u8], area: Area, at: usize) -> Result<u32, QueueError> {
    let given = region.len();
    let chunk = region
        .get(at..)
        .and_then(|rest| rest.first_chunk::<4>())
        .ok_or(QueueError::Region {
            area,
            needed: at.saturating_add(4),
            given,
        })?;
    Ok(u32::from_le_bytes(*chunk))
}

/// Writes a little-endian `u16` at `at`.
pub(crate) fn write_u16(
    region: &mut [u8],
    area: Area,
    at: usize,
    value: u16,
) -> Result<(), QueueError> {
    let given = region.len();
    let chunk = region
        .get_mut(at..)
        .and_then(|rest| rest.first_chunk_mut::<2>())
        .ok_or(QueueError::Region {
            area,
            needed: at.saturating_add(2),
            given,
        })?;
    *chunk = value.to_le_bytes();
    Ok(())
}

/// Writes a little-endian `u32` at `at`.
pub(crate) fn write_u32(
    region: &mut [u8],
    area: Area,
    at: usize,
    value: u32,
) -> Result<(), QueueError> {
    let given = region.len();
    let chunk = region
        .get_mut(at..)
        .and_then(|rest| rest.first_chunk_mut::<4>())
        .ok_or(QueueError::Region {
            area,
            needed: at.saturating_add(4),
            given,
        })?;
    *chunk = value.to_le_bytes();
    Ok(())
}

/// Writes a little-endian `u64` at `at`.
pub(crate) fn write_u64(
    region: &mut [u8],
    area: Area,
    at: usize,
    value: u64,
) -> Result<(), QueueError> {
    let given = region.len();
    let chunk = region
        .get_mut(at..)
        .and_then(|rest| rest.first_chunk_mut::<8>())
        .ok_or(QueueError::Region {
            area,
            needed: at.saturating_add(8),
            given,
        })?;
    *chunk = value.to_le_bytes();
    Ok(())
}
