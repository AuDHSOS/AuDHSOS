// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A reader and a writer over a byte buffer, in the byte order of every
//! internet header.
//!
//! Invariants: a cursor holds the whole buffer and the position it has
//! reached; every operation either completes and advances the position by
//! exactly what it took, or fails and leaves both the position and the
//! buffer as they were. A partial write is what would make a header half
//! valid, so there is none: [`Writer::write_u32`] into three free bytes
//! writes nothing at all.
//!
//! What [`Reader`] hands out borrows the buffer it was built over, so a
//! header is read where the frame landed and nothing is copied.

use core::fmt;

use crate::addr::{Ipv4Addr, MacAddr, Port};
use crate::error::WireError;

/// A reader over a frame or over one header inside it.
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

    /// What has not been read, without consuming it. This is how a layer
    /// hands its payload to the one above it.
    #[must_use]
    pub fn rest(&self) -> &'a [u8] {
        self.bytes.get(self.position..).unwrap_or(&[])
    }

    /// The next `len` bytes.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than `len` bytes are left.
    /// The position does not move.
    pub fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], WireError> {
        let end = self
            .position
            .checked_add(len)
            .ok_or(WireError::OutOfBounds {
                needed: len,
                available: self.remaining(),
            })?;
        let taken = self
            .bytes
            .get(self.position..end)
            .ok_or(WireError::OutOfBounds {
                needed: len,
                available: self.remaining(),
            })?;
        self.position = end;
        Ok(taken)
    }

    /// The next `N` bytes as an array, for a caller that wants a value
    /// rather than a borrow.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than `N` bytes are left.
    pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let mut value = [0u8; N];
        // `read_bytes` returns exactly `N` bytes or nothing at all, so the
        // two slices are the same length and the copy cannot fail.
        value.copy_from_slice(self.read_bytes(N)?);
        Ok(value)
    }

    /// The next byte.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when the reader is empty.
    pub fn read_u8(&mut self) -> Result<u8, WireError> {
        Ok(u8::from_be_bytes(self.read_array::<1>()?))
    }

    /// The next two bytes, most significant first.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than two bytes are left.
    pub fn read_u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_be_bytes(self.read_array::<2>()?))
    }

    /// The next four bytes, most significant first.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than four bytes are left.
    pub fn read_u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_be_bytes(self.read_array::<4>()?))
    }

    /// The next six bytes as a hardware address.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than six bytes are left.
    pub fn read_mac(&mut self) -> Result<MacAddr, WireError> {
        Ok(MacAddr::new(self.read_array::<6>()?))
    }

    /// The next four bytes as an IPv4 address.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than four bytes are left.
    pub fn read_ipv4(&mut self) -> Result<Ipv4Addr, WireError> {
        Ok(Ipv4Addr::from_octets(self.read_array::<4>()?))
    }

    /// The next two bytes as a port.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than two bytes are left.
    pub fn read_port(&mut self) -> Result<Port, WireError> {
        Ok(Port::new(self.read_u16()?))
    }

    /// Passes `len` bytes over, which is how an IPv4 option field is
    /// stepped across without being interpreted.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than `len` bytes are left.
    /// The position does not move.
    pub fn skip(&mut self, len: usize) -> Result<(), WireError> {
        self.read_bytes(len).map(|_| ())
    }
}

/// A writer over the buffer a frame is being built in.
pub struct Writer<'a> {
    /// The whole buffer.
    bytes: &'a mut [u8],
    /// How much of it has been written.
    position: usize,
}

impl fmt::Debug for Writer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Writer")
            .field("capacity", &self.bytes.len())
            .field("position", &self.position)
            .finish()
    }
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

    /// How much room is left.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    /// What has been written so far. This is the slice a checksum is taken
    /// over once a header stands.
    #[must_use]
    pub fn written(&self) -> &[u8] {
        self.bytes.get(..self.position).unwrap_or(&[])
    }

    /// Consumes the writer and returns what was written, with the lifetime
    /// of the buffer rather than of the writer.
    #[must_use]
    pub fn finish(self) -> &'a [u8] {
        self.bytes.get(..self.position).unwrap_or(&[])
    }

    /// Writes `data`.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when there is not room for all of it.
    /// Nothing is written and the position does not move.
    pub fn write_bytes(&mut self, data: &[u8]) -> Result<(), WireError> {
        let available = self.remaining();
        let end = self
            .position
            .checked_add(data.len())
            .ok_or(WireError::OutOfBounds {
                needed: data.len(),
                available,
            })?;
        let room = self
            .bytes
            .get_mut(self.position..end)
            .ok_or(WireError::OutOfBounds {
                needed: data.len(),
                available,
            })?;
        room.copy_from_slice(data);
        self.position = end;
        Ok(())
    }

    /// Writes one byte.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when the buffer is full.
    pub fn write_u8(&mut self, value: u8) -> Result<(), WireError> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes two bytes, most significant first.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than two bytes are free.
    pub fn write_u16(&mut self, value: u16) -> Result<(), WireError> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes four bytes, most significant first.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than four bytes are free.
    pub fn write_u32(&mut self, value: u32) -> Result<(), WireError> {
        self.write_bytes(&value.to_be_bytes())
    }

    /// Writes a hardware address.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than six bytes are free.
    pub fn write_mac(&mut self, address: MacAddr) -> Result<(), WireError> {
        self.write_bytes(&address.octets())
    }

    /// Writes an IPv4 address.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than four bytes are free.
    pub fn write_ipv4(&mut self, address: Ipv4Addr) -> Result<(), WireError> {
        self.write_bytes(&address.octets())
    }

    /// Writes a port.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than two bytes are free.
    pub fn write_port(&mut self, port: Port) -> Result<(), WireError> {
        self.write_u16(port.get())
    }

    /// Writes `len` zero bytes, which is how a checksum field is left to
    /// be filled in once the rest of the header stands.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when fewer than `len` bytes are free.
    pub fn write_zeros(&mut self, len: usize) -> Result<(), WireError> {
        let available = self.remaining();
        let end = self
            .position
            .checked_add(len)
            .ok_or(WireError::OutOfBounds {
                needed: len,
                available,
            })?;
        let room = self
            .bytes
            .get_mut(self.position..end)
            .ok_or(WireError::OutOfBounds {
                needed: len,
                available,
            })?;
        room.fill(0);
        self.position = end;
        Ok(())
    }

    /// Puts two bytes at `at`, without moving the position.
    ///
    /// This is the one operation that writes backwards, and it exists for
    /// one reason: a checksum covers the header it sits in, so the field
    /// is written as zero, the header is finished, the sum is taken over
    /// [`written`](Writer::written), and the answer goes back into the
    /// field.
    ///
    /// # Errors
    ///
    /// [`WireError::OutOfBounds`] when the two bytes are not both inside
    /// what has been written.
    pub fn patch_u16(&mut self, at: usize, value: u16) -> Result<(), WireError> {
        let end = at.checked_add(2).ok_or(WireError::OutOfBounds {
            needed: 2,
            available: 0,
        })?;
        if end > self.position {
            return Err(WireError::OutOfBounds {
                needed: 2,
                available: self.position.saturating_sub(at),
            });
        }
        let capacity = self.bytes.len();
        let room = self.bytes.get_mut(at..end).ok_or(WireError::OutOfBounds {
            needed: 2,
            available: capacity.saturating_sub(at),
        })?;
        room.copy_from_slice(&value.to_be_bytes());
        Ok(())
    }
}
