// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The base address registers of a type-0 header: what a function decodes,
//! where, and how much of it.
//!
//! The layout and the probe are the *PCI Express Base Specification* 6.0,
//! section 7.5.1.2.1: bit 0 separates I/O from memory, bits 2:1 of a memory
//! register say whether it is 32 or 64 bits wide, bit 3 says whether it is
//! prefetchable, and the size follows from writing all ones and reading
//! back what stayed clear.
//!
//! Invariants: the probe reads only a type-0 header, because the six
//! registers are its own — a type-1 header carries two of them and the bus
//! numbers and windows of a bridge where the other four would be, and a
//! probe there would re-route everything behind that bridge; the probe
//! clears the decode bits of the command register before it writes,
//! restores every register it wrote, and restores the command register on
//! every path out, the ones that found nothing and the ones that failed
//! included; a 64-bit register consumes the register above it, and that one
//! is never decoded again.

use crate::address::Address;
use crate::error::PciError;
use crate::header::{
    BAR0, COMMAND_IO, COMMAND_MEMORY, HEADER_TYPE, Kind, read_command, write_command,
};
use crate::space::{ConfigSpace, read_u8, read_word};

/// Number of base address registers a type-0 header has.
pub const MAX_BARS: usize = 6;

/// The value a probe writes: everything the register will take.
const PROBE: u32 = 0xFFFF_FFFF;

/// Bit 0 of a register: set for I/O, clear for memory.
const IO_SPACE: u32 = 1 << 0;

/// Bits 2:1 of a memory register, which say how wide it is.
const MEMORY_TYPE: u32 = 0b110;

/// Bits 2:1 of a 32-bit memory register.
const TYPE_32: u32 = 0b000;

/// Bits 2:1 of a 64-bit memory register.
const TYPE_64: u32 = 0b100;

/// Bit 3 of a memory register: the device tolerates a read ahead of use.
const PREFETCHABLE: u32 = 1 << 3;

/// The bits of a memory register that are not an address.
const MEMORY_FLAGS: u32 = 0xF;

/// The bits of an I/O register that are not an address.
const IO_FLAGS: u32 = 0x3;

/// How wide a memory register is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Width {
    /// One register.
    Bits32,
    /// This register and the one above it.
    Bits64,
}

/// What a register decodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Space {
    /// Memory.
    Memory {
        /// How wide the register is.
        width: Width,
        /// Whether the device tolerates a read ahead of use.
        prefetchable: bool,
    },
    /// I/O ports.
    Io,
}

/// One base address register that decodes something.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Bar {
    /// Which register it is.
    pub index: u8,
    /// What it decodes.
    pub space: Space,
    /// Where the range starts, as the firmware programmed it.
    pub base: u64,
    /// How many bytes it covers.
    pub len: u64,
}

impl Bar {
    /// `true` when the register decodes memory.
    #[must_use]
    pub const fn is_memory(&self) -> bool {
        matches!(self.space, Space::Memory { .. })
    }

    /// Number of registers this one takes.
    #[must_use]
    pub const fn registers(&self) -> u8 {
        match self.space {
            Space::Memory {
                width: Width::Bits64,
                ..
            } => 2,
            Space::Memory { .. } | Space::Io => 1,
        }
    }
}

/// Every base address register of `address` that decodes something.
///
/// Probing writes, so the memory and I/O decode bits of the command
/// register are cleared first and restored afterwards: while a register
/// holds all ones the function would answer at an address that is not its
/// own.
///
/// # Errors
///
/// [`PciError::HeaderType`] for a function whose header is not a type-0
/// one, which is refused before anything is written;
/// [`PciError::BarTruncated`] for a 64-bit register in the last index; the
/// errors of [`crate::space::read_word`]. The command register is restored
/// before any of them leaves this function.
pub fn probe(
    space: &mut impl ConfigSpace,
    address: Address,
) -> Result<[Option<Bar>; MAX_BARS], PciError> {
    let header_type = read_u8(space, address, HEADER_TYPE)?;
    if !matches!(Kind::of(header_type), Kind::Endpoint) {
        return Err(PciError::HeaderType(header_type));
    }
    let command = read_command(space, address)?;
    let quiet = command & !(COMMAND_MEMORY | COMMAND_IO);
    write_command(space, address, quiet)?;
    let found = decode_all(space, address);
    let restored = write_command(space, address, command);
    let bars = found?;
    restored?;
    Ok(bars)
}

/// Decodes every register, with the decode bits already cleared.
fn decode_all(
    space: &mut impl ConfigSpace,
    address: Address,
) -> Result<[Option<Bar>; MAX_BARS], PciError> {
    let mut bars = [None; MAX_BARS];
    let mut index = 0u8;
    while usize::from(index) < MAX_BARS {
        let bar = decode(space, address, index)?;
        let step = bar.map_or(1, |bar| bar.registers());
        if let Some(slot) = bars.get_mut(usize::from(index)) {
            *slot = bar;
        }
        index = index.saturating_add(step);
    }
    Ok(bars)
}

/// The register at `index`, or `None` when it decodes nothing.
fn decode(
    space: &mut impl ConfigSpace,
    address: Address,
    index: u8,
) -> Result<Option<Bar>, PciError> {
    let (original, probed) = size_of(space, address, offset_of(index)?)?;
    if probed == 0 {
        return Ok(None);
    }
    if original & IO_SPACE != 0 {
        return Ok(Some(Bar {
            index,
            space: Space::Io,
            base: u64::from(original & !IO_FLAGS),
            len: length(probed & !IO_FLAGS),
        }));
    }
    let prefetchable = original & PREFETCHABLE != 0;
    match original & MEMORY_TYPE {
        TYPE_32 => Ok(Some(Bar {
            index,
            space: Space::Memory {
                width: Width::Bits32,
                prefetchable,
            },
            base: u64::from(original & !MEMORY_FLAGS),
            len: length(probed & !MEMORY_FLAGS),
        })),
        TYPE_64 => {
            let upper = index.saturating_add(1);
            if usize::from(upper) >= MAX_BARS {
                return Err(PciError::BarTruncated(index));
            }
            let (high_original, high_probed) = size_of(space, address, offset_of(upper)?)?;
            Ok(Some(Bar {
                index,
                space: Space::Memory {
                    width: Width::Bits64,
                    prefetchable,
                },
                base: join(high_original, original & !MEMORY_FLAGS),
                len: wide_length(join(high_probed, probed & !MEMORY_FLAGS)),
            }))
        }
        // Bits 2:1 of `01` named the registers below one mebibyte of the
        // machines the first PCI specification described, and every
        // revision since reserves the encoding. A register that carries it
        // decodes nothing this system can map.
        _ => Ok(None),
    }
}

/// The value of the register at `offset` and what stays set when all ones
/// are written to it. The register is left as it was found.
fn size_of(
    space: &mut impl ConfigSpace,
    address: Address,
    offset: u16,
) -> Result<(u32, u32), PciError> {
    let original = read_word(space, address, offset)?;
    space.write_u32(address, offset, PROBE);
    let probed = read_word(space, address, offset);
    space.write_u32(address, offset, original);
    Ok((original, probed?))
}

/// The offset of the register at `index`.
fn offset_of(index: u8) -> Result<u16, PciError> {
    if usize::from(index) >= MAX_BARS {
        return Err(PciError::BarIndex(index));
    }
    Ok(BAR0.saturating_add(u16::from(index).saturating_mul(4)))
}

/// The length a 32-bit probe with its flag bits cleared names.
fn length(mask: u32) -> u64 {
    u64::from(!mask).saturating_add(1)
}

/// The length a 64-bit probe with its flag bits cleared names.
const fn wide_length(mask: u64) -> u64 {
    (!mask).saturating_add(1)
}

/// One 64-bit value out of the upper and the lower register.
fn join(high: u32, low: u32) -> u64 {
    u64::from(high).wrapping_shl(32) | u64::from(low)
}
