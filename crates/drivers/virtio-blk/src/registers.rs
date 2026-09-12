// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The one thing this crate asks of the world: a register in and a
//! register out, named by the structure it belongs to.

/// One of the four structures a virtio device publishes through its PCI
/// capabilities (virtio 4.1.4).
///
/// The crate never computes an address: it names a structure and an
/// offset inside it, and where that lands is the adapter's business. The
/// three kinds a capability may also name — access through configuration
/// space, shared memory, and vendor data — are not here, because this
/// driver reads none of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Structure {
    /// The common configuration of virtio 4.1.4.3: status, features, and
    /// the queues.
    Common,
    /// The notification structure of virtio 4.1.4.4.
    Notify,
    /// The interrupt status of virtio 4.1.4.5.
    Isr,
    /// The configuration of the device type, which for a block device is
    /// virtio 5.2.4.
    Device,
}

/// How wide one access is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Width {
    /// One byte.
    B8,
    /// Two bytes.
    B16,
    /// Four bytes.
    B32,
    /// Eight bytes.
    B64,
}

impl Width {
    /// How many bytes the access moves.
    #[must_use]
    pub const fn bytes(self) -> u16 {
        match self {
            Width::B8 => 1,
            Width::B16 => 2,
            Width::B32 => 4,
            Width::B64 => 8,
        }
    }

    /// The value with every bit above this width cleared, which is what a
    /// register of this width can hold.
    #[must_use]
    pub const fn mask(self, value: u64) -> u64 {
        match self {
            Width::B8 => value & 0xFF,
            Width::B16 => value & 0xFFFF,
            Width::B32 => value & 0xFFFF_FFFF,
            Width::B64 => value,
        }
    }
}

/// The low byte of a register value, which is the whole of a
/// [`Width::B8`] register.
#[must_use]
#[expect(
    clippy::as_conversions,
    reason = "the mask is the truncation, so the cast drops nothing; one site rather than one per caller"
)]
pub const fn low_u8(value: u64) -> u8 {
    (value & 0xFF) as u8
}

/// The low two bytes of a register value, which is the whole of a
/// [`Width::B16`] register.
#[must_use]
#[expect(
    clippy::as_conversions,
    reason = "the mask is the truncation, so the cast drops nothing; one site rather than one per caller"
)]
pub const fn low_u16(value: u64) -> u16 {
    (value & 0xFFFF) as u16
}

/// Access to the structures of one device.
///
/// Every value travels as a `u64` and the width says how much of it is
/// the register, so that one pair of methods covers the four widths the
/// structures use. An implementation reads and writes exactly the bytes
/// `width` names, little-endian, as virtio 1.4 requires of every field of
/// these structures.
///
/// A read is `&self` because a device register is not memory that
/// behaves: the implementation reaches it volatile, and the one read with
/// a side effect — the interrupt status, which clears on read — is the
/// device's own doing and not a mutation of the driver's state.
pub trait Registers {
    /// Reads `width` bytes at `offset` of `structure`.
    fn read(&self, structure: Structure, offset: u16, width: Width) -> u64;

    /// Writes `width` bytes at `offset` of `structure`.
    fn write(&mut self, structure: Structure, offset: u16, width: Width, value: u64);
}
