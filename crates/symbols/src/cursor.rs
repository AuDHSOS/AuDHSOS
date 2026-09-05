// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Reading numbers and strings out of a byte slice, one after another.
//!
//! Invariant: a read that would leave the slice fails and moves nothing, so
//! that a truncated structure is an error and never a panic.

use crate::error::SymbolError;

/// Number of value bits one byte of a variable-length number carries.
const PAYLOAD_BITS: u32 = 7;

/// The bit that says another byte follows.
const CONTINUE: u8 = 0x80;

/// The sign bit of the last byte of a signed variable-length number.
const SIGN: u8 = 0x40;

/// A position in a byte slice.
#[derive(Clone, Copy, Debug)]
pub struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    /// A cursor at the start of `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Cursor { bytes, position: 0 }
    }

    /// The offset the cursor stands at.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    /// Moves the cursor to `position`, which may be the end.
    ///
    /// # Errors
    ///
    /// [`SymbolError::Truncated`] if `position` is beyond the end.
    pub const fn seek(&mut self, position: usize) -> Result<(), SymbolError> {
        if position > self.bytes.len() {
            return Err(SymbolError::Truncated);
        }
        self.position = position;
        Ok(())
    }

    /// The number of bytes left.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    /// `true` if nothing is left.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// The next `len` bytes.
    ///
    /// # Errors
    ///
    /// [`SymbolError::Truncated`] if fewer than `len` bytes are left.
    pub fn take(&mut self, len: usize) -> Result<&'a [u8], SymbolError> {
        let end = self
            .position
            .checked_add(len)
            .ok_or(SymbolError::Truncated)?;
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or(SymbolError::Truncated)?;
        self.position = end;
        Ok(slice)
    }

    /// Skips `len` bytes.
    ///
    /// # Errors
    ///
    /// The errors of [`Cursor::take`].
    pub fn skip(&mut self, len: usize) -> Result<(), SymbolError> {
        self.take(len).map(|_| ())
    }

    /// The next byte.
    ///
    /// # Errors
    ///
    /// The errors of [`Cursor::take`].
    pub fn u8(&mut self) -> Result<u8, SymbolError> {
        let slice = self.take(1)?;
        slice.first().copied().ok_or(SymbolError::Truncated)
    }

    /// The next byte as a signed number.
    ///
    /// # Errors
    ///
    /// The errors of [`Cursor::take`].
    pub fn i8(&mut self) -> Result<i8, SymbolError> {
        Ok(i8::from_le_bytes([self.u8()?]))
    }

    /// The next little-endian `u16`.
    ///
    /// # Errors
    ///
    /// The errors of [`Cursor::take`].
    pub fn u16(&mut self) -> Result<u16, SymbolError> {
        let slice = self.take(2)?;
        let mut value = [0u8; 2];
        copy(&mut value, slice);
        Ok(u16::from_le_bytes(value))
    }

    /// The next little-endian `u32`.
    ///
    /// # Errors
    ///
    /// The errors of [`Cursor::take`].
    pub fn u32(&mut self) -> Result<u32, SymbolError> {
        let slice = self.take(4)?;
        let mut value = [0u8; 4];
        copy(&mut value, slice);
        Ok(u32::from_le_bytes(value))
    }

    /// The next little-endian `u64`.
    ///
    /// # Errors
    ///
    /// The errors of [`Cursor::take`].
    pub fn u64(&mut self) -> Result<u64, SymbolError> {
        let slice = self.take(8)?;
        let mut value = [0u8; 8];
        copy(&mut value, slice);
        Ok(u64::from_le_bytes(value))
    }

    /// The next `len` bytes as a little-endian number, for the widths
    /// DWARF uses for an address.
    ///
    /// # Errors
    ///
    /// The errors of [`Cursor::take`]; [`SymbolError::Overflow`] for a
    /// width above eight.
    pub fn unsigned(&mut self, len: usize) -> Result<u64, SymbolError> {
        if len > 8 {
            return Err(SymbolError::Overflow);
        }
        let slice = self.take(len)?;
        let mut value = [0u8; 8];
        copy(&mut value, slice);
        Ok(u64::from_le_bytes(value))
    }

    /// The next unsigned variable-length number.
    ///
    /// # Errors
    ///
    /// The errors of [`Cursor::take`]; [`SymbolError::Overflow`] if the
    /// number needs more than 64 bits.
    pub fn uleb(&mut self) -> Result<u64, SymbolError> {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = self.u8()?;
            let payload = u64::from(byte & !CONTINUE);
            if shift >= 64 {
                if payload != 0 {
                    return Err(SymbolError::Overflow);
                }
            } else {
                value |= payload.checked_shl(shift).ok_or(SymbolError::Overflow)?;
            }
            if byte & CONTINUE == 0 {
                return Ok(value);
            }
            shift = shift.saturating_add(PAYLOAD_BITS);
        }
    }

    /// The next unsigned variable-length number, as an offset.
    ///
    /// # Errors
    ///
    /// The errors of [`Cursor::uleb`]; [`SymbolError::Overflow`] if the
    /// number is larger than an offset.
    pub fn uleb_usize(&mut self) -> Result<usize, SymbolError> {
        usize::try_from(self.uleb()?).map_err(|_| SymbolError::Overflow)
    }

    /// The next signed variable-length number.
    ///
    /// # Errors
    ///
    /// The errors of [`Cursor::take`]; [`SymbolError::Overflow`] if the
    /// number needs more than 64 bits.
    pub fn sleb(&mut self) -> Result<i64, SymbolError> {
        let mut value = 0i64;
        let mut shift = 0u32;
        loop {
            let byte = self.u8()?;
            let payload = i64::from(byte & !CONTINUE);
            if shift < 64 {
                value |= payload.checked_shl(shift).ok_or(SymbolError::Overflow)?;
            } else if payload != 0 && payload != i64::from(!CONTINUE) {
                return Err(SymbolError::Overflow);
            }
            shift = shift.saturating_add(PAYLOAD_BITS);
            if byte & CONTINUE == 0 {
                if shift < 64 && byte & SIGN != 0 {
                    let mask = (-1i64).checked_shl(shift).unwrap_or(0);
                    value |= mask;
                }
                return Ok(value);
            }
        }
    }

    /// The next zero-terminated string.
    ///
    /// # Errors
    ///
    /// [`SymbolError::Truncated`] if no terminator follows.
    pub fn string(&mut self) -> Result<&'a str, SymbolError> {
        let rest = self
            .bytes
            .get(self.position..)
            .ok_or(SymbolError::Truncated)?;
        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(SymbolError::Truncated)?;
        let text = rest.get(..end).ok_or(SymbolError::Truncated)?;
        self.position = self
            .position
            .checked_add(end)
            .and_then(|p| p.checked_add(1))
            .ok_or(SymbolError::Truncated)?;
        core::str::from_utf8(text).map_err(|_| SymbolError::Truncated)
    }
}

/// Copies `source` into the low bytes of `value`.
fn copy(value: &mut [u8], source: &[u8]) {
    for (slot, byte) in value.iter_mut().zip(source) {
        *slot = *byte;
    }
}
