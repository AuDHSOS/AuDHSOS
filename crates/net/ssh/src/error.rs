// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a buffer is not what the documents describe.

use core::fmt;

use crypto_rng::RngError;

/// What this crate found wrong with the bytes it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SshError {
    /// A cursor was asked for more bytes than it has left. The two
    /// numbers are what was wanted and what remains; the cursor did not
    /// move.
    OutOfBounds {
        /// How many bytes the operation needed.
        needed: usize,
        /// How many bytes were left.
        available: usize,
    },
    /// An `mpint` that is not in the form RFC 4251, section 5, states:
    /// zero written with bytes, or an unnecessary leading `00` or `ff`.
    Mpint,
    /// An unsigned value was wanted and the `mpint` is negative. Every
    /// `mpint` this client reads is a group element or a modulus.
    Negative,
    /// A name-list that is not US-ASCII, holds a null, or holds a name of
    /// no length — a list that begins, ends, or doubles a comma. A name
    /// given to the writer may hold no comma either, because the comma is
    /// what separates them.
    NameList,
    /// A `packet_length` field that names no packet: below the minimum of
    /// sixteen bytes, above what section 6.1 makes mandatory, or not a
    /// multiple of the block size.
    PacketLength(u32),
    /// A `padding_length` field outside its packet, or below the four
    /// bytes RFC 4253, section 6, requires.
    PaddingLength(u8),
    /// A payload longer than the 32768 bytes of RFC 4253, section 6.1.
    PayloadLength(usize),
    /// A cipher block size a packet cannot be padded to.
    BlockSize(usize),
    /// The generator had no bytes for the padding.
    Rng(RngError),
}

impl fmt::Display for SshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SshError::OutOfBounds { needed, available } => {
                write!(f, "{needed} bytes were needed and {available} are left")
            }
            SshError::Mpint => f.write_str("the mpint is not in canonical form"),
            SshError::Negative => f.write_str("the mpint is negative"),
            SshError::NameList => f.write_str("the name-list holds a name it may not hold"),
            SshError::PacketLength(length) => write!(f, "{length} is no packet length"),
            SshError::PaddingLength(length) => write!(f, "{length} bytes of padding do not fit"),
            SshError::PayloadLength(length) => write!(f, "a payload of {length} bytes is too long"),
            SshError::BlockSize(size) => write!(f, "{size} is no block size to pad to"),
            SshError::Rng(error) => write!(f, "the padding has no randomness: {error}"),
        }
    }
}
