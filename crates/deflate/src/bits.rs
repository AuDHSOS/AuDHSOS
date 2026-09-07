// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bits into bytes, and back out.
//!
//! RFC 1951, section 3.1.1, says how the two are packed: an element goes
//! into a byte starting at its least significant bit, and an element that
//! is not a Huffman code is written from its own least significant bit
//! first. A Huffman code is the exception — it is written from its most
//! significant bit — so the writer turns one round before it goes out and
//! the reader builds one up the same way.

use crate::Error;

/// Writes bits into a buffer the caller owns.
pub(crate) struct Writer<'a> {
    /// Where the bytes go.
    out: &'a mut [u8],
    /// How many bytes have been written.
    at: usize,
    /// The bits that have no byte yet, in the low end.
    held: u32,
    /// How many bits are held.
    count: u32,
}

impl<'a> Writer<'a> {
    /// A writer over an empty buffer.
    pub(crate) const fn new(out: &'a mut [u8]) -> Self {
        Self {
            out,
            at: 0,
            held: 0,
            count: 0,
        }
    }

    /// Writes `count` low bits of `value`, least significant first.
    pub(crate) fn bits(&mut self, value: u32, count: u32) -> Result<(), Error> {
        let mask = (1u32 << count.min(31)).saturating_sub(1);
        self.held |= (value & mask) << self.count.min(31);
        self.count = self.count.saturating_add(count);
        while self.count >= 8 {
            let byte = u8::try_from(self.held & 0xFF).unwrap_or(0);
            self.byte(byte)?;
            self.held >>= 8;
            self.count = self.count.saturating_sub(8);
        }
        Ok(())
    }

    /// Writes a Huffman code, most significant bit first.
    pub(crate) fn code(&mut self, code: u16, length: u8) -> Result<(), Error> {
        let mut turned = 0u32;
        for step in 0..u32::from(length) {
            let bit =
                (u32::from(code) >> (u32::from(length).saturating_sub(step).saturating_sub(1))) & 1;
            turned |= bit << step;
        }
        self.bits(turned, u32::from(length))
    }

    /// Fills the byte being written with zeroes, so that what follows
    /// starts on a byte of its own.
    pub(crate) fn align(&mut self) -> Result<(), Error> {
        if self.count > 0 {
            let missing = 8u32.saturating_sub(self.count);
            self.bits(0, missing)?;
        }
        Ok(())
    }

    /// Writes whole bytes, which only an aligned writer may do.
    pub(crate) fn bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        for byte in bytes {
            self.byte(*byte)?;
        }
        Ok(())
    }

    /// Ends the stream, and says how many bytes it took.
    pub(crate) fn finish(mut self) -> Result<usize, Error> {
        self.align()?;
        Ok(self.at)
    }

    /// Writes one byte.
    fn byte(&mut self, byte: u8) -> Result<(), Error> {
        let slot = self.out.get_mut(self.at).ok_or(Error::Output)?;
        *slot = byte;
        self.at = self.at.saturating_add(1);
        Ok(())
    }
}

/// Reads bits out of a buffer.
pub(crate) struct Reader<'a> {
    /// The bytes.
    input: &'a [u8],
    /// How many bytes have been taken.
    at: usize,
    /// The bits taken from the byte being read, in the low end.
    held: u32,
    /// How many bits are held.
    count: u32,
}

impl<'a> Reader<'a> {
    /// A reader over a stream.
    pub(crate) const fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            at: 0,
            held: 0,
            count: 0,
        }
    }

    /// Reads `count` bits, least significant first.
    pub(crate) fn bits(&mut self, count: u32) -> Result<u32, Error> {
        while self.count < count {
            let byte = *self.input.get(self.at).ok_or(Error::Input)?;
            self.at = self.at.saturating_add(1);
            self.held |= u32::from(byte) << self.count.min(31);
            self.count = self.count.saturating_add(8);
        }
        let mask = (1u32 << count.min(31)).saturating_sub(1);
        let value = self.held & mask;
        self.held >>= count.min(31);
        self.count = self.count.saturating_sub(count);
        Ok(value)
    }

    /// Reads one bit.
    pub(crate) fn bit(&mut self) -> Result<u32, Error> {
        self.bits(1)
    }

    /// Drops what is left of the byte being read.
    pub(crate) const fn align(&mut self) {
        self.held = 0;
        self.count = 0;
    }

    /// Takes `count` whole bytes, which only an aligned reader may do.
    pub(crate) fn bytes(&mut self, count: usize) -> Result<&'a [u8], Error> {
        let end = self.at.saturating_add(count);
        let taken = self.input.get(self.at..end).ok_or(Error::Input)?;
        self.at = end;
        Ok(taken)
    }

    /// Takes as many bytes as the array has room for, which is how a
    /// caller gets a length or a checksum without a case for a slice
    /// that came back the wrong size.
    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let mut out = [0u8; N];
        for (slot, byte) in out.iter_mut().zip(self.bytes(N)?) {
            *slot = *byte;
        }
        Ok(out)
    }
}
