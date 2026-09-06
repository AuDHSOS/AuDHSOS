// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The descriptor table: one descriptor's fields, and how a chain names
//! the next.
//!
//! `struct virtq_desc` and its three flags are virtio 1.4, section
//! 2.7.5.

use crate::error::{Area, QueueError};
use crate::memory::{DESCRIPTOR_BYTES, read_u16, read_u32, write_u16, write_u32, write_u64};

/// Descriptor flag: the chain goes on at [`Descriptor::next`]
/// (`VIRTQ_DESC_F_NEXT`, section 2.7.5).
pub const DESC_F_NEXT: u16 = 1 << 0;

/// Descriptor flag: the device writes this buffer instead of reading it
/// (`VIRTQ_DESC_F_WRITE`, section 2.7.5).
pub const DESC_F_WRITE: u16 = 1 << 1;

/// Descriptor flag: the buffer holds a table of further descriptors
/// (`VIRTQ_DESC_F_INDIRECT`, section 2.7.5). This crate never sets it and
/// never negotiates `VIRTIO_F_INDIRECT_DESC`, which is what gives it a
/// meaning (D-52).
pub const DESC_F_INDIRECT: u16 = 1 << 2;

/// Offset of the buffer address inside a descriptor.
const ADDRESS_AT: usize = 0;

/// Offset of the buffer length inside a descriptor.
const LENGTH_AT: usize = 8;

/// Offset of the flags inside a descriptor.
const FLAGS_AT: usize = 12;

/// Offset of the next index inside a descriptor.
const NEXT_AT: usize = 14;

/// Which way a buffer moves.
///
/// The order of the two is the order they have to appear in within a
/// chain — section 2.7.4.2 has the driver place every device-writable
/// element after every device-readable one — which is what makes [`Ord`]
/// here more than a formality.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Direction {
    /// The driver filled the buffer and the device reads it.
    DriverToDevice,
    /// The buffer is empty and the device writes it.
    DeviceToDriver,
}

impl Direction {
    /// The descriptor flag this direction carries.
    #[must_use]
    pub const fn flag(self) -> u16 {
        match self {
            Direction::DriverToDevice => 0,
            Direction::DeviceToDriver => DESC_F_WRITE,
        }
    }
}

/// One buffer of a request, as the caller describes it.
///
/// The address is the one the device uses, which on a system with an
/// IOMMU is not the one the driver dereferences. Which of the two it is
/// is settled above this crate; here it is a number that is written into
/// a descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Buffer {
    /// Address the device reads or writes.
    pub address: u64,
    /// Length in bytes.
    pub length: u32,
    /// Which way the bytes move.
    pub direction: Direction,
}

impl Buffer {
    /// A buffer the device reads.
    #[must_use]
    pub const fn to_device(address: u64, length: u32) -> Buffer {
        Buffer {
            address,
            length,
            direction: Direction::DriverToDevice,
        }
    }

    /// A buffer the device writes.
    #[must_use]
    pub const fn from_device(address: u64, length: u32) -> Buffer {
        Buffer {
            address,
            length,
            direction: Direction::DeviceToDriver,
        }
    }
}

/// One descriptor as it lies in the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Descriptor {
    /// Address the device reads or writes.
    pub address: u64,
    /// Length in bytes.
    pub length: u32,
    /// [`DESC_F_NEXT`] and [`DESC_F_WRITE`].
    pub flags: u16,
    /// The next descriptor of the chain, meaningful only with
    /// [`DESC_F_NEXT`].
    pub next: u16,
}

impl Descriptor {
    /// Whether the chain goes on after this descriptor.
    #[must_use]
    pub const fn has_next(&self) -> bool {
        self.flags & DESC_F_NEXT != 0
    }

    /// The byte the descriptor `index` begins at.
    #[must_use]
    #[expect(clippy::as_conversions, reason = "widening a u16 in a const fn")]
    pub const fn offset(index: u16) -> usize {
        (index as usize).saturating_mul(DESCRIPTOR_BYTES)
    }

    /// Reads the flags and the next index of descriptor `index`.
    ///
    /// Walking a chain needs no more than these two, and reading no more
    /// keeps the walk to two accesses per descriptor.
    ///
    /// # Errors
    ///
    /// [`QueueError::Region`] when the table is shorter than the
    /// descriptor.
    pub(crate) fn link_of(table: &[u8], index: u16) -> Result<(u16, u16), QueueError> {
        let at = Descriptor::offset(index);
        let flags = read_u16(table, Area::DescriptorTable, at.saturating_add(FLAGS_AT))?;
        let next = read_u16(table, Area::DescriptorTable, at.saturating_add(NEXT_AT))?;
        Ok((flags, next))
    }

    /// Reads what one step of a chain walk needs: the length, the flags,
    /// and the next index.
    ///
    /// The length is in it because the walk adds up how many bytes the
    /// device was given to write, which is the only bound there is on
    /// what it may report having written.
    ///
    /// # Errors
    ///
    /// [`QueueError::Region`] when the table is shorter than the
    /// descriptor.
    pub(crate) fn step_of(table: &[u8], index: u16) -> Result<(u32, u16, u16), QueueError> {
        let at = Descriptor::offset(index);
        let length = read_u32(table, Area::DescriptorTable, at.saturating_add(LENGTH_AT))?;
        let (flags, next) = Descriptor::link_of(table, index)?;
        Ok((length, flags, next))
    }

    /// Reads descriptor `index` whole.
    ///
    /// # Errors
    ///
    /// [`QueueError::Region`] when the table is shorter than the
    /// descriptor.
    pub fn read(table: &[u8], index: u16) -> Result<Descriptor, QueueError> {
        let at = Descriptor::offset(index);
        let address = read_address(table, at)?;
        let length = read_u32(table, Area::DescriptorTable, at.saturating_add(LENGTH_AT))?;
        let (flags, next) = Descriptor::link_of(table, index)?;
        Ok(Descriptor {
            address,
            length,
            flags,
            next,
        })
    }

    /// Writes `self` into descriptor `index`.
    ///
    /// # Errors
    ///
    /// [`QueueError::Region`] when the table is shorter than the
    /// descriptor.
    pub fn write(&self, table: &mut [u8], index: u16) -> Result<(), QueueError> {
        let at = Descriptor::offset(index);
        write_u64(
            table,
            Area::DescriptorTable,
            at.saturating_add(ADDRESS_AT),
            self.address,
        )?;
        write_u32(
            table,
            Area::DescriptorTable,
            at.saturating_add(LENGTH_AT),
            self.length,
        )?;
        write_u16(
            table,
            Area::DescriptorTable,
            at.saturating_add(FLAGS_AT),
            self.flags,
        )?;
        write_u16(
            table,
            Area::DescriptorTable,
            at.saturating_add(NEXT_AT),
            self.next,
        )
    }

    /// Sets [`DESC_F_NEXT`] on descriptor `index` and points it at
    /// `next`, leaving its address and length alone.
    ///
    /// # Errors
    ///
    /// [`QueueError::Region`] when the table is shorter than the
    /// descriptor.
    pub(crate) fn link_to(table: &mut [u8], index: u16, next: u16) -> Result<(), QueueError> {
        let at = Descriptor::offset(index);
        let (flags, _) = Descriptor::link_of(table, index)?;
        write_u16(
            table,
            Area::DescriptorTable,
            at.saturating_add(FLAGS_AT),
            flags | DESC_F_NEXT,
        )?;
        write_u16(
            table,
            Area::DescriptorTable,
            at.saturating_add(NEXT_AT),
            next,
        )
    }
}

/// Reads the eight address bytes of the descriptor beginning at `at`.
fn read_address(table: &[u8], at: usize) -> Result<u64, QueueError> {
    let given = table.len();
    let chunk = table
        .get(at.saturating_add(ADDRESS_AT)..)
        .and_then(|rest| rest.first_chunk::<8>())
        .ok_or(QueueError::Region {
            area: Area::DescriptorTable,
            needed: at.saturating_add(ADDRESS_AT).saturating_add(8),
            given,
        })?;
    Ok(u64::from_le_bytes(*chunk))
}
