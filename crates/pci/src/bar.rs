// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The base address registers of a type-0 header: what a function decodes,
//! where, and how much of it.
//!
//! The layout and the probe are the *PCI Express Base Specification* 6.0,
//! section 7.5.1.2.1: bit 0 separates I/O from memory, bits 2:1 of a memory
//! register say whether it is 32 or 64 bits wide, bit 3 says whether it is
//! prefetchable, and the size is the lowest address bit that stays set
//! after all ones are written.
//!
//! Invariants: the probe reads only a type-0 header, because the six
//! registers are its own — a type-1 header carries two of them and the bus
//! numbers and windows of a bridge where the other four would be, and a
//! probe there would re-route everything behind that bridge; the probe
//! clears the decode bits of the command register before it writes,
//! restores every register it wrote, and restores the command register on
//! every path out, the ones that found nothing and the ones that failed
//! included; a 64-bit register consumes the register above it, and that one
//! is never decoded again; a `len` is a power of two, and a register whose
//! probe keeps no address bit decodes nothing.

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

    /// `true` when `len` bytes at `offset` lie inside the range.
    #[must_use]
    pub const fn holds(&self, offset: u64, len: u64) -> bool {
        match offset.checked_add(len) {
            Some(end) => end <= self.len,
            None => false,
        }
    }
}

/// Firmware-assigned BAR address; size requires exclusive probing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AssignedBar {
    /// Register index; a 64-bit address occupies two registers.
    pub index: u8,
    /// Address space and width.
    pub space: Space,
    /// Assigned base address.
    pub base: u64,
}

/// Read BAR addresses without writing configuration registers.
///
/// Zero and reserved encodings are omitted. A zero 32-bit BAR cannot be
/// distinguished from an unimplemented register without probing.
/// # Errors
/// Rejects non-endpoint headers, unreadable registers, and truncated 64-bit BARs.
pub fn assigned(
    space: &impl ConfigSpace,
    address: Address,
) -> Result<[Option<AssignedBar>; MAX_BARS], PciError> {
    let header_type = read_u8(space, address, HEADER_TYPE)?;
    if !matches!(Kind::of(header_type), Kind::Endpoint) {
        return Err(PciError::HeaderType(header_type));
    }
    let mut bars = [None; MAX_BARS];
    let mut index = 0_u8;
    while usize::from(index) < MAX_BARS {
        let original = read_word(space, address, offset_of(index)?)?;
        let mut upper = index;
        let value = if original == 0 {
            None
        } else if original & IO_SPACE != 0 {
            Some((Space::Io, u64::from(original & !IO_FLAGS)))
        } else {
            let prefetchable = original & PREFETCHABLE != 0;
            match original & MEMORY_TYPE {
                TYPE_32 => Some((
                    Space::Memory {
                        width: Width::Bits32,
                        prefetchable,
                    },
                    u64::from(original & !MEMORY_FLAGS),
                )),
                TYPE_64 => {
                    upper = index.saturating_add(1);
                    if usize::from(upper) >= MAX_BARS {
                        return Err(PciError::BarTruncated(index));
                    }
                    let high = read_word(space, address, offset_of(upper)?)?;
                    Some((
                        Space::Memory {
                            width: Width::Bits64,
                            prefetchable,
                        },
                        join(high, original & !MEMORY_FLAGS),
                    ))
                }
                _ => None,
            }
        };
        if let Some(slot) = bars.get_mut(usize::from(index)) {
            *slot = value.map(|(space, base)| AssignedBar { index, space, base });
        }
        index = upper.saturating_add(1);
    }
    Ok(bars)
}

/// Every base address register of `address` that decodes something.
///
/// The caller must quiesce the function and exclude concurrent drivers.
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
        let (bar, step) = decode(space, address, index)?;
        if let Some(slot) = bars.get_mut(usize::from(index)) {
            *slot = bar;
        }
        index = index.saturating_add(step);
    }
    Ok(bars)
}

/// The register at `index`, or `None` when it decodes nothing, and the
/// number of registers it takes.
fn decode(
    space: &mut impl ConfigSpace,
    address: Address,
    index: u8,
) -> Result<(Option<Bar>, u8), PciError> {
    let (original, probed) = size_of(space, address, offset_of(index)?)?;
    if probed == 0 {
        return Ok((None, 1));
    }
    if original & IO_SPACE != 0 {
        let bar = length(u64::from(probed & !IO_FLAGS)).map(|len| Bar {
            index,
            space: Space::Io,
            base: u64::from(original & !IO_FLAGS),
            len,
        });
        return Ok((bar, 1));
    }
    let prefetchable = original & PREFETCHABLE != 0;
    match original & MEMORY_TYPE {
        TYPE_32 => {
            let bar = length(u64::from(probed & !MEMORY_FLAGS)).map(|len| Bar {
                index,
                space: Space::Memory {
                    width: Width::Bits32,
                    prefetchable,
                },
                base: u64::from(original & !MEMORY_FLAGS),
                len,
            });
            Ok((bar, 1))
        }
        TYPE_64 => {
            let upper = index.saturating_add(1);
            if usize::from(upper) >= MAX_BARS {
                return Err(PciError::BarTruncated(index));
            }
            let (high_original, high_probed) = size_of(space, address, offset_of(upper)?)?;
            let bar = length(join(high_probed, probed & !MEMORY_FLAGS)).map(|len| Bar {
                index,
                space: Space::Memory {
                    width: Width::Bits64,
                    prefetchable,
                },
                base: join(high_original, original & !MEMORY_FLAGS),
                len,
            });
            Ok((bar, 2))
        }
        // Bits 2:1 of `01` named memory below one mebibyte in the first PCI
        // specification; every revision since reserves them.
        _ => Ok((None, 1)),
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

/// The size a probe with its flag bits cleared names: its lowest set bit,
/// or `None` for a probe without an address bit.
const fn length(mask: u64) -> Option<u64> {
    match mask.isolate_lowest_one() {
        0 => None,
        lowest => Some(lowest),
    }
}

/// One 64-bit value out of the upper and the lower register.
fn join(high: u32, low: u32) -> u64 {
    u64::from(high).wrapping_shl(32) | u64::from(low)
}
