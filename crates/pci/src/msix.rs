// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The MSI-X capability: how many vectors a function has, where its table
//! and its pending bits are, and what one table entry holds.
//!
//! The layout is the *PCI Express Base Specification* 6.0, section 7.7.2:
//! the message control word at offset two, the table offset and its base
//! address register at four, the pending bit array and its register at
//! eight, and a sixteen-byte table entry of message address, message data,
//! and vector control.
//!
//! The entry is written into a slice the caller hands over rather than
//! into the table itself: the table is in the device's own window, which
//! this crate cannot reach, so the bytes are computed here and stored by
//! whoever holds the mapping.

use crate::address::Address;
use crate::capability::{ID_MSIX, fits};
use crate::error::PciError;
use crate::space::{ConfigSpace, read_u16, read_word, write_u16};

/// Number of bytes of the capability.
pub const CAPABILITY_LEN: u16 = 12;

/// Number of bytes of one table entry.
pub const ENTRY_LEN: usize = 16;

/// Offset of the message control word inside the capability.
const CONTROL: u16 = 2;

/// Offset of the table's register and offset inside the capability.
const TABLE: u16 = 4;

/// Offset of the pending bit array's register and offset.
const PENDING: u16 = 8;

/// Bits 10:0 of the message control word: the table size, less one.
const SIZE_MASK: u16 = 0x07FF;

/// Bit 14 of the message control word: every vector of the function is
/// masked, whatever its own control word says.
const FUNCTION_MASK: u16 = 1 << 14;

/// Bit 15 of the message control word: the function raises MSI-X.
const ENABLE: u16 = 1 << 15;

/// Bits 2:0 of the table and pending words: which base address register
/// the structure lies in.
const BAR_MASK: u32 = 0b111;

/// Bit 0 of the vector control word: this vector is masked.
const VECTOR_MASKED: u32 = 1 << 0;

/// Where one of the two structures of the capability lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    /// The base address register the structure lies in.
    pub bar: u8,
    /// How far into that register's range it starts.
    pub offset: u32,
}

/// What the capability of one function says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MsiX {
    /// Where the capability itself lies in the configuration space.
    pub capability: u16,
    /// How many vectors the function has.
    pub vectors: u16,
    /// Whether the function raises MSI-X.
    pub enabled: bool,
    /// Whether every vector of the function is masked.
    pub function_masked: bool,
    /// Where the table is.
    pub table: Location,
    /// Where the pending bits are.
    pub pending: Location,
}

/// One entry of the table, as the kernel's `interrupt_create_msi` answers
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Entry {
    /// The address the device writes to.
    pub address: u64,
    /// The word it writes there.
    pub data: u32,
    /// Whether this vector is masked.
    pub masked: bool,
}

/// The capability at `offset` of `address`.
///
/// # Errors
///
/// [`PciError::NotMsix`] when the capability carries another identifier;
/// [`PciError::Offset`] when it would leave the list; the errors of
/// [`crate::space::read_word`].
pub fn read(space: &impl ConfigSpace, address: Address, offset: u16) -> Result<MsiX, PciError> {
    fits(offset, CAPABILITY_LEN)?;
    let id = u8::try_from(read_word(space, address, offset)? & 0xFF).unwrap_or(0);
    if id != ID_MSIX {
        return Err(PciError::NotMsix(id));
    }
    let control = read_u16(space, address, offset.wrapping_add(CONTROL))?;
    let table = read_word(space, address, offset.wrapping_add(TABLE))?;
    let pending = read_word(space, address, offset.wrapping_add(PENDING))?;
    Ok(MsiX {
        capability: offset,
        vectors: (control & SIZE_MASK).saturating_add(1),
        enabled: control & ENABLE != 0,
        function_masked: control & FUNCTION_MASK != 0,
        table: location(table),
        pending: location(pending),
    })
}

/// Turns the delivery of MSI-X messages of this function on or off.
///
/// # Errors
///
/// The errors of [`crate::space::read_word`].
pub fn set_enabled(
    space: &mut impl ConfigSpace,
    address: Address,
    capability: &MsiX,
    enabled: bool,
) -> Result<(), PciError> {
    set_bit(space, address, capability, ENABLE, enabled)
}

/// Masks or unmasks every vector of this function at once.
///
/// # Errors
///
/// The errors of [`crate::space::read_word`].
pub fn set_function_mask(
    space: &mut impl ConfigSpace,
    address: Address,
    capability: &MsiX,
    masked: bool,
) -> Result<(), PciError> {
    set_bit(space, address, capability, FUNCTION_MASK, masked)
}

/// Writes `entry` into the sixteen bytes of a table entry.
///
/// # Errors
///
/// [`PciError::EntryTooShort`] for fewer than [`ENTRY_LEN`] bytes.
pub fn write_entry(bytes: &mut [u8], entry: &Entry) -> Result<(), PciError> {
    let len = bytes.len();
    let slot = bytes
        .get_mut(..ENTRY_LEN)
        .ok_or(PciError::EntryTooShort(len))?;
    let control = if entry.masked { VECTOR_MASKED } else { 0 };
    let words = [
        u32::try_from(entry.address & 0xFFFF_FFFF).unwrap_or(0),
        u32::try_from(entry.address.wrapping_shr(32) & 0xFFFF_FFFF).unwrap_or(0),
        entry.data,
        control,
    ];
    for (index, word) in words.iter().enumerate() {
        let at = index.saturating_mul(4);
        if let Some(place) = slot.get_mut(at..at.saturating_add(4)) {
            place.copy_from_slice(&word.to_le_bytes());
        }
    }
    Ok(())
}

/// The entry those sixteen bytes hold, which is what a test reads back.
///
/// # Errors
///
/// [`PciError::EntryTooShort`] for fewer than [`ENTRY_LEN`] bytes.
pub fn read_entry(bytes: &[u8]) -> Result<Entry, PciError> {
    let slot = bytes
        .get(..ENTRY_LEN)
        .ok_or(PciError::EntryTooShort(bytes.len()))?;
    let word = |at: usize| -> u32 {
        slot.get(at..at.saturating_add(4))
            .and_then(|bytes| bytes.first_chunk::<4>().copied())
            .map_or(0, u32::from_le_bytes)
    };
    Ok(Entry {
        address: u64::from(word(4)).wrapping_shl(32) | u64::from(word(0)),
        data: word(8),
        masked: word(12) & VECTOR_MASKED != 0,
    })
}

/// Sets or clears one bit of the message control word.
fn set_bit(
    space: &mut impl ConfigSpace,
    address: Address,
    capability: &MsiX,
    bit: u16,
    set: bool,
) -> Result<(), PciError> {
    let at = capability.capability.wrapping_add(CONTROL);
    let control = read_u16(space, address, at)?;
    let wanted = if set { control | bit } else { control & !bit };
    write_u16(space, address, at, wanted)
}

/// The register and the offset one of the two words names.
fn location(word: u32) -> Location {
    Location {
        bar: u8::try_from(word & BAR_MASK).unwrap_or(0),
        offset: word & !BAR_MASK,
    }
}
