// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a segment could not be read or written, and why an operation on a
//! connection could not be carried out.

use core::fmt;

use crypto_rng::RngError;
use net_wire::{Port, WireError};

use crate::state::State;

/// What this crate found wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TcpError {
    /// A cursor ran out of buffer.
    Wire(WireError),
    /// Fewer bytes arrived than the twenty a header takes. The value is
    /// how many there were.
    Short(usize),
    /// The data offset is below the five words a header takes, or names
    /// more bytes than arrived. The value is the field, in words.
    DataOffset(u8),
    /// The checksum does not verify. The value is the one the segment
    /// carried. TCP has no omitted form in either family.
    Checksum(u16),
    /// An option list that is not one: an option whose length is below the
    /// two bytes its kind and length take, or that reaches past the
    /// header. The value is the kind it was found at.
    BadOption(u8),
    /// The two addresses are not of one family, so there is no
    /// pseudo-header to sum over.
    MixedFamilies,
    /// The payload is longer than the buffer it was to be written into.
    TooLarge(usize),
    /// The operation is not one this state allows.
    WrongState(State),
    /// The peer reset the connection.
    Reset,
    /// A connection already holds that local port.
    PortInUse(Port),
    /// The table is full.
    NoConnection,
    /// The identifier names no open connection.
    UnknownConnection,
    /// The generator an initial sequence number was to be drawn from
    /// failed.
    Rng(RngError),
}

impl From<WireError> for TcpError {
    fn from(error: WireError) -> TcpError {
        TcpError::Wire(error)
    }
}

impl From<RngError> for TcpError {
    fn from(error: RngError) -> TcpError {
        TcpError::Rng(error)
    }
}

impl fmt::Display for TcpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TcpError::Wire(error) => error.fmt(f),
            TcpError::Short(len) => write!(f, "{len} bytes are fewer than a header takes"),
            TcpError::DataOffset(words) => {
                write!(
                    f,
                    "a data offset of {words} words is not one this segment has"
                )
            }
            TcpError::Checksum(carried) => {
                write!(f, "the checksum {carried:#06x} does not verify")
            }
            TcpError::BadOption(kind) => write!(f, "option {kind} is not a whole option"),
            TcpError::MixedFamilies => f.write_str("the two addresses are not of one family"),
            TcpError::TooLarge(len) => write!(f, "{len} bytes do not fit the buffer"),
            TcpError::WrongState(state) => write!(f, "{state} does not allow that"),
            TcpError::Reset => f.write_str("the peer reset the connection"),
            TcpError::PortInUse(port) => write!(f, "port {} is already bound", port.get()),
            TcpError::NoConnection => f.write_str("the connection table is full"),
            TcpError::UnknownConnection => f.write_str("that identifier names no open connection"),
            TcpError::Rng(error) => error.fmt(f),
        }
    }
}
