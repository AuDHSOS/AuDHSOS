// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The types of RFC 4251, section 5, read and written over a buffer.
//!
//! Invariants: a cursor holds the whole buffer and the position it has
//! reached; a read either completes and advances the position by exactly
//! what it took, or fails and leaves the position where it was. A write
//! that does not fit writes nothing, so a half-written field never
//! reaches a packet.
//!
//! What [`Reader`] hands out borrows the buffer it was built over.

use core::str::Split;

use crate::error::SshError;

/// A reader over a packet payload, or over one field inside it.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    /// The whole buffer.
    bytes: &'a [u8],
    /// How much of it has been read.
    position: usize,
}

impl<'a> Reader<'a> {
    /// A reader over `bytes`, at position zero.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, position: 0 }
    }

    /// How many bytes have been read.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    /// How many bytes are left.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    /// Whether everything has been read.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// What has not been read, without consuming it.
    #[must_use]
    pub fn rest(&self) -> &'a [u8] {
        self.bytes.get(self.position..).unwrap_or(&[])
    }

    /// The next `len` bytes, which is `byte[n]` of section 5.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when fewer than `len` bytes are left.
    pub fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], SshError> {
        let end = self.position.saturating_add(len);
        let taken = self
            .bytes
            .get(self.position..end)
            .ok_or(SshError::OutOfBounds {
                needed: len,
                available: self.remaining(),
            })?;
        self.position = end;
        Ok(taken)
    }

    /// One `byte`.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when nothing is left.
    pub fn read_byte(&mut self) -> Result<u8, SshError> {
        let byte = self.bytes.get(self.position).copied().ok_or({
            SshError::OutOfBounds {
                needed: 1,
                available: 0,
            }
        })?;
        self.position = self.position.saturating_add(1);
        Ok(byte)
    }

    /// One `boolean`. Section 5 stores false as zero and true as one, and
    /// requires every other value to be read as true.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when nothing is left.
    pub fn read_boolean(&mut self) -> Result<bool, SshError> {
        Ok(self.read_byte()? != 0)
    }

    /// One `uint32`, most significant byte first.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when fewer than four bytes are left.
    pub fn read_u32(&mut self) -> Result<u32, SshError> {
        let bytes = self.read_bytes(4)?;
        let mut value = 0u32;
        for byte in bytes {
            value = value.wrapping_shl(8) | u32::from(*byte);
        }
        Ok(value)
    }

    /// One `uint64`, most significant byte first.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when fewer than eight bytes are left.
    pub fn read_u64(&mut self) -> Result<u64, SshError> {
        let bytes = self.read_bytes(8)?;
        let mut value = 0u64;
        for byte in bytes {
            value = value.wrapping_shl(8) | u64::from(*byte);
        }
        Ok(value)
    }

    /// One `string`: a `uint32` length and that many bytes, which may be
    /// any bytes at all.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when the length or the bytes it names are
    /// not there. The position does not move.
    pub fn read_string(&mut self) -> Result<&'a [u8], SshError> {
        let mut probe = self.clone();
        let len = usize::try_from(probe.read_u32()?).unwrap_or(usize::MAX);
        let value = probe.read_bytes(len)?;
        self.position = probe.position;
        Ok(value)
    }

    /// One `mpint`, checked against the form section 5 states.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when the string is not there,
    /// [`SshError::Mpint`] when it is not canonical. The position does not
    /// move.
    pub fn read_mpint(&mut self) -> Result<Mpint<'a>, SshError> {
        let mut probe = self.clone();
        let value = Mpint::new(probe.read_string()?)?;
        self.position = probe.position;
        Ok(value)
    }

    /// One `name-list`, checked against the rules section 5 states.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when the string is not there,
    /// [`SshError::NameList`] when a name is empty or not US-ASCII. The
    /// position does not move.
    pub fn read_name_list(&mut self) -> Result<NameList<'a>, SshError> {
        let mut probe = self.clone();
        let value = NameList::new(probe.read_string()?)?;
        self.position = probe.position;
        Ok(value)
    }
}

/// A writer over the buffer a packet is built in.
#[derive(Debug)]
pub struct Writer<'a> {
    /// The whole buffer.
    bytes: &'a mut [u8],
    /// How much of it has been written.
    position: usize,
}

impl<'a> Writer<'a> {
    /// A writer over `bytes`, at position zero.
    #[must_use]
    pub const fn new(bytes: &'a mut [u8]) -> Writer<'a> {
        Writer { bytes, position: 0 }
    }

    /// How many bytes have been written.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    /// How many bytes are free.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    /// What has been written.
    #[must_use]
    pub fn written(&self) -> &[u8] {
        self.bytes.get(..self.position).unwrap_or(&[])
    }

    /// Reserves `len` bytes and hands them out to be filled, which is how
    /// the padding of a packet is drawn straight into the frame.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when fewer than `len` bytes are free.
    pub fn take(&mut self, len: usize) -> Result<&mut [u8], SshError> {
        let end = self.position.saturating_add(len);
        let free = self.remaining();
        let slot = self
            .bytes
            .get_mut(self.position..end)
            .ok_or(SshError::OutOfBounds {
                needed: len,
                available: free,
            })?;
        self.position = end;
        Ok(slot)
    }

    /// Writes `bytes` as they are, which is `byte[n]` of section 5.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when they do not fit. Nothing is written.
    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), SshError> {
        let slot = self.take(bytes.len())?;
        slot.copy_from_slice(bytes);
        Ok(())
    }

    /// Writes one `byte`.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when the buffer is full.
    pub fn write_byte(&mut self, value: u8) -> Result<(), SshError> {
        self.write_bytes(&[value])
    }

    /// Writes one `boolean`. Section 5 forbids storing anything but zero
    /// and one, so that is what this writes.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when the buffer is full.
    pub fn write_boolean(&mut self, value: bool) -> Result<(), SshError> {
        self.write_byte(u8::from(value))
    }

    /// Writes one `uint32`, most significant byte first.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when four bytes do not fit.
    pub fn write_u32(&mut self, value: u32) -> Result<(), SshError> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes one `uint64`, most significant byte first.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when eight bytes do not fit.
    pub fn write_u64(&mut self, value: u64) -> Result<(), SshError> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes one `string`: the length and then the bytes.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when they do not fit. Nothing is written.
    pub fn write_string(&mut self, value: &[u8]) -> Result<(), SshError> {
        let len = u32::try_from(value.len()).unwrap_or(u32::MAX);
        let slot = self.take(value.len().saturating_add(4))?;
        let (length, bytes) = slot.split_at_mut(4);
        length.copy_from_slice(&len.to_be_bytes());
        bytes.copy_from_slice(value);
        Ok(())
    }

    /// Writes one `mpint`, which is canonical because [`Mpint`] is.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when it does not fit. Nothing is written.
    pub fn write_mpint(&mut self, value: Mpint<'_>) -> Result<(), SshError> {
        self.write_string(value.as_bytes())
    }

    /// Writes an unsigned big-endian value as an `mpint`: leading zero
    /// bytes come off, and one goes back on when the top bit is set.
    ///
    /// This is the encoding RFC 8731, section 3.1, requires of the shared
    /// secret, where a fixed-length string instead of an `mpint` is the
    /// mistake that fails one connection in two.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when it does not fit. Nothing is written.
    pub fn write_unsigned(&mut self, magnitude: &[u8]) -> Result<(), SshError> {
        let start = magnitude
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(magnitude.len());
        let value = magnitude.get(start..).unwrap_or(&[]);
        let pad = usize::from(value.first().is_some_and(|byte| byte & 0x80 != 0));
        let len = u32::try_from(value.len().saturating_add(pad)).unwrap_or(u32::MAX);
        let slot = self.take(value.len().saturating_add(pad).saturating_add(4))?;
        let (length, rest) = slot.split_at_mut(4);
        length.copy_from_slice(&len.to_be_bytes());
        let (zero, bytes) = rest.split_at_mut(pad);
        zero.fill(0);
        bytes.copy_from_slice(value);
        Ok(())
    }

    /// Writes one `name-list`, the names separated by commas.
    ///
    /// # Errors
    ///
    /// [`SshError::NameList`] when a name is empty, holds a comma, or is
    /// not US-ASCII; [`SshError::OutOfBounds`] when the list does not fit.
    /// Nothing is written in either case.
    pub fn write_name_list(&mut self, names: &[&str]) -> Result<(), SshError> {
        let mut len = 0usize;
        for name in names {
            if name.is_empty() || !name.is_ascii() || name.contains([',', '\0']) {
                return Err(SshError::NameList);
            }
            len = len.saturating_add(name.len());
        }
        let separators = names.len().saturating_sub(1);
        let len = len.saturating_add(separators);
        let value = u32::try_from(len).unwrap_or(u32::MAX);
        let slot = self.take(len.saturating_add(4))?;
        let (length, mut rest) = slot.split_at_mut(4);
        length.copy_from_slice(&value.to_be_bytes());
        for (index, name) in names.iter().enumerate() {
            if index != 0 {
                let (comma, tail) = rest.split_at_mut(1);
                comma.fill(b',');
                rest = tail;
            }
            let (slot, tail) = rest.split_at_mut(name.len());
            slot.copy_from_slice(name.as_bytes());
            rest = tail;
        }
        Ok(())
    }
}

/// A multiple precision integer in the form of RFC 4251, section 5: two's
/// complement, most significant byte first, no unnecessary leading byte,
/// and zero as no bytes at all.
///
/// The value is held as the bytes that were on the wire. Making one is
/// the only way to get one, so a value of this type is canonical.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Mpint<'a> {
    /// The data part of the string, without its length.
    data: &'a [u8],
}

impl<'a> Mpint<'a> {
    /// The value `data` holds.
    ///
    /// # Errors
    ///
    /// [`SshError::Mpint`] when `data` is not canonical: a single `00`,
    /// which is zero written with a byte; a `00` before a byte whose top
    /// bit is clear; or an `ff` before a byte whose top bit is set.
    pub const fn new(data: &'a [u8]) -> Result<Mpint<'a>, SshError> {
        match data {
            [0x00] => Err(SshError::Mpint),
            [0x00, next, ..] if *next < 0x80 => Err(SshError::Mpint),
            [0xFF, next, ..] if *next >= 0x80 => Err(SshError::Mpint),
            _ => Ok(Mpint { data }),
        }
    }

    /// The bytes, as they stand in the packet.
    #[must_use]
    pub const fn as_bytes(&self) -> &'a [u8] {
        self.data
    }

    /// Whether this is the value zero, which section 5 stores as a string
    /// of no bytes.
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.data.is_empty()
    }

    /// Whether the value is negative, which is the top bit of the first
    /// byte.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.data.first().is_some_and(|byte| byte & 0x80 != 0)
    }

    /// The value as unsigned big-endian bytes, without the zero byte a
    /// positive number wears when its top bit is set.
    ///
    /// # Errors
    ///
    /// [`SshError::Negative`] when the value is negative. Every `mpint`
    /// this client computes with is a group element, and a negative one is
    /// a peer doing something other than the key exchange.
    pub fn magnitude(&self) -> Result<&'a [u8], SshError> {
        if self.is_negative() {
            return Err(SshError::Negative);
        }
        Ok(self.data.strip_prefix(&[0x00]).unwrap_or(self.data))
    }
}

/// A comma-separated list of names, checked against RFC 4251, section 5.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NameList<'a> {
    /// The names, with their commas.
    names: &'a str,
}

impl<'a> NameList<'a> {
    /// The list `bytes` holds.
    ///
    /// # Errors
    ///
    /// [`SshError::NameList`] when a byte is not US-ASCII, when a null is
    /// in it, or when a name has no length — which is what a leading, a
    /// trailing, or a doubled comma means.
    pub fn new(bytes: &'a [u8]) -> Result<NameList<'a>, SshError> {
        let names = core::str::from_utf8(bytes).map_err(|_| SshError::NameList)?;
        if !names.is_ascii() || names.contains('\0') {
            return Err(SshError::NameList);
        }
        if !names.is_empty() && names.split(',').any(str::is_empty) {
            return Err(SshError::NameList);
        }
        Ok(NameList { names })
    }

    /// The list as it stands in the packet.
    #[must_use]
    pub const fn as_str(&self) -> &'a str {
        self.names
    }

    /// Whether the list holds no names.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// The names, in the order they were sent, which for the lists of
    /// RFC 4253, section 7.1, is the order of preference.
    #[must_use]
    pub fn iter(&self) -> Names<'a> {
        Names {
            inner: if self.names.is_empty() {
                None
            } else {
                Some(self.names.split(','))
            },
        }
    }

    /// Whether `name` is in the list.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.iter().any(|held| held == name)
    }
}

impl<'a> IntoIterator for &NameList<'a> {
    type Item = &'a str;
    type IntoIter = Names<'a>;

    fn into_iter(self) -> Names<'a> {
        self.iter()
    }
}

/// The names of a [`NameList`].
#[derive(Clone, Debug)]
pub struct Names<'a> {
    /// The split, or nothing at all for the empty list, which splits into
    /// one empty name rather than into none.
    inner: Option<Split<'a, char>>,
}

impl<'a> Iterator for Names<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        self.inner.as_mut()?.next()
    }
}
