// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ICMPv6` of RFC 4443.
//!
//! Five messages are read and written: echo request and reply,
//! destination unreachable, packet too big, and time exceeded. The three
//! errors carry as much of the packet that caused them as fits the
//! minimum MTU, which is what lets the transport above match an error to
//! the connection that earned it.
//!
//! Packet too big is the one this family cannot do without. A router
//! never fragments an IPv6 packet, so a packet that does not fit comes
//! back as this message and nothing else; path MTU discovery is driven by
//! it, and is not optional here (see [`crate::pmtu`]).
//!
//! The checksum covers the pseudo-header of RFC 8200, section 8.1, which
//! `ICMPv4`'s does not. RFC 4443, section 2.3 made that change, and it is
//! why every function here that sums takes the two addresses: a message
//! read without them is read structurally and not verified.
//!
//! Nothing here generates an error. This module reads one, writes one it
//! is given, and answers with [`may_answer_with_error`] whether one is
//! allowed at all; a caller that decides to send one bounds the rate with
//! `net_ip::TokenBucket`, which is the same mechanism under the same
//! reasoning — RFC 4443, section 2.4 (f) asks for a limit and names no
//! number, as RFC 1122 does for `ICMPv4`.

use net_wire::{Ipv6Addr, Protocol, Reader, Writer, transport_v6};

use crate::error::Ipv6Error;
use crate::header::{HEADER_LEN as PACKET_HEADER_LEN, MIN_MTU, Packet, Upper};

/// The fixed part of every message: type, code, checksum, and four bytes
/// whose meaning depends on the type.
pub const HEADER_LEN: usize = 8;

/// Where the checksum sits.
pub(crate) const CHECKSUM_OFFSET: usize = 2;

/// The type numbers of RFC 4443 this crate names.
pub(crate) mod message_type {
    /// Destination unreachable.
    pub(crate) const DESTINATION_UNREACHABLE: u8 = 1;
    /// Packet too big.
    pub(crate) const PACKET_TOO_BIG: u8 = 2;
    /// Time exceeded.
    pub(crate) const TIME_EXCEEDED: u8 = 3;
    /// Echo request.
    pub(crate) const ECHO_REQUEST: u8 = 128;
    /// Echo reply.
    pub(crate) const ECHO_REPLY: u8 = 129;
    /// The first type that is informational rather than an error, which
    /// RFC 4443, section 2.1 puts at the top bit of the field.
    pub(crate) const FIRST_INFORMATIONAL: u8 = 128;
}

/// How much of the offending packet an error quotes.
///
/// RFC 4443, section 2.4 (c) asks for as much as fits without the error
/// exceeding the minimum MTU, which is that MTU less the error's own IPv6
/// header and its `ICMPv6` header.
#[must_use]
pub const fn quoted_len(packet_len: usize) -> usize {
    let room = MIN_MTU
        .saturating_sub(PACKET_HEADER_LEN)
        .saturating_sub(HEADER_LEN);
    if packet_len < room { packet_len } else { room }
}

/// Why a destination could not be reached, as RFC 4443, section 3.1 codes
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unreachable {
    /// No route to the destination.
    NoRoute,
    /// A firewall said no.
    Prohibited,
    /// Beyond the scope of the source address.
    BeyondScope,
    /// The address is not there.
    Address,
    /// Nothing is bound to that port. This is the one a transport waits
    /// for.
    Port,
    /// The source address failed an ingress or egress policy.
    SourcePolicy,
    /// The route to the destination is a reject route.
    RejectRoute,
    /// A code this crate does not name.
    Other(u8),
}

impl Unreachable {
    /// The number on the wire.
    #[must_use]
    pub const fn get(self) -> u8 {
        match self {
            Unreachable::NoRoute => 0,
            Unreachable::Prohibited => 1,
            Unreachable::BeyondScope => 2,
            Unreachable::Address => 3,
            Unreachable::Port => 4,
            Unreachable::SourcePolicy => 5,
            Unreachable::RejectRoute => 6,
            Unreachable::Other(code) => code,
        }
    }

    /// The reason of this number.
    #[must_use]
    pub const fn new(code: u8) -> Unreachable {
        match code {
            0 => Unreachable::NoRoute,
            1 => Unreachable::Prohibited,
            2 => Unreachable::BeyondScope,
            3 => Unreachable::Address,
            4 => Unreachable::Port,
            5 => Unreachable::SourcePolicy,
            6 => Unreachable::RejectRoute,
            other => Unreachable::Other(other),
        }
    }
}

/// One `ICMPv6` message, borrowed from the bytes it arrived in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message<'a> {
    /// An echo request: its identifier, its sequence number, and the
    /// payload to send back unchanged.
    EchoRequest {
        /// Which pinger sent it.
        identifier: u16,
        /// Which ping it is.
        sequence: u16,
        /// What comes back with the reply.
        payload: &'a [u8],
    },
    /// An echo reply, of the same shape.
    EchoReply {
        /// Which pinger it answers.
        identifier: u16,
        /// Which ping it answers.
        sequence: u16,
        /// What was sent.
        payload: &'a [u8],
    },
    /// A destination that could not be reached.
    DestinationUnreachable {
        /// Why not.
        reason: Unreachable,
        /// The beginning of the packet that caused it.
        quoted: &'a [u8],
    },
    /// A packet that did not fit a link on the way.
    PacketTooBig {
        /// What the next link carries.
        mtu: u32,
        /// The beginning of the packet that caused it.
        quoted: &'a [u8],
    },
    /// A packet that ran out of hops or of reassembly time.
    TimeExceeded {
        /// Zero in transit, one in reassembly.
        code: u8,
        /// The beginning of the packet that caused it.
        quoted: &'a [u8],
    },
    /// A type this crate does not read here, which includes the four
    /// Neighbor Discovery messages of [`crate::ndp`]. It is carried
    /// rather than refused, so the layer above can dispatch on it.
    Other {
        /// The type number.
        message_type: u8,
        /// The code.
        code: u8,
        /// Everything behind the four fixed bytes.
        body: &'a [u8],
    },
}

impl<'a> Message<'a> {
    /// The message in `bytes`, read for its structure and not verified.
    ///
    /// The checksum is not checked, because it cannot be: it covers a
    /// pseudo-header of two addresses this function is not given.
    /// [`parse_checked`](Self::parse_checked) is what a receive path
    /// calls; this one is for a caller that has the message and not the
    /// packet.
    ///
    /// # Errors
    ///
    /// [`Ipv6Error::BadIcmp`] when the message is shorter than its
    /// header.
    pub fn parse(bytes: &'a [u8]) -> Result<Message<'a>, Ipv6Error> {
        let mut reader = Reader::new(bytes);
        let message_type = reader.read_u8().map_err(|_| Ipv6Error::BadIcmp)?;
        let code = reader.read_u8().map_err(|_| Ipv6Error::BadIcmp)?;
        let _checksum = reader.read_u16().map_err(|_| Ipv6Error::BadIcmp)?;
        let first = reader.read_u16().map_err(|_| Ipv6Error::BadIcmp)?;
        let second = reader.read_u16().map_err(|_| Ipv6Error::BadIcmp)?;
        let rest = reader.rest();
        Ok(match message_type {
            message_type::ECHO_REQUEST => Message::EchoRequest {
                identifier: first,
                sequence: second,
                payload: rest,
            },
            message_type::ECHO_REPLY => Message::EchoReply {
                identifier: first,
                sequence: second,
                payload: rest,
            },
            message_type::DESTINATION_UNREACHABLE => Message::DestinationUnreachable {
                reason: Unreachable::new(code),
                quoted: rest,
            },
            message_type::PACKET_TOO_BIG => Message::PacketTooBig {
                mtu: u32::from(first).wrapping_shl(16) | u32::from(second),
                quoted: rest,
            },
            message_type::TIME_EXCEEDED => Message::TimeExceeded { code, quoted: rest },
            other => Message::Other {
                message_type: other,
                code,
                body: rest,
            },
        })
    }

    /// The message in `bytes`, whose checksum must verify over the
    /// pseudo-header of the two addresses it arrived between.
    ///
    /// # Errors
    ///
    /// [`Ipv6Error::BadIcmp`] when the message is short or the checksum
    /// does not verify.
    pub fn parse_checked(
        source: Ipv6Addr,
        destination: Ipv6Addr,
        bytes: &'a [u8],
    ) -> Result<Message<'a>, Ipv6Error> {
        if !verify(source, destination, bytes) {
            return Err(Ipv6Error::BadIcmp);
        }
        Message::parse(bytes)
    }

    /// Whether this message is an error rather than a question or an
    /// answer, which RFC 4443, section 2.4 (e) forbids answering with
    /// another error.
    ///
    /// A type this crate does not name is an error when its top bit is
    /// clear, which is how RFC 4443, section 2.1 splits the space. That
    /// is not cosmetic: section 2.4 (a) has an error of unknown type
    /// passed up to the process whose packet caused it, where an
    /// informational message of unknown type is dropped.
    #[must_use]
    pub const fn is_error(&self) -> bool {
        match self {
            Message::DestinationUnreachable { .. }
            | Message::PacketTooBig { .. }
            | Message::TimeExceeded { .. } => true,
            Message::Other { message_type, .. } => {
                *message_type < message_type::FIRST_INFORMATIONAL
            }
            Message::EchoRequest { .. } | Message::EchoReply { .. } => false,
        }
    }

    /// The reply this message deserves, or `None` when it deserves none.
    /// Only an echo request does.
    #[must_use]
    pub const fn reply(self) -> Option<Message<'a>> {
        match self {
            Message::EchoRequest {
                identifier,
                sequence,
                payload,
            } => Some(Message::EchoReply {
                identifier,
                sequence,
                payload,
            }),
            _ => None,
        }
    }

    /// Writes the message with its checksum over the pseudo-header of
    /// `source` and `destination`.
    ///
    /// # Errors
    ///
    /// [`Ipv6Error::Wire`] when the buffer is too small, and
    /// [`Ipv6Error::BadIcmp`] for a message this crate carries but does
    /// not write, which is [`Message::Other`].
    pub fn write(
        &self,
        source: Ipv6Addr,
        destination: Ipv6Addr,
        writer: &mut Writer<'_>,
    ) -> Result<(), Ipv6Error> {
        let (message_type, code, first, second, rest) = match *self {
            Message::EchoRequest {
                identifier,
                sequence,
                payload,
            } => (message_type::ECHO_REQUEST, 0, identifier, sequence, payload),
            Message::EchoReply {
                identifier,
                sequence,
                payload,
            } => (message_type::ECHO_REPLY, 0, identifier, sequence, payload),
            Message::DestinationUnreachable { reason, quoted } => (
                message_type::DESTINATION_UNREACHABLE,
                reason.get(),
                0,
                0,
                quoted,
            ),
            Message::PacketTooBig { mtu, quoted } => {
                let high = u16::try_from(mtu.wrapping_shr(16)).unwrap_or(u16::MAX);
                let low = u16::try_from(mtu & 0xFFFF).unwrap_or(u16::MAX);
                (message_type::PACKET_TOO_BIG, 0, high, low, quoted)
            }
            Message::TimeExceeded { code, quoted } => {
                (message_type::TIME_EXCEEDED, code, 0, 0, quoted)
            }
            Message::Other { .. } => return Err(Ipv6Error::BadIcmp),
        };
        let needed = HEADER_LEN.saturating_add(rest.len());
        if writer.remaining() < needed {
            return Err(Ipv6Error::Wire(net_wire::WireError::OutOfBounds {
                needed,
                available: writer.remaining(),
            }));
        }
        let at = writer.position();
        writer.write_u8(message_type)?;
        writer.write_u8(code)?;
        // The checksum covers the message it sits in, so it goes in zero
        // and is filled in once the message stands.
        writer.write_u16(0)?;
        writer.write_u16(first)?;
        writer.write_u16(second)?;
        writer.write_bytes(rest)?;
        finish(writer, at, source, destination)
    }
}

/// Whether the checksum in `message` verifies over the pseudo-header of
/// the two addresses.
///
/// Summing a block that already holds its own checksum yields zero, which
/// is RFC 1071, section 1, step (3); the pseudo-header goes in front of it
/// as RFC 8200, section 8.1 prescribes.
#[must_use]
pub fn verify(source: Ipv6Addr, destination: Ipv6Addr, message: &[u8]) -> bool {
    if message.len() < HEADER_LEN {
        return false;
    }
    transport_v6(source, destination, Protocol::ICMPV6, message) == Ok(0)
}

/// Fills in the checksum of the message that begins at `at` in `writer`.
///
/// This is where every writer in this crate ends, Neighbor Discovery
/// included: the sum covers the pseudo-header and then the message, so it
/// cannot be taken before the message stands.
///
/// # Errors
///
/// [`Ipv6Error::BadIcmp`] when `at` is not inside what was written, and
/// [`Ipv6Error::Wire`] when the patch does not land.
pub(crate) fn finish(
    writer: &mut Writer<'_>,
    at: usize,
    source: Ipv6Addr,
    destination: Ipv6Addr,
) -> Result<(), Ipv6Error> {
    let message = writer.written().get(at..).ok_or(Ipv6Error::BadIcmp)?;
    let sum = transport_v6(source, destination, Protocol::ICMPV6, message)
        .map_err(|_| Ipv6Error::BadIcmp)?;
    writer.patch_u16(at.saturating_add(CHECKSUM_OFFSET), sum)?;
    Ok(())
}

/// Whether RFC 4443, section 2.4 (e) allows an error in answer to
/// `packet`, which arrived at a link-layer multicast address or did not.
///
/// The rules are the document's, in its order: no error answers an error,
/// a packet addressed to a multicast address, one that arrived as a
/// link-layer multicast or broadcast, or one whose source names no single
/// node.
///
/// The two exceptions the document makes are for packet too big and for a
/// parameter problem of code 2, and neither is generated here: this host
/// forwards nothing, so it is never the node that finds a packet too big
/// for the next link.
#[must_use]
pub fn may_answer_with_error(packet: Packet<'_>, upper: Upper<'_>, link_multicast: bool) -> bool {
    if upper.protocol == Protocol::ICMPV6
        && Message::parse(upper.payload).is_ok_and(|message| message.is_error())
    {
        return false;
    }
    if packet.destination().is_multicast() || link_multicast {
        return false;
    }
    let source = packet.source();
    !(source.is_unspecified() || source.is_multicast() || source.is_loopback())
}
