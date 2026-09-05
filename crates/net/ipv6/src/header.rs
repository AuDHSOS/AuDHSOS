// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The IPv6 header of RFC 8200, section 3, and the extension header chain
//! of section 4.
//!
//! The header is forty bytes and always forty bytes: what IPv4 put in
//! options is a chain of headers behind it, each naming the next. A
//! packet is a borrowed view over the bytes it arrived in, and
//! [`Packet::upper_layer`] walks that chain to the header the transport
//! reads.
//!
//! There is no header checksum and therefore nothing to verify. RFC 8200,
//! section 8.1 made the upper-layer checksum mandatory in exchange, and
//! it covers a pseudo-header that names both addresses, so a corrupted
//! address is caught there instead. Nothing in this module computes a sum
//! over a header, and its absence is not an omission.
//!
//! The walk is bounded twice: in the number of headers and in the bytes
//! they take. A chain is a linked list an attacker writes, and the two
//! bounds are what keep a walk over it finite. Every header is at least
//! eight bytes, so the walk also runs out of packet on its own; the
//! bounds are what stop it before it has read a kilobyte of nothing.

use net_wire::{Ipv6Addr, Protocol, Reader, Writer};

use crate::error::Ipv6Error;

/// The header, which has no variable part.
pub const HEADER_LEN: usize = 40;

/// The smallest MTU an IPv6 link may have (RFC 8200, section 5). It is
/// also the floor of the path MTU estimate, which no message may push it
/// below (RFC 8201, section 4).
pub const MIN_MTU: usize = 1280;

/// The version this crate reads.
pub const VERSION: u8 = 6;

/// The hop limit this host sends with.
pub const DEFAULT_HOP_LIMIT: u8 = 64;

/// The hop limit RFC 4861, section 7.1 requires of every Neighbor
/// Discovery message, so that nothing which crossed a router is read as
/// one.
pub const DISCOVERY_HOP_LIMIT: u8 = 255;

/// The fragment header of RFC 8200, section 4.5.
pub const FRAGMENT_HEADER_LEN: usize = 8;

/// How the fragment offset is scaled: eight bytes, as in IPv4.
pub const FRAGMENT_UNIT: usize = 8;

/// How long one unit of an extension header's length field is. The field
/// counts eight-byte units and does not count the first eight.
const EXTENSION_UNIT: usize = 8;

/// How many extension headers this host steps over before it gives up.
///
/// RFC 8200 recommends at most one of each kind and names six, so a
/// well-formed chain is short. Eight leaves room for a repeated
/// destination options header, which the document does allow, and stops
/// well short of a chain built to be walked.
pub const MAX_EXTENSION_HEADERS: usize = 8;

/// How many bytes those headers may take together.
pub const MAX_EXTENSION_BYTES: usize = 512;

/// The offset field of a fragment header sits in the top thirteen bits of
/// its word, which is the byte offset with the low three bits cleared.
const FRAGMENT_OFFSET_MASK: u16 = 0xFFF8;

/// The more-fragments bit, which is the low bit of that same word.
const FRAGMENT_MORE: u16 = 1;

/// One packet, borrowed from the buffer it arrived in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet<'a> {
    /// The forty bytes of the header.
    header: &'a [u8],
    /// What follows it, cut to the payload length the header declares.
    payload: &'a [u8],
    /// Where it came from.
    source: Ipv6Addr,
    /// Where it is going.
    destination: Ipv6Addr,
    /// What the first header behind this one is: an extension header, an
    /// upper-layer protocol, or nothing at all.
    next_header: Protocol,
    /// How many hops are left.
    hop_limit: u8,
}

impl<'a> Packet<'a> {
    /// The packet in `bytes`.
    ///
    /// What is checked: the version, and that the payload length fits the
    /// bytes that arrived. There is nothing else to check — no header
    /// length, because the header has one length, and no checksum,
    /// because the format has none.
    ///
    /// The traffic class and the flow label are read past and not
    /// carried. Neither is set by this host and neither is read by any
    /// layer above it: a flow label is a hint to routers, and a host that
    /// forwards nothing has no use for one.
    ///
    /// A payload length of zero is an empty payload. The jumbogram of
    /// RFC 2675, which puts the real length in a hop-by-hop option and
    /// zero in this field, is not implemented: no link this system
    /// carries traffic over has an MTU near 65535, so a jumbogram is a
    /// packet that cannot arrive (D-50).
    ///
    /// # Errors
    ///
    /// [`Ipv6Error::Version`], [`Ipv6Error::PayloadLength`], and
    /// [`Ipv6Error::Wire`] when the buffer ends inside the header.
    pub fn parse(bytes: &'a [u8]) -> Result<Packet<'a>, Ipv6Error> {
        let mut reader = Reader::new(bytes);
        let version_and_class = reader.read_u8()?;
        let version = version_and_class.wrapping_shr(4);
        if version != VERSION {
            return Err(Ipv6Error::Version(version));
        }
        // The rest of the traffic class and the whole flow label.
        reader.skip(3)?;
        let payload_length = reader.read_u16()?;
        let next_header = Protocol::new(reader.read_u8()?);
        let hop_limit = reader.read_u8()?;
        let source = reader.read_ipv6()?;
        let destination = reader.read_ipv6()?;

        let total = HEADER_LEN.saturating_add(usize::from(payload_length));
        let header = bytes
            .get(..HEADER_LEN)
            .ok_or(Ipv6Error::PayloadLength(payload_length))?;
        let payload = bytes
            .get(HEADER_LEN..total)
            .ok_or(Ipv6Error::PayloadLength(payload_length))?;
        Ok(Packet {
            header,
            payload,
            source,
            destination,
            next_header,
            hop_limit,
        })
    }

    /// Where the packet came from.
    #[must_use]
    pub const fn source(self) -> Ipv6Addr {
        self.source
    }

    /// Where it is going.
    #[must_use]
    pub const fn destination(self) -> Ipv6Addr {
        self.destination
    }

    /// What the first header behind this one is.
    #[must_use]
    pub const fn next_header(self) -> Protocol {
        self.next_header
    }

    /// How many hops are left.
    #[must_use]
    pub const fn hop_limit(self) -> u8 {
        self.hop_limit
    }

    /// Whether the hop limit has run out, which is what a router would
    /// answer with a time-exceeded message.
    ///
    /// A packet addressed to this host arrives with whatever is left and
    /// is not forwarded, so nothing here drops one; this is for a caller
    /// that forwards, which this system does not.
    #[must_use]
    pub const fn is_expired(self) -> bool {
        self.hop_limit == 0
    }

    /// The forty bytes of the header.
    #[must_use]
    pub const fn header(self) -> &'a [u8] {
        self.header
    }

    /// Everything behind the header, cut to the length it declares. The
    /// extension headers, if there are any, are at its front.
    #[must_use]
    pub const fn payload(self) -> &'a [u8] {
        self.payload
    }

    /// The header and the payload together.
    #[must_use]
    pub const fn total_len(self) -> usize {
        self.header.len().saturating_add(self.payload.len())
    }

    /// Walks the extension header chain to the upper-layer header.
    ///
    /// Four kinds are stepped over — hop-by-hop options, routing,
    /// fragment, and destination options — and none is interpreted. The
    /// routing header in particular is read for its length and nothing
    /// else: this host originates none, and a source route is a field an
    /// attacker writes.
    ///
    /// The fragment header is the one whose contents are kept, because
    /// reassembly needs them. A second one is read past and ignored: a
    /// packet is fragmented once, and the first is the one the fragment
    /// this host holds belongs to.
    ///
    /// # Errors
    ///
    /// [`Ipv6Error::ChainTooLong`] when the chain exceeds
    /// [`MAX_EXTENSION_HEADERS`] or [`MAX_EXTENSION_BYTES`],
    /// [`Ipv6Error::ExtensionLength`] when a header declares more bytes
    /// than the packet has, and [`Ipv6Error::Wire`] when it ends inside
    /// one.
    pub fn upper_layer(self) -> Result<Upper<'a>, Ipv6Error> {
        let mut protocol = self.next_header;
        let mut rest = self.payload;
        let mut fragment = None;
        let mut headers = 0usize;
        let mut bytes = 0usize;
        while protocol.is_extension_header() {
            headers = headers.saturating_add(1);
            if headers > MAX_EXTENSION_HEADERS {
                return Err(Ipv6Error::ChainTooLong { headers, bytes });
            }
            let mut reader = Reader::new(rest);
            let following = Protocol::new(reader.read_u8()?);
            let length = if protocol == Protocol::FRAGMENT {
                // Reserved, then the offset with its flags, then the
                // identification: eight bytes in all, and no length field,
                // because there is nothing variable in it.
                reader.skip(1)?;
                let offset_and_flags = reader.read_u16()?;
                let identification = reader.read_u32()?;
                if fragment.is_none() {
                    fragment = Some(Fragment {
                        // The field counts eight-byte units in the top
                        // thirteen bits, so masking the low three off the
                        // word is the offset in bytes already.
                        offset: usize::from(offset_and_flags & FRAGMENT_OFFSET_MASK),
                        more: offset_and_flags & FRAGMENT_MORE != 0,
                        identification,
                    });
                }
                FRAGMENT_HEADER_LEN
            } else {
                // The length counts eight-byte units and does not count
                // the first eight.
                let units = reader.read_u8()?;
                usize::from(units)
                    .saturating_add(1)
                    .saturating_mul(EXTENSION_UNIT)
            };
            bytes = bytes.saturating_add(length);
            if bytes > MAX_EXTENSION_BYTES {
                return Err(Ipv6Error::ChainTooLong { headers, bytes });
            }
            rest = rest
                .get(length..)
                .ok_or(Ipv6Error::ExtensionLength(bytes))?;
            protocol = following;
        }
        Ok(Upper {
            protocol,
            payload: rest,
            fragment,
            headers,
            bytes,
        })
    }
}

/// What a fragment header said.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fragment {
    /// Where this piece begins in the datagram, in bytes.
    pub offset: usize,
    /// Whether another piece follows.
    pub more: bool,
    /// Which datagram of this address pair it is. Thirty-two bits, where
    /// IPv4 has sixteen.
    pub identification: u32,
}

/// What the walk down the chain reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Upper<'a> {
    /// The upper-layer protocol, or [`Protocol::NO_NEXT_HEADER`] when the
    /// chain ended without one.
    pub protocol: Protocol,
    /// The upper-layer header and its payload.
    pub payload: &'a [u8],
    /// What the fragment header said, when there was one.
    pub fragment: Option<Fragment>,
    /// How many extension headers were stepped over.
    pub headers: usize,
    /// How many bytes they took.
    pub bytes: usize,
}

impl Upper<'_> {
    /// Whether the packet is a piece of a larger datagram.
    ///
    /// A fragment header whose offset is zero and whose more bit is clear
    /// is an atomic fragment: it names a whole datagram and RFC 8200,
    /// section 4.5 has it reassembled as one.
    #[must_use]
    pub const fn is_fragment(&self) -> bool {
        match self.fragment {
            Some(fragment) => fragment.offset != 0 || fragment.more,
            None => false,
        }
    }
}

/// What a header is written from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// Where the packet comes from.
    pub source: Ipv6Addr,
    /// Where it goes.
    pub destination: Ipv6Addr,
    /// What follows the header.
    pub next_header: Protocol,
    /// How many hops it may take.
    pub hop_limit: u8,
    /// How long everything behind the header is, extension headers
    /// included.
    pub payload_len: usize,
}

impl Header {
    /// A header with the default hop limit.
    #[must_use]
    pub const fn new(
        source: Ipv6Addr,
        destination: Ipv6Addr,
        next_header: Protocol,
        payload_len: usize,
    ) -> Header {
        Header {
            source,
            destination,
            next_header,
            hop_limit: DEFAULT_HOP_LIMIT,
            payload_len,
        }
    }

    /// Writes the header, ready for the payload to follow through the
    /// same writer.
    ///
    /// The traffic class and the flow label go out as zero. RFC 6437 lets
    /// a host set a flow label to help a router keep a flow together, and
    /// a host that draws one has to draw it from something unguessable;
    /// zero is the value the document defines for a host that does not,
    /// and it costs nothing this system can measure.
    ///
    /// # Errors
    ///
    /// [`Ipv6Error::PayloadTooLong`] when the payload exceeds the 65535
    /// bytes the length field holds, and [`Ipv6Error::Wire`] when fewer
    /// than forty bytes are free. Nothing is written in either case.
    pub fn write(&self, writer: &mut Writer<'_>) -> Result<(), Ipv6Error> {
        let payload_length = u16::try_from(self.payload_len)
            .map_err(|_| Ipv6Error::PayloadTooLong(self.payload_len))?;
        if writer.remaining() < HEADER_LEN {
            return Err(Ipv6Error::Wire(net_wire::WireError::OutOfBounds {
                needed: HEADER_LEN,
                available: writer.remaining(),
            }));
        }
        writer.write_u8(VERSION.wrapping_shl(4))?;
        writer.write_u8(0)?;
        writer.write_u16(0)?;
        writer.write_u16(payload_length)?;
        writer.write_u8(self.next_header.get())?;
        writer.write_u8(self.hop_limit)?;
        writer.write_ipv6(self.source)?;
        writer.write_ipv6(self.destination)?;
        Ok(())
    }
}

/// What a fragment header is written from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FragmentHeader {
    /// What follows this header, which is the upper-layer protocol.
    pub next_header: Protocol,
    /// Where this piece begins in the datagram, in bytes. The low three
    /// bits are dropped, because the field counts eight-byte units and
    /// the fragmenter works in those units already.
    pub offset: usize,
    /// Whether another piece follows.
    pub more: bool,
    /// Which datagram this is.
    pub identification: u32,
}

impl FragmentHeader {
    /// Writes the eight bytes of RFC 8200, section 4.5.
    ///
    /// # Errors
    ///
    /// [`Ipv6Error::Wire`] when fewer than eight bytes are free.
    pub fn write(&self, writer: &mut Writer<'_>) -> Result<(), Ipv6Error> {
        if writer.remaining() < FRAGMENT_HEADER_LEN {
            return Err(Ipv6Error::Wire(net_wire::WireError::OutOfBounds {
                needed: FRAGMENT_HEADER_LEN,
                available: writer.remaining(),
            }));
        }
        let mut offset_and_flags =
            u16::try_from(self.offset).unwrap_or(FRAGMENT_OFFSET_MASK) & FRAGMENT_OFFSET_MASK;
        if self.more {
            offset_and_flags |= FRAGMENT_MORE;
        }
        writer.write_u8(self.next_header.get())?;
        writer.write_u8(0)?;
        writer.write_u16(offset_and_flags)?;
        writer.write_u32(self.identification)?;
        Ok(())
    }
}
