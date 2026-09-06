// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A byte string of at most `N` bytes, held in an array.
//!
//! A name and a chunk of console text are both short byte strings a message
//! carries, and both have to live somewhere a program with no heap can put
//! them. The type is the same for both; what differs is the bound, which is
//! part of the protocol and not of the container.
//!
//! Invariant: a value that exists is at most `N` bytes long, so nothing that
//! carries one has to check again.

use crate::label::ProtoError;

/// A byte string of at most `N` bytes.
#[derive(Clone, Copy, Debug)]
pub struct Bytes<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Default for Bytes<N> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<const N: usize> Bytes<N> {
    /// A string of no bytes.
    #[must_use]
    pub const fn empty() -> Self {
        Bytes {
            bytes: [0; N],
            len: 0,
        }
    }

    /// The string over `source`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::TooLong`] for more than `N` bytes.
    pub fn new(source: &[u8]) -> Result<Self, ProtoError> {
        let mut value = Bytes::empty();
        let slot = value
            .bytes
            .get_mut(..source.len())
            .ok_or(ProtoError::TooLong {
                len: source.len(),
                capacity: N,
            })?;
        slot.copy_from_slice(source);
        value.len = source.len();
        Ok(value)
    }

    /// The bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or(&[])
    }

    /// How many bytes there are.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// `true` for a string of no bytes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// How many bytes a string of this kind holds.
    #[must_use]
    pub const fn capacity() -> usize {
        N
    }
}

impl<const N: usize> PartialEq for Bytes<N> {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl<const N: usize> Eq for Bytes<N> {}
