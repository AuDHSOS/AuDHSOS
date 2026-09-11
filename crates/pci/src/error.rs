// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a read of the configuration space was refused.

use core::fmt;

/// What a function of this crate found wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PciError {
    /// A device number above the thirty-one a bus has.
    Device(u8),
    /// A function number above the seven a device has.
    Function(u8),
    /// A window whose last bus is below its first.
    BusRange {
        /// The first bus the window names.
        first_bus: u8,
        /// The last bus it names.
        last_bus: u8,
    },
    /// An address on a bus the window does not cover.
    BusOutside {
        /// The bus the address names.
        bus: u8,
        /// The first bus the window covers.
        first_bus: u8,
        /// The last bus it covers.
        last_bus: u8,
    },
    /// An address in a segment group the window does not cover.
    Segment {
        /// The segment the address names.
        wanted: u16,
        /// The segment the window covers.
        window: u16,
    },
    /// An offset at or beyond the end of a function's configuration space.
    Offset(u16),
    /// An offset that is no whole word, which the mechanism reads in.
    Unaligned(u16),
    /// The configuration space refused a read at this offset.
    Unreadable(u16),
    /// A header type whose registers this crate does not lay out, where a
    /// type-0 header was needed.
    HeaderType(u8),
    /// A base address register index above the five a type-0 header has.
    BarIndex(u8),
    /// A 64-bit base address register in the last index, whose upper half
    /// would lie outside the header.
    BarTruncated(u8),
    /// A capability pointer inside the header, which holds no capability.
    CapabilityPointer(u8),
    /// A capability list longer than the space it lies in, which is a list
    /// that points back into itself.
    CapabilityLoop,
    /// A capability shorter than its own type needs.
    CapabilityLength {
        /// The capability identifier.
        id: u8,
        /// The length the capability announced.
        length: u8,
    },
    /// A capability that names a base address register a function has not.
    CapabilityBar(u8),
    /// A capability the caller read as MSI-X that carries another
    /// identifier.
    NotMsix(u8),
    /// A slice too short to hold a message table entry.
    EntryTooShort(usize),
}

impl fmt::Display for PciError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PciError::Device(device) => write!(f, "device {device} is not one a bus has"),
            PciError::Function(function) => {
                write!(f, "function {function} is not one a device has")
            }
            PciError::BusRange {
                first_bus,
                last_bus,
            } => write!(
                f,
                "the window covers buses {first_bus} to {last_bus}, which run backwards"
            ),
            PciError::BusOutside {
                bus,
                first_bus,
                last_bus,
            } => write!(
                f,
                "bus {bus} is outside the window's buses {first_bus} to {last_bus}"
            ),
            PciError::Segment { wanted, window } => {
                write!(f, "segment {wanted} is not the window's segment {window}")
            }
            PciError::Offset(offset) => {
                write!(f, "offset {offset:#x} is outside a configuration space")
            }
            PciError::Unaligned(offset) => write!(f, "offset {offset:#x} is no whole word"),
            PciError::Unreadable(offset) => {
                write!(f, "the configuration space answered nothing at {offset:#x}")
            }
            PciError::HeaderType(header_type) => write!(
                f,
                "the header type {header_type:#04x} is not the type-0 one this reads"
            ),
            PciError::BarIndex(index) => {
                write!(f, "base address register {index} is not one a header has")
            }
            PciError::BarTruncated(index) => write!(
                f,
                "base address register {index} is 64 bits wide and has no register above it"
            ),
            PciError::CapabilityPointer(pointer) => {
                write!(f, "a capability at {pointer:#x} would lie in the header")
            }
            PciError::CapabilityLoop => {
                f.write_str("the capability list is longer than the space it lies in")
            }
            PciError::CapabilityLength { id, length } => write!(
                f,
                "the capability {id:#x} announces {length} bytes, which is less than it needs"
            ),
            PciError::CapabilityBar(index) => write!(
                f,
                "a capability names base address register {index}, which a function has not"
            ),
            PciError::NotMsix(id) => write!(f, "the capability {id:#x} is not the msi-x one"),
            PciError::EntryTooShort(len) => {
                write!(f, "{len} bytes are too few for a message table entry")
            }
        }
    }
}
