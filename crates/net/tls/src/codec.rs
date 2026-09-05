// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Reading and writing the shapes TLS uses: fixed-width numbers, and
//! vectors whose length goes in front of them in one, two, or three bytes.
//!
//! Invariants: a reader hands out slices of its input and never copies; a
//! read that does not fit leaves the reader where it was; a writer either
//! writes a whole value or reports that the buffer is too small.

use crate::error::TlsError;

/// A cursor over bytes that arrived.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    /// What has not been read.
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    /// A reader over `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes }
    }

    /// Whether everything has been read.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// How much is left.
    #[must_use]
    pub const fn left(&self) -> usize {
        self.bytes.len()
    }

    /// What has not been read.
    #[must_use]
    pub const fn rest(&self) -> &'a [u8] {
        self.bytes
    }

    /// Consumes the reader, refusing anything left over.
    ///
    /// # Errors
    ///
    /// [`TlsError::Decode`] when bytes remain.
    pub const fn finish(self) -> Result<(), TlsError> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(TlsError::Decode)
        }
    }

    /// The next `count` bytes.
    ///
    /// # Errors
    ///
    /// [`TlsError::Decode`] when fewer than `count` are left.
    pub fn take(&mut self, count: usize) -> Result<&'a [u8], TlsError> {
        let (head, tail) = self.bytes.split_at_checked(count).ok_or(TlsError::Decode)?;
        self.bytes = tail;
        Ok(head)
    }

    /// The next byte.
    ///
    /// # Errors
    ///
    /// [`TlsError::Decode`] when the reader is empty.
    pub fn u8(&mut self) -> Result<u8, TlsError> {
        let (&byte, tail) = self.bytes.split_first().ok_or(TlsError::Decode)?;
        self.bytes = tail;
        Ok(byte)
    }

    /// The next two bytes as a number.
    ///
    /// # Errors
    ///
    /// [`TlsError::Decode`] when fewer than two are left.
    pub fn u16(&mut self) -> Result<u16, TlsError> {
        let bytes = self.take(2)?;
        let pair: [u8; 2] = bytes.try_into().map_err(|_| TlsError::Decode)?;
        Ok(u16::from_be_bytes(pair))
    }

    /// The next three bytes as a number, which is how a handshake message
    /// and a certificate list carry their length.
    ///
    /// # Errors
    ///
    /// [`TlsError::Decode`] when fewer than three are left.
    pub fn u24(&mut self) -> Result<usize, TlsError> {
        let bytes = self.take(3)?;
        let [high, middle, low]: [u8; 3] = bytes.try_into().map_err(|_| TlsError::Decode)?;
        Ok(usize::from(high) << 16 | usize::from(middle) << 8 | usize::from(low))
    }

    /// A vector whose length is one byte.
    ///
    /// # Errors
    ///
    /// [`TlsError::Decode`] when the length or the body does not fit.
    pub fn vector8(&mut self) -> Result<&'a [u8], TlsError> {
        let length = usize::from(self.u8()?);
        self.take(length)
    }

    /// A vector whose length is two bytes.
    ///
    /// # Errors
    ///
    /// [`TlsError::Decode`] when the length or the body does not fit.
    pub fn vector16(&mut self) -> Result<&'a [u8], TlsError> {
        let length = usize::from(self.u16()?);
        self.take(length)
    }

    /// A vector whose length is three bytes.
    ///
    /// # Errors
    ///
    /// [`TlsError::Decode`] when the length or the body does not fit.
    pub fn vector24(&mut self) -> Result<&'a [u8], TlsError> {
        let length = self.u24()?;
        self.take(length)
    }
}

/// A cursor that appends into a buffer the caller owns.
pub struct Writer<'a> {
    /// The buffer.
    buffer: &'a mut [u8],
    /// How much of it is used.
    used: usize,
}

impl<'a> Writer<'a> {
    /// A writer over `buffer`.
    #[must_use]
    pub const fn new(buffer: &'a mut [u8]) -> Writer<'a> {
        Writer { buffer, used: 0 }
    }

    /// How much has been written.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.used
    }

    /// Whether nothing has been written.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.used == 0
    }

    /// What has been written.
    #[must_use]
    pub fn written(&self) -> &[u8] {
        self.buffer.get(..self.used).unwrap_or(&[])
    }

    /// Appends one byte.
    ///
    /// # Errors
    ///
    /// [`TlsError::BufferTooSmall`] when it does not fit.
    pub fn u8(&mut self, value: u8) -> Result<(), TlsError> {
        let slot = self
            .buffer
            .get_mut(self.used)
            .ok_or(TlsError::BufferTooSmall)?;
        *slot = value;
        self.used = self.used.wrapping_add(1);
        Ok(())
    }

    /// Appends two bytes.
    ///
    /// # Errors
    ///
    /// [`TlsError::BufferTooSmall`] when they do not fit.
    pub fn u16(&mut self, value: u16) -> Result<(), TlsError> {
        self.bytes(&value.to_be_bytes())
    }

    /// Appends bytes.
    ///
    /// # Errors
    ///
    /// [`TlsError::BufferTooSmall`] when they do not fit.
    pub fn bytes(&mut self, values: &[u8]) -> Result<(), TlsError> {
        for value in values {
            self.u8(*value)?;
        }
        Ok(())
    }

    /// Writes what `body` writes, with its length in front of it in one
    /// byte.
    ///
    /// # Errors
    ///
    /// [`TlsError::BufferTooSmall`] when it does not fit, and
    /// [`TlsError::Encode`] when the body is longer than the length can
    /// say.
    pub fn vector8<F>(&mut self, body: F) -> Result<(), TlsError>
    where
        F: FnOnce(&mut Writer<'_>) -> Result<(), TlsError>,
    {
        self.vector(1, body)
    }

    /// Writes what `body` writes, with its length in front of it in two
    /// bytes.
    ///
    /// # Errors
    ///
    /// See [`Writer::vector8`].
    pub fn vector16<F>(&mut self, body: F) -> Result<(), TlsError>
    where
        F: FnOnce(&mut Writer<'_>) -> Result<(), TlsError>,
    {
        self.vector(2, body)
    }

    /// Writes what `body` writes, with its length in front of it in three
    /// bytes.
    ///
    /// # Errors
    ///
    /// See [`Writer::vector8`].
    pub fn vector24<F>(&mut self, body: F) -> Result<(), TlsError>
    where
        F: FnOnce(&mut Writer<'_>) -> Result<(), TlsError>,
    {
        self.vector(3, body)
    }

    /// The common part: reserve the length, write the body, fill the
    /// length in.
    fn vector<F>(&mut self, width: usize, body: F) -> Result<(), TlsError>
    where
        F: FnOnce(&mut Writer<'_>) -> Result<(), TlsError>,
    {
        let start = self.used;
        for _ in 0..width {
            self.u8(0)?;
        }
        let after_length = self.used;
        body(self)?;

        let length = self.used.saturating_sub(after_length);
        let limit = 1usize.checked_shl(u32::try_from(width.wrapping_mul(8)).unwrap_or(0));
        if limit.is_some_and(|limit| length >= limit) {
            return Err(TlsError::Encode);
        }
        for step in 0..width {
            let shift = width.saturating_sub(step).saturating_sub(1).wrapping_mul(8);
            let byte = u8::try_from(length.wrapping_shr(u32::try_from(shift).unwrap_or(0)) & 0xFF)
                .unwrap_or(0);
            let slot = self
                .buffer
                .get_mut(start.wrapping_add(step))
                .ok_or(TlsError::BufferTooSmall)?;
            *slot = byte;
        }
        Ok(())
    }
}
