// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a message could not be read or written, and why a resolution did
//! not finish.

use core::fmt;

use crypto_rng::RngError;
use net_udp::UdpError;
use net_wire::WireError;

use crate::message::ResponseCode;
use crate::record::RecordType;

/// What this crate found wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DnsError {
    /// A cursor ran out of buffer.
    Wire(WireError),
    /// The generator a transaction id was to be drawn from failed.
    Rng(RngError),
    /// The datagram around the message could not be written.
    Udp(UdpError),
    /// A label of more than the 63 bytes of RFC 1035, section 2.3.4, or
    /// one of none. The value is the length that was offered.
    Label(usize),
    /// A name of more than the 255 bytes of RFC 1035, section 2.3.4, in
    /// the form it takes on the wire.
    NameTooLong,
    /// The text is not a domain name in the preferred syntax of RFC 1035,
    /// section 2.3.1 as RFC 1123, section 2.1 relaxed it.
    Text,
    /// The two top bits of a length octet are `01` or `10`, which RFC 1035,
    /// section 4.1.4 reserves and gives no meaning. The value is the
    /// octet.
    LabelKind(u8),
    /// A compression pointer that does not point backwards. Following one
    /// is what a message can be written to make a reader do for ever.
    PointerForward {
        /// Where the pointer stands.
        at: usize,
        /// Where it points.
        to: usize,
    },
    /// More compression jumps in one name than the reader follows.
    PointerChain,
    /// The body of a record is not the length its type has. The two values
    /// are the type and the length the record claimed.
    Rdata {
        /// Which type the record is of.
        record_type: RecordType,
        /// How long its body was.
        len: usize,
    },
    /// The answer did not fit a datagram and there is no TCP here
    /// (RFC 1035, section 4.2.2).
    Truncated,
    /// The server answered with a failure code.
    Rcode(ResponseCode),
    /// A `CNAME` chain stepped through a record it had already used.
    CnameLoop,
    /// A `CNAME` chain longer than the eight links this resolver follows.
    CnameChain,
    /// A server of a family other than the one the resolver's own address
    /// belongs to. There is no pseudo-header over one address of each.
    MixedFamilies,
    /// No server was given, so there is nobody to ask.
    NoServer,
    /// The server table is full.
    TooManyServers,
    /// A resolution is running, and there is room for one.
    Busy,
    /// Every attempt at every server went unanswered.
    Exhausted,
    /// The deadline passed with neither question settled.
    Deadline,
}

impl From<WireError> for DnsError {
    fn from(error: WireError) -> DnsError {
        DnsError::Wire(error)
    }
}

impl From<RngError> for DnsError {
    fn from(error: RngError) -> DnsError {
        DnsError::Rng(error)
    }
}

impl From<UdpError> for DnsError {
    fn from(error: UdpError) -> DnsError {
        DnsError::Udp(error)
    }
}

impl fmt::Display for DnsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DnsError::Wire(error) => error.fmt(f),
            DnsError::Rng(error) => error.fmt(f),
            DnsError::Udp(error) => error.fmt(f),
            DnsError::Label(len) => write!(f, "a label of {len} bytes is not one a name has"),
            DnsError::NameTooLong => f.write_str("a name is at most 255 bytes on the wire"),
            DnsError::Text => f.write_str("the text is not a domain name"),
            DnsError::LabelKind(octet) => {
                write!(f, "the length octet {octet:#04x} names no kind of label")
            }
            DnsError::PointerForward { at, to } => {
                write!(f, "the pointer at {at} points to {to} and not backwards")
            }
            DnsError::PointerChain => f.write_str("a name of more compression jumps than are read"),
            DnsError::Rdata { record_type, len } => {
                write!(
                    f,
                    "a body of {len} bytes is not a record of type {}",
                    record_type.get()
                )
            }
            DnsError::Truncated => f.write_str("the answer was truncated and there is no TCP here"),
            DnsError::Rcode(code) => write!(f, "the server answered {code}"),
            DnsError::CnameLoop => f.write_str("the alias chain returns to a record it has used"),
            DnsError::CnameChain => f.write_str("the alias chain is longer than eight links"),
            DnsError::MixedFamilies => f.write_str("the server is not of this resolver's family"),
            DnsError::NoServer => f.write_str("no server was given"),
            DnsError::TooManyServers => f.write_str("the server table is full"),
            DnsError::Busy => f.write_str("a resolution is already running"),
            DnsError::Exhausted => f.write_str("every attempt at every server went unanswered"),
            DnsError::Deadline => f.write_str("the deadline passed before an answer came"),
        }
    }
}
