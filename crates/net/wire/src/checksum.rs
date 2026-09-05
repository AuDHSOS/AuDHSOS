// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The internet checksum of RFC 1071, which IPv4, ICMP, `ICMPv6`, UDP, and
//! TCP all use.
//!
//! The sum is the sixteen-bit one's complement of the one's complement sum
//! of the data taken as sixteen-bit words, most significant byte first. An
//! odd last byte is paired with a zero, and a checksum is verified by
//! summing the data *including* its checksum field: the result is zero
//! when it is right, which is what RFC 1071 section 1 states as "all 1
//! bits" before the complement.
//!
//! Two properties of section 2 are what this module is built on. The sum
//! is associative and commutative as long as the even and odd positions
//! keep their halves, so a caller may hand it a pseudo-header, then a
//! header, then a payload, in as many calls as it likes; and it is
//! independent of the byte order of the machine, so nothing here has a
//! target in it.
//!
//! The two pseudo-headers differ in more than the width of an address.
//! The IPv6 one of RFC 8200, section 8.1 carries a thirty-two bit length
//! and names the upper-layer protocol rather than the next-header field of
//! the packet, which is a different value whenever an extension header
//! stands between them; and `ICMPv6` is summed over a pseudo-header where
//! `ICMPv4` is not.
//!
//! Where this departs from the memo: RFC 1071 section 2 recommends
//! deferring the end-around carries into the high half of a wider
//! accumulator, which saves an instruction per word. [`Checksum`] carries
//! them at once instead, and its accumulator is therefore sixteen bits
//! that provably never leave sixteen bits. That is what lets every
//! addition in this crate be a checked one under a lint profile that
//! forbids an operation which could overflow. The deferred form is a hand
//! optimization for assembly; this is Rust, and the bound is worth more
//! than the instruction.

use crate::addr::{IpAddr, Ipv4Addr, Ipv6Addr};
use crate::error::WireError;
use crate::protocol::Protocol;

/// The one's complement sum of everything fed to it so far.
///
/// The accumulator holds the sum, not the checksum: [`finish`](Checksum::finish)
/// complements it. A caller that sums in several calls keeps this value
/// between them, including the odd byte one call ended on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Checksum {
    /// The one's complement sum of the complete words seen so far.
    sum: u16,
    /// The odd byte the last call ended on, which is the high half of the
    /// next word rather than a word of its own. Without it, two calls over
    /// three and five bytes would sum different words than one call over
    /// eight, and RFC 1071 section 2 (A) is explicit that the even and odd
    /// positions must keep their halves.
    pending: Option<u8>,
}

impl Checksum {
    /// An empty sum.
    #[must_use]
    pub const fn new() -> Checksum {
        Checksum {
            sum: 0,
            pending: None,
        }
    }

    /// Adds one sixteen-bit word with the end-around carry.
    pub const fn add_word(&mut self, word: u16) {
        let (total, carry) = self.sum.overflowing_add(word);
        // Both operands are at most `0xFFFF`, so a sum that carried is at
        // most `0xFFFE` and the carry that goes back in cannot overflow.
        self.sum = if carry {
            total.saturating_add(1)
        } else {
            total
        };
    }

    /// Adds `data`, taken as words of two bytes, the first byte the more
    /// significant. An odd final byte is held back and pairs with the
    /// first byte of the next call, or with a zero at [`finish`](Checksum::finish).
    pub fn add_bytes(&mut self, data: &[u8]) {
        let mut rest = data;
        if let Some(high) = self.pending
            && let Some((low, tail)) = rest.split_first()
        {
            self.pending = None;
            self.add_word(u16::from_be_bytes([high, *low]));
            rest = tail;
        }
        let (words, remainder) = rest.as_chunks::<2>();
        for word in words {
            self.add_word(u16::from_be_bytes(*word));
        }
        if let Some(&odd) = remainder.first() {
            self.pending = Some(odd);
        }
    }

    /// Adds the IPv4 pseudo-header of RFC 793: the two addresses, a zero,
    /// the protocol, and the length of the transport segment. UDP and TCP
    /// both begin their checksum with it, which is what binds a segment to
    /// the addresses it was addressed with.
    pub fn add_pseudo_header_v4(
        &mut self,
        source: Ipv4Addr,
        destination: Ipv4Addr,
        protocol: Protocol,
        length: u16,
    ) {
        self.add_bytes(&source.octets());
        self.add_bytes(&destination.octets());
        self.add_word(u16::from(protocol.get()));
        self.add_word(length);
    }

    /// Adds the IPv6 pseudo-header of RFC 8200, section 8.1: the two
    /// addresses, the upper-layer length in thirty-two bits, three zero
    /// bytes, and the next-header value.
    ///
    /// Two things differ from the IPv4 form beyond the address width. The
    /// length is thirty-two bits, because a jumbogram has no sixteen-bit
    /// length; and `next_header` is the upper-layer protocol, not the
    /// next-header field of the packet, which names the first extension
    /// header whenever there is one.
    pub fn add_pseudo_header_v6(
        &mut self,
        source: Ipv6Addr,
        destination: Ipv6Addr,
        next_header: Protocol,
        length: u32,
    ) {
        self.add_bytes(&source.octets());
        self.add_bytes(&destination.octets());
        self.add_bytes(&length.to_be_bytes());
        self.add_word(0);
        self.add_word(u16::from(next_header.get()));
    }

    /// The checksum: the sum completed, then complemented.
    ///
    /// A byte held back pairs with a zero here, as RFC 1071 case \[2\]
    /// prescribes for an odd count of bytes.
    #[must_use]
    pub const fn finish(self) -> u16 {
        let mut done = self;
        if let Some(high) = done.pending {
            done.pending = None;
            done.add_word(u16::from_be_bytes([high, 0]));
        }
        !done.sum
    }

    /// The sum itself, uncomplemented and with any held byte still held.
    /// A caller needs this only to combine sums it kept apart.
    #[must_use]
    pub const fn sum(self) -> u16 {
        self.sum
    }
}

/// The checksum of `data`.
#[must_use]
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum = Checksum::new();
    sum.add_bytes(data);
    sum.finish()
}

/// Whether `data` carries a correct checksum in it.
///
/// The field is not named here and does not have to be: summing a block
/// that already holds its own checksum yields zero, whichever field it
/// sits in. This is RFC 1071 section 1, step (3).
#[must_use]
pub fn is_valid(data: &[u8]) -> bool {
    checksum(data) == 0
}

/// The checksum of a UDP or TCP segment over IPv4: the pseudo-header of
/// the two addresses, the protocol, and the segment length, followed by
/// the segment itself.
///
/// `segment` is the transport header and its payload, with the checksum
/// field zeroed. The length in the pseudo-header is the length of
/// `segment`, which is what both protocols put there.
///
/// # Errors
///
/// [`WireError::Length`] when the segment is longer than the sixteen-bit
/// length the pseudo-header carries.
pub fn transport_v4(
    source: Ipv4Addr,
    destination: Ipv4Addr,
    protocol: Protocol,
    segment: &[u8],
) -> Result<u16, WireError> {
    let length = u16::try_from(segment.len()).map_err(|_| WireError::Length(segment.len()))?;
    let mut sum = Checksum::new();
    sum.add_pseudo_header_v4(source, destination, protocol, length);
    sum.add_bytes(segment);
    Ok(sum.finish())
}

/// The checksum of a UDP, TCP, or `ICMPv6` segment over IPv6: the
/// pseudo-header of RFC 8200, section 8.1, followed by the segment.
///
/// `segment` is the upper-layer header and its payload with the checksum
/// field zeroed, and its length is what goes into the pseudo-header.
/// `next_header` is the upper-layer protocol, not the next-header field of
/// the packet.
///
/// # Errors
///
/// [`WireError::Length`] when the segment is longer than the thirty-two
/// bit length the pseudo-header carries. No buffer this system builds
/// comes near four gibibytes, so the check stands for the type of a
/// `usize` rather than for a case that occurs.
pub fn transport_v6(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    next_header: Protocol,
    segment: &[u8],
) -> Result<u16, WireError> {
    let length = u32::try_from(segment.len()).map_err(|_| WireError::Length(segment.len()))?;
    let mut sum = Checksum::new();
    sum.add_pseudo_header_v6(source, destination, next_header, length);
    sum.add_bytes(segment);
    Ok(sum.finish())
}

/// The checksum of a transport segment over whichever family the two
/// addresses belong to.
///
/// This is what a layer above the wire calls, so that a socket writes its
/// checksum once rather than once per family.
///
/// # Errors
///
/// [`WireError::MixedFamilies`] when the two addresses are not of one
/// family, and [`WireError::Length`] as the two family-specific functions
/// return it.
pub fn transport(
    source: IpAddr,
    destination: IpAddr,
    protocol: Protocol,
    segment: &[u8],
) -> Result<u16, WireError> {
    match (source, destination) {
        (IpAddr::V4(source), IpAddr::V4(destination)) => {
            transport_v4(source, destination, protocol, segment)
        }
        (IpAddr::V6(source), IpAddr::V6(destination)) => {
            transport_v6(source, destination, protocol, segment)
        }
        _ => Err(WireError::MixedFamilies),
    }
}
