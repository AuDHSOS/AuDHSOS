// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The IPv4 header of RFC 791, section 3.1.
//!
//! A datagram is a borrowed view over the bytes it arrived in. Options are
//! located and skipped, never interpreted: this host originates no option
//! and needs none to read a datagram, and a parser that understood them
//! would be reading a field an attacker writes. RFC 791 requires exactly
//! that a host step over what it does not implement.
//!
//! The header checksum is verified on receipt and computed on send, both
//! through the internet checksum of `net-wire`.

use net_wire::{Ipv4Addr, Protocol, Reader, Writer, checksum, is_valid};

use crate::error::IpError;

/// The header without options.
pub const MIN_HEADER_LEN: usize = 20;

/// The longest header, which is fifteen words.
pub const MAX_HEADER_LEN: usize = 60;

/// Where the checksum sits, for the caller that fills it in after the
/// header stands.
pub const CHECKSUM_OFFSET: usize = 10;

/// The version this crate reads.
pub const VERSION: u8 = 4;

/// The default time to live, which is what RFC 1122, section 3.2.1.7
/// recommends.
pub const DEFAULT_TTL: u8 = 64;

/// The don't-fragment bit.
const FLAG_DONT_FRAGMENT: u16 = 0x4000;

/// The more-fragments bit.
const FLAG_MORE_FRAGMENTS: u16 = 0x2000;

/// The mask that leaves the fragment offset.
const OFFSET_MASK: u16 = 0x1FFF;

/// How the offset field is scaled: RFC 791 counts it in units of eight
/// bytes, which is why every fragment but the last is a multiple of eight.
pub const FRAGMENT_UNIT: usize = 8;

/// The same, in the width the field is read in.
const FRAGMENT_UNIT_U16: u16 = 8;

/// One datagram, borrowed from the buffer it arrived in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Datagram<'a> {
    /// The header, including its options.
    header: &'a [u8],
    /// What follows it, cut to the total length the header declares.
    payload: &'a [u8],
    /// Where it came from.
    source: Ipv4Addr,
    /// Where it is going.
    destination: Ipv4Addr,
    /// What the payload is.
    protocol: Protocol,
    /// How many hops are left.
    ttl: u8,
    /// The identification, which names the datagram a fragment belongs to.
    identification: u16,
    /// The flags and the offset, as they sit on the wire.
    flags_and_offset: u16,
}

impl<'a> Datagram<'a> {
    /// The datagram in `bytes`.
    ///
    /// What is checked: the version, that the header length is between
    /// five and fifteen words and inside the buffer, that the total length
    /// is at least the header and no more than the buffer, and that the
    /// header checksum verifies. What is not checked is the time to live,
    /// because a datagram addressed to this host arrives with whatever is
    /// left and is not forwarded; [`is_expired`](Self::is_expired) is
    /// there for a caller that forwards, which this system does not.
    ///
    /// # Errors
    ///
    /// [`IpError::Version`], [`IpError::HeaderLength`],
    /// [`IpError::TotalLength`], [`IpError::Checksum`], or
    /// [`IpError::Wire`] when the buffer ends inside the header.
    pub fn parse(bytes: &'a [u8]) -> Result<Datagram<'a>, IpError> {
        let mut reader = Reader::new(bytes);
        let version_and_length = reader.read_u8()?;
        let version = version_and_length.wrapping_shr(4);
        if version != VERSION {
            return Err(IpError::Version(version));
        }
        let words = version_and_length & 0x0F;
        let header_len = usize::from(words).saturating_mul(4);
        if header_len < MIN_HEADER_LEN || header_len > bytes.len() {
            return Err(IpError::HeaderLength(words));
        }
        let _differentiated_services = reader.read_u8()?;
        let total_length = reader.read_u16()?;
        let total = usize::from(total_length);
        if total < header_len || total > bytes.len() {
            return Err(IpError::TotalLength(total_length));
        }
        let identification = reader.read_u16()?;
        let flags_and_offset = reader.read_u16()?;
        let ttl = reader.read_u8()?;
        let protocol = Protocol::new(reader.read_u8()?);
        let carried = reader.read_u16()?;
        let source = reader.read_ipv4()?;
        let destination = reader.read_ipv4()?;

        let header = bytes
            .get(..header_len)
            .ok_or(IpError::HeaderLength(words))?;
        if !is_valid(header) {
            return Err(IpError::Checksum(carried));
        }
        let payload = bytes
            .get(header_len..total)
            .ok_or(IpError::TotalLength(total_length))?;
        Ok(Datagram {
            header,
            payload,
            source,
            destination,
            protocol,
            ttl,
            identification,
            flags_and_offset,
        })
    }

    /// Where the datagram came from.
    #[must_use]
    pub const fn source(self) -> Ipv4Addr {
        self.source
    }

    /// Where it is going.
    #[must_use]
    pub const fn destination(self) -> Ipv4Addr {
        self.destination
    }

    /// What the payload is.
    #[must_use]
    pub const fn protocol(self) -> Protocol {
        self.protocol
    }

    /// How many hops are left.
    #[must_use]
    pub const fn ttl(self) -> u8 {
        self.ttl
    }

    /// Whether the time to live has run out, which is what a router would
    /// answer with a time-exceeded message.
    #[must_use]
    pub const fn is_expired(self) -> bool {
        self.ttl == 0
    }

    /// The identification field, which names the datagram that a fragment
    /// belongs to.
    #[must_use]
    pub const fn identification(self) -> u16 {
        self.identification
    }

    /// Whether the sender forbade fragmentation.
    #[must_use]
    pub const fn dont_fragment(self) -> bool {
        self.flags_and_offset & FLAG_DONT_FRAGMENT != 0
    }

    /// Whether another fragment follows this one.
    #[must_use]
    pub const fn more_fragments(self) -> bool {
        self.flags_and_offset & FLAG_MORE_FRAGMENTS != 0
    }

    /// Where this fragment's payload begins in the datagram, in bytes.
    #[expect(clippy::as_conversions, reason = "widening cast in a const fn")]
    #[must_use]
    pub const fn fragment_offset(self) -> usize {
        // The field is thirteen bits, so the product is at most 65528 and
        // fits every `usize` there is.
        (self.flags_and_offset & OFFSET_MASK).wrapping_mul(FRAGMENT_UNIT_U16) as usize
    }

    /// Whether the datagram is a piece of a larger one.
    #[must_use]
    pub const fn is_fragment(self) -> bool {
        self.more_fragments() || self.fragment_offset() != 0
    }

    /// Whether this is the first piece, which is the one that carries the
    /// transport header. RFC 1122, section 3.2.2 forbids answering any
    /// other piece with an error.
    #[must_use]
    pub const fn is_initial_fragment(self) -> bool {
        self.fragment_offset() == 0
    }

    /// The header, options included.
    #[must_use]
    pub const fn header(self) -> &'a [u8] {
        self.header
    }

    /// How long the header is.
    #[must_use]
    pub const fn header_len(self) -> usize {
        self.header.len()
    }

    /// The payload, cut to the length the header declares.
    #[must_use]
    pub const fn payload(self) -> &'a [u8] {
        self.payload
    }

    /// The whole datagram, header and payload, which is what a fragment is
    /// reassembled from and what an ICMP error quotes.
    #[must_use]
    pub const fn total_len(self) -> usize {
        self.header.len().saturating_add(self.payload.len())
    }
}

/// What a header is written from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// Where the datagram comes from.
    pub source: Ipv4Addr,
    /// Where it goes.
    pub destination: Ipv4Addr,
    /// What the payload is.
    pub protocol: Protocol,
    /// How many hops it may take.
    pub ttl: u8,
    /// Which datagram it is, for reassembly.
    pub identification: u16,
    /// Whether it may be cut up.
    pub dont_fragment: bool,
    /// Whether another piece follows.
    pub more_fragments: bool,
    /// Where this piece begins, in bytes. It must be a multiple of eight,
    /// which [`write`](Header::write) rounds down rather than refuses,
    /// because the fragmenter is the only caller that sets it and it works
    /// in those units already.
    pub fragment_offset: usize,
    /// How long the payload is.
    pub payload_len: usize,
}

impl Header {
    /// A header for one whole datagram, with no fragmentation and the
    /// default time to live.
    #[must_use]
    pub const fn new(
        source: Ipv4Addr,
        destination: Ipv4Addr,
        protocol: Protocol,
        payload_len: usize,
    ) -> Header {
        Header {
            source,
            destination,
            protocol,
            ttl: DEFAULT_TTL,
            identification: 0,
            dont_fragment: false,
            more_fragments: false,
            fragment_offset: 0,
            payload_len,
        }
    }

    /// Writes the header with its checksum, ready for the payload to
    /// follow through the same writer.
    ///
    /// # Errors
    ///
    /// [`IpError::TotalLength`] when the header and the payload together
    /// exceed the 65535 bytes the length field holds, and
    /// [`IpError::Wire`] when fewer than twenty bytes are free. Nothing is
    /// written in either case.
    pub fn write(&self, writer: &mut Writer<'_>) -> Result<(), IpError> {
        let total = MIN_HEADER_LEN.saturating_add(self.payload_len);
        let total_length = u16::try_from(total).map_err(|_| IpError::TotalLength(u16::MAX))?;
        if writer.remaining() < MIN_HEADER_LEN {
            return Err(IpError::Wire(net_wire::WireError::OutOfBounds {
                needed: MIN_HEADER_LEN,
                available: writer.remaining(),
            }));
        }
        let at = writer.position();
        let mut flags_and_offset = u16::try_from(self.fragment_offset / FRAGMENT_UNIT)
            .unwrap_or(OFFSET_MASK)
            & OFFSET_MASK;
        if self.dont_fragment {
            flags_and_offset |= FLAG_DONT_FRAGMENT;
        }
        if self.more_fragments {
            flags_and_offset |= FLAG_MORE_FRAGMENTS;
        }
        // Five words, no options: this host originates none.
        writer.write_u8(VERSION.wrapping_shl(4) | 5)?;
        writer.write_u8(0)?;
        writer.write_u16(total_length)?;
        writer.write_u16(self.identification)?;
        writer.write_u16(flags_and_offset)?;
        writer.write_u8(self.ttl)?;
        writer.write_u8(self.protocol.get())?;
        // The checksum covers the header it sits in, so it goes in zero
        // and is filled in once the header stands.
        writer.write_u16(0)?;
        writer.write_ipv4(self.source)?;
        writer.write_ipv4(self.destination)?;

        let header = writer
            .written()
            .get(at..)
            .ok_or(IpError::TotalLength(total_length))?;
        let sum = checksum(header);
        writer.patch_u16(at.saturating_add(CHECKSUM_OFFSET), sum)?;
        Ok(())
    }
}
