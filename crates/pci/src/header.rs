// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The header every function starts with: who made it, what it is, what it
//! decodes, and where its capabilities are.
//!
//! The layout is the *PCI Express Base Specification* 6.0, section 7.5.1,
//! the type-0 header.
//!
//! Invariant: a function whose vendor identifier reads as all ones is
//! absent, which is an answer and not an error; every other header is read
//! whole, and a header type this crate does not lay out is reported as
//! [`Kind::Other`] rather than read as a type-0 one.

use crate::address::Address;
use crate::error::PciError;
use crate::space::{ConfigSpace, read_u8, read_u16, read_word};

/// Offset of the vendor and device identifiers.
pub const VENDOR: u16 = 0x00;

/// Offset of the command register.
pub const COMMAND: u16 = 0x04;

/// Offset of the status register.
pub const STATUS: u16 = 0x06;

/// Offset of the revision, the programming interface, the subclass, and
/// the class.
pub const REVISION: u16 = 0x08;

/// Offset of the header type.
pub const HEADER_TYPE: u16 = 0x0E;

/// Offset of the first base address register.
pub const BAR0: u16 = 0x10;

/// Offset of the subsystem identifiers of a type-0 header.
pub const SUBSYSTEM: u16 = 0x2C;

/// Offset of the pointer to the first capability.
pub const CAPABILITY_POINTER: u16 = 0x34;

/// The vendor identifier an absent function answers with.
pub const ABSENT: u16 = 0xFFFF;

/// Command bit: the function decodes I/O space.
pub const COMMAND_IO: u16 = 1 << 0;

/// Command bit: the function decodes memory space.
pub const COMMAND_MEMORY: u16 = 1 << 1;

/// Command bit: the function may be a bus master, which a device that
/// reads and writes memory of its own accord needs.
pub const COMMAND_BUS_MASTER: u16 = 1 << 2;

/// Status bit: the function has a capability list.
pub const STATUS_CAPABILITIES: u16 = 1 << 4;

/// The bit of the header type that makes a device multi-function.
const MULTI_FUNCTION: u8 = 0x80;

/// What the rest of a header holds after the first sixteen bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A type-0 header: a device that is not a bridge.
    Endpoint,
    /// A type-1 header: a bridge to another bus, which this crate reports
    /// and does not descend into (D-112).
    Bridge,
    /// A header type this crate does not lay out.
    Other(u8),
}

impl Kind {
    /// The kind the low seven bits of a header type name.
    #[must_use]
    pub const fn of(header_type: u8) -> Kind {
        match header_type & !MULTI_FUNCTION {
            0 => Kind::Endpoint,
            1 => Kind::Bridge,
            other => Kind::Other(other),
        }
    }
}

/// The subsystem a type-0 header names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Subsystem {
    /// Who made the subsystem.
    pub vendor: u16,
    /// Which subsystem it is.
    pub device: u16,
}

/// What the header of one function says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Header {
    /// Who made the function.
    pub vendor: u16,
    /// Which device it is.
    pub device: u16,
    /// What the function decodes and may do.
    pub command: u16,
    /// What the function reports about itself.
    pub status: u16,
    /// The revision of the device.
    pub revision: u8,
    /// The programming interface inside the subclass.
    pub prog_if: u8,
    /// The subclass inside the class.
    pub subclass: u8,
    /// What kind of device it is.
    pub class: u8,
    /// The header type, multi-function bit included.
    pub header_type: u8,
    /// What the rest of the header holds.
    pub kind: Kind,
    /// The subsystem, for a type-0 header.
    pub subsystem: Option<Subsystem>,
    /// The offset of the first capability, or zero when the function has
    /// no capability list.
    pub capabilities: u8,
}

impl Header {
    /// `true` when the device has functions above function zero.
    #[must_use]
    pub const fn is_multi_function(&self) -> bool {
        self.header_type & MULTI_FUNCTION != 0
    }

    /// `true` when the function decodes memory space.
    #[must_use]
    pub const fn decodes_memory(&self) -> bool {
        self.command & COMMAND_MEMORY != 0
    }

    /// `true` when the function has a capability list to walk.
    #[must_use]
    pub const fn has_capabilities(&self) -> bool {
        self.status & STATUS_CAPABILITIES != 0 && self.capabilities != 0
    }
}

/// The header of `address`, or `None` for a function that is not there.
///
/// # Errors
///
/// The errors of [`crate::space::read_word`].
pub fn read(space: &impl ConfigSpace, address: Address) -> Result<Option<Header>, PciError> {
    let identity = read_word(space, address, VENDOR)?;
    let vendor = u16::try_from(identity & 0xFFFF).unwrap_or(ABSENT);
    if vendor == ABSENT {
        return Ok(None);
    }
    let device = u16::try_from(identity.wrapping_shr(16) & 0xFFFF).unwrap_or(0);
    let command = read_u16(space, address, COMMAND)?;
    let status = read_u16(space, address, STATUS)?;
    let classes = read_word(space, address, REVISION)?;
    let header_type = read_u8(space, address, HEADER_TYPE)?;
    let kind = Kind::of(header_type);
    let subsystem = match kind {
        Kind::Endpoint => Some(Subsystem {
            vendor: read_u16(space, address, SUBSYSTEM)?,
            device: read_u16(space, address, SUBSYSTEM.wrapping_add(2))?,
        }),
        Kind::Bridge | Kind::Other(_) => None,
    };
    let capabilities = if status & STATUS_CAPABILITIES == 0 {
        0
    } else {
        read_u8(space, address, CAPABILITY_POINTER)?
    };
    Ok(Some(Header {
        vendor,
        device,
        command,
        status,
        revision: field(classes, 0),
        prog_if: field(classes, 8),
        subclass: field(classes, 16),
        class: field(classes, 24),
        header_type,
        kind,
        subsystem,
        capabilities,
    }))
}

/// The command register of `address`.
///
/// # Errors
///
/// The errors of [`crate::space::read_word`].
pub fn read_command(space: &impl ConfigSpace, address: Address) -> Result<u16, PciError> {
    read_u16(space, address, COMMAND)
}

/// Writes the command register of `address`.
///
/// The status register shares the word with it and its error bits are
/// cleared by writing a one, so the upper half goes out as zero rather than
/// as what was read there: a write that carried those bits back would clear
/// every one the device had set, and a device that recorded an abort would
/// read as one that never did.
///
/// # Errors
///
/// None; the result is a `Result` because every write of this crate is.
pub fn write_command(
    space: &mut impl ConfigSpace,
    address: Address,
    value: u16,
) -> Result<(), PciError> {
    space.write_u32(address, COMMAND, u32::from(value));
    Ok(())
}

/// The byte of `word` that starts at bit `shift`.
fn field(word: u32, shift: u32) -> u8 {
    u8::try_from(word.wrapping_shr(shift) & 0xFF).unwrap_or(0)
}
