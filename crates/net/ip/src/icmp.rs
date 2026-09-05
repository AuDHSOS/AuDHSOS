// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ICMPv4` of RFC 792, and the rules of RFC 1122, section 3.2.2 about
//! when an error may be sent at all.
//!
//! Four messages are read and written: echo request and reply,
//! destination unreachable, and time exceeded. The last two carry the
//! header of the datagram that caused them plus the first eight bytes of
//! its payload, which is what lets the transport above match an error to
//! the connection that earned it — a TCP connection to a closed port
//! fails at once instead of timing out.
//!
//! An error is generated only when every restriction of RFC 1122,
//! section 3.2.2 allows it. The memo is explicit that those take
//! precedence over every other requirement to send one, and the reason is
//! in its own discussion: a broadcast to a closed port would otherwise
//! draw an answer from every host on the link at once.

use audhsos_time::{Duration, Instant};
use net_wire::{Ipv4Addr, Protocol, Reader, Writer, checksum, is_valid};

use crate::error::IpError;
use crate::header::Datagram;

/// The fixed part of every message this crate reads: type, code,
/// checksum, and four bytes whose meaning depends on the type.
pub const HEADER_LEN: usize = 8;

/// How much of the offending datagram an error quotes: its header and the
/// eight bytes behind it, which is what RFC 792 requires and what a
/// transport needs to find its own connection.
pub const QUOTED_PAYLOAD_LEN: usize = 8;

/// Where the checksum sits.
const CHECKSUM_OFFSET: usize = 2;

/// The type numbers of RFC 792 this crate knows.
mod message_type {
    /// Echo reply.
    pub(super) const ECHO_REPLY: u8 = 0;
    /// Destination unreachable.
    pub(super) const DESTINATION_UNREACHABLE: u8 = 3;
    /// Echo request.
    pub(super) const ECHO_REQUEST: u8 = 8;
    /// Time exceeded.
    pub(super) const TIME_EXCEEDED: u8 = 11;
}

/// Why a destination could not be reached, as RFC 792 codes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unreachable {
    /// No route to the network.
    Network,
    /// The host is not there.
    Host,
    /// Nothing speaks that protocol.
    Protocol,
    /// Nothing is bound to that port. This is the one a transport waits
    /// for.
    Port,
    /// The datagram was too long for a link and could not be fragmented,
    /// which is what path MTU discovery reads.
    FragmentationNeeded,
    /// A code this crate does not name.
    Other(u8),
}

impl Unreachable {
    /// The number on the wire.
    #[must_use]
    pub const fn get(self) -> u8 {
        match self {
            Unreachable::Network => 0,
            Unreachable::Host => 1,
            Unreachable::Protocol => 2,
            Unreachable::Port => 3,
            Unreachable::FragmentationNeeded => 4,
            Unreachable::Other(code) => code,
        }
    }

    /// The reason of this number.
    #[must_use]
    pub const fn new(code: u8) -> Unreachable {
        match code {
            0 => Unreachable::Network,
            1 => Unreachable::Host,
            2 => Unreachable::Protocol,
            3 => Unreachable::Port,
            4 => Unreachable::FragmentationNeeded,
            other => Unreachable::Other(other),
        }
    }
}

/// One `ICMPv4` message, borrowed from the bytes it arrived in.
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
    /// A destination that could not be reached, with the beginning of the
    /// datagram that tried.
    DestinationUnreachable {
        /// Why not.
        reason: Unreachable,
        /// The header and eight bytes of the datagram that caused it.
        quoted: &'a [u8],
    },
    /// A datagram that ran out of hops or of reassembly time.
    TimeExceeded {
        /// Zero in transit, one in reassembly, as RFC 792 codes it.
        code: u8,
        /// The header and eight bytes of the datagram that caused it.
        quoted: &'a [u8],
    },
    /// A type this crate does not read. It is carried rather than refused,
    /// so that the layer above can count it and drop it.
    Other {
        /// The type number.
        message_type: u8,
        /// The code.
        code: u8,
    },
}

impl<'a> Message<'a> {
    /// The message in `bytes`, whose checksum must verify.
    ///
    /// # Errors
    ///
    /// [`IpError::BadIcmp`] when the message is shorter than its header or
    /// the checksum does not verify.
    pub fn parse(bytes: &'a [u8]) -> Result<Message<'a>, IpError> {
        if bytes.len() < HEADER_LEN || !is_valid(bytes) {
            return Err(IpError::BadIcmp);
        }
        let mut reader = Reader::new(bytes);
        let message_type = reader.read_u8().map_err(|_| IpError::BadIcmp)?;
        let code = reader.read_u8().map_err(|_| IpError::BadIcmp)?;
        let _checksum = reader.read_u16().map_err(|_| IpError::BadIcmp)?;
        let identifier = reader.read_u16().map_err(|_| IpError::BadIcmp)?;
        let sequence = reader.read_u16().map_err(|_| IpError::BadIcmp)?;
        let rest = reader.rest();
        Ok(match message_type {
            message_type::ECHO_REQUEST => Message::EchoRequest {
                identifier,
                sequence,
                payload: rest,
            },
            message_type::ECHO_REPLY => Message::EchoReply {
                identifier,
                sequence,
                payload: rest,
            },
            message_type::DESTINATION_UNREACHABLE => Message::DestinationUnreachable {
                reason: Unreachable::new(code),
                quoted: rest,
            },
            message_type::TIME_EXCEEDED => Message::TimeExceeded { code, quoted: rest },
            other => Message::Other {
                message_type: other,
                code,
            },
        })
    }

    /// Whether this message is itself an error, which RFC 1122,
    /// section 3.2.2 forbids answering with another.
    #[must_use]
    pub const fn is_error(&self) -> bool {
        matches!(
            self,
            Message::DestinationUnreachable { .. } | Message::TimeExceeded { .. }
        )
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

    /// Writes the message with its checksum.
    ///
    /// # Errors
    ///
    /// [`IpError::Wire`] when the buffer is too small, and
    /// [`IpError::BadIcmp`] for a message this crate carries but cannot
    /// write, which is [`Message::Other`].
    pub fn write(&self, writer: &mut Writer<'_>) -> Result<(), IpError> {
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
            Message::TimeExceeded { code, quoted } => {
                (message_type::TIME_EXCEEDED, code, 0, 0, quoted)
            }
            Message::Other { .. } => return Err(IpError::BadIcmp),
        };
        let needed = HEADER_LEN.saturating_add(rest.len());
        if writer.remaining() < needed {
            return Err(IpError::Wire(net_wire::WireError::OutOfBounds {
                needed,
                available: writer.remaining(),
            }));
        }
        let at = writer.position();
        writer.write_u8(message_type)?;
        writer.write_u8(code)?;
        writer.write_u16(0)?;
        writer.write_u16(first)?;
        writer.write_u16(second)?;
        writer.write_bytes(rest)?;
        let message = writer.written().get(at..).ok_or(IpError::BadIcmp)?;
        let sum = checksum(message);
        writer.patch_u16(at.saturating_add(CHECKSUM_OFFSET), sum)?;
        Ok(())
    }
}

/// How much of a datagram an error quotes, which is its header and eight
/// bytes of payload or as much of the payload as there is.
#[must_use]
pub fn quoted_len(datagram: Datagram<'_>) -> usize {
    datagram
        .header_len()
        .saturating_add(datagram.payload().len().min(QUOTED_PAYLOAD_LEN))
}

/// Whether RFC 1122, section 3.2.2 allows an error in answer to
/// `datagram`, which arrived at `interface` and was, or was not, a
/// link-layer broadcast.
///
/// The five restrictions are the memo's, in its order: no error answers an
/// error, a datagram addressed to a broadcast or multicast address, one
/// that arrived as a link-layer broadcast, a non-initial fragment, or one
/// whose source address names no single host.
#[must_use]
pub fn may_answer_with_error(
    datagram: Datagram<'_>,
    link_broadcast: bool,
    interface_broadcast: Ipv4Addr,
) -> bool {
    if datagram.protocol() == Protocol::ICMP
        && Message::parse(datagram.payload()).is_ok_and(|message| message.is_error())
    {
        return false;
    }
    if datagram.destination().is_broadcast()
        || datagram.destination().is_multicast()
        || datagram.destination() == interface_broadcast
    {
        return false;
    }
    if link_broadcast || !datagram.is_initial_fragment() {
        return false;
    }
    let source = datagram.source();
    !(source.is_unspecified()
        || source.is_broadcast()
        || source.is_loopback()
        || source.is_multicast())
}

/// How many errors a host may generate, and how fast the allowance comes
/// back.
///
/// RFC 1122 requires a limit and names no number. This one refills at a
/// steady rate rather than in bursts, because an error that is a little
/// late is worth more than a burst of them at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimit {
    /// How many errors may be sent back to back.
    pub capacity: u32,
    /// How long one allowance takes to come back.
    pub interval: Duration,
}

impl RateLimit {
    /// Ten errors, one back every hundred milliseconds.
    pub const DEFAULT: RateLimit = RateLimit {
        capacity: 10,
        interval: Duration::from_millis(100),
    };
}

impl Default for RateLimit {
    fn default() -> RateLimit {
        RateLimit::DEFAULT
    }
}

/// The token bucket that bounds generated errors.
#[derive(Clone, Copy, Debug)]
pub struct TokenBucket {
    /// The limit it runs on.
    limit: RateLimit,
    /// How many allowances are left.
    tokens: u32,
    /// When one was last added.
    refilled_at: Instant,
}

impl TokenBucket {
    /// A full bucket, starting at `now`.
    #[must_use]
    pub const fn new(limit: RateLimit, now: Instant) -> TokenBucket {
        TokenBucket {
            limit,
            tokens: limit.capacity,
            refilled_at: now,
        }
    }

    /// How many allowances are left, after refilling to `now`.
    pub fn available(&mut self, now: Instant) -> u32 {
        self.refill(now);
        self.tokens
    }

    /// Takes one allowance if there is one, and answers whether there was.
    pub fn take(&mut self, now: Instant) -> bool {
        self.refill(now);
        let Some(left) = self.tokens.checked_sub(1) else {
            return false;
        };
        self.tokens = left;
        true
    }

    /// Adds the allowances that `now` has earned.
    fn refill(&mut self, now: Instant) {
        let interval = self.limit.interval.as_micros();
        if interval == 0 {
            self.tokens = self.limit.capacity;
            self.refilled_at = now;
            return;
        }
        let elapsed = now.saturating_duration_since(self.refilled_at).as_micros();
        // `interval` is not zero: the case is handled above.
        let earned = elapsed.checked_div(interval).unwrap_or(0);
        if earned == 0 {
            return;
        }
        let earned32 = u32::try_from(earned).unwrap_or(u32::MAX);
        self.tokens = self
            .tokens
            .saturating_add(earned32)
            .min(self.limit.capacity);
        // Only the whole intervals are consumed, so the remainder still
        // counts towards the next allowance and the rate does not drift.
        self.refilled_at = self
            .refilled_at
            .saturating_add(Duration::from_micros(earned.saturating_mul(interval)));
    }
}
