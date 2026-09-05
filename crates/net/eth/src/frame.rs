// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Ethernet II frames, as RFC 894 carries IP over them.
//!
//! Fourteen bytes of header — destination, source, type — and at most
//! 1500 bytes behind it. No VLAN tag is read or written in this version
//! (D-50), and no frame check sequence is computed: RFC 894 leaves that
//! to the hardware, and a device hands up a frame with it already
//! stripped and verified.
//!
//! A frame is a borrowed view. Nothing here copies a payload; the layer
//! above reads it where it landed.

use net_wire::{EtherType, Ipv6Addr, MacAddr, Reader, Writer};

use crate::error::EthError;

/// The header: two addresses and a type.
pub const HEADER_LEN: usize = 14;

/// The largest payload an Ethernet II frame carries (RFC 894).
pub const MTU: usize = 1500;

/// The largest frame, header and payload together. The frame check
/// sequence is not part of it, because the device strips it.
pub const MAX_FRAME_LEN: usize = HEADER_LEN + MTU;

/// A frame, borrowed from the buffer it arrived in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame<'a> {
    /// The destination address.
    destination: MacAddr,
    /// The source address.
    source: MacAddr,
    /// What the payload is.
    ether_type: EtherType,
    /// The payload, borrowed from the input.
    payload: &'a [u8],
}

impl<'a> Frame<'a> {
    /// The frame in `bytes`.
    ///
    /// A frame shorter than the header cannot be read at all, which is
    /// the one structural failure here; every other judgement about a
    /// frame is a policy question and belongs to [`receive`].
    ///
    /// # Errors
    ///
    /// [`EthError::FrameTooShort`] when there are fewer than fourteen
    /// bytes, and [`EthError::PayloadTooLong`] when the payload exceeds
    /// the MTU.
    pub fn parse(bytes: &'a [u8]) -> Result<Frame<'a>, EthError> {
        if bytes.len() < HEADER_LEN {
            return Err(EthError::FrameTooShort(bytes.len()));
        }
        let mut reader = Reader::new(bytes);
        let destination = reader.read_mac()?;
        let source = reader.read_mac()?;
        let ether_type = EtherType::new(reader.read_u16()?);
        let payload = reader.rest();
        if payload.len() > MTU {
            return Err(EthError::PayloadTooLong(payload.len()));
        }
        Ok(Frame {
            destination,
            source,
            ether_type,
            payload,
        })
    }

    /// Who the frame is addressed to.
    #[must_use]
    pub const fn destination(self) -> MacAddr {
        self.destination
    }

    /// Who sent it.
    #[must_use]
    pub const fn source(self) -> MacAddr {
        self.source
    }

    /// What the payload is.
    #[must_use]
    pub const fn ether_type(self) -> EtherType {
        self.ether_type
    }

    /// The payload, borrowed from the bytes the frame was read out of.
    #[must_use]
    pub const fn payload(self) -> &'a [u8] {
        self.payload
    }

    /// Whether a station with the address `interface` should look at this
    /// frame.
    ///
    /// Three destinations pass: this station's own address, the broadcast
    /// address, and any multicast group. The last is wider than it has to
    /// be — a station belongs to some groups and not others — and it is
    /// wide on purpose: the groups an IPv6 host joins follow from the
    /// addresses it holds, which this layer does not know, so the group
    /// membership is checked one layer up where it is known. What that
    /// costs is parsing a multicast frame that is then dropped; what it
    /// buys is that this layer needs no list to keep in step.
    #[must_use]
    pub fn is_for(self, interface: MacAddr) -> bool {
        self.destination == interface
            || self.destination.is_broadcast()
            || self.destination.is_multicast()
    }

    /// Writes a frame with this payload.
    ///
    /// # Errors
    ///
    /// [`EthError::PayloadTooLong`] when the payload exceeds the MTU, and
    /// [`EthError::Wire`] when the buffer is too small for the frame.
    /// Nothing is written in either case.
    pub fn write(
        writer: &mut Writer<'_>,
        destination: MacAddr,
        source: MacAddr,
        ether_type: EtherType,
        payload: &[u8],
    ) -> Result<(), EthError> {
        if payload.len() > MTU {
            return Err(EthError::PayloadTooLong(payload.len()));
        }
        let needed = HEADER_LEN.saturating_add(payload.len());
        if writer.remaining() < needed {
            return Err(EthError::Wire(net_wire::WireError::OutOfBounds {
                needed,
                available: writer.remaining(),
            }));
        }
        writer.write_mac(destination)?;
        writer.write_mac(source)?;
        writer.write_u16(ether_type.get())?;
        writer.write_bytes(payload)?;
        Ok(())
    }
}

/// The Ethernet address an IPv6 multicast group is reached at.
///
/// RFC 2464, section 7: the two bytes `33:33` and the last four bytes of
/// the group. It is a function and not a table, so a host sends to a
/// group without having joined anything — which is what Neighbor
/// Discovery does, since the solicited-node group of a neighbor is
/// derived from that neighbor's address and never from this station's.
///
/// ARP has no counterpart: it broadcasts, and a broadcast reaches every
/// station on the link whether it cares or not.
#[must_use]
pub const fn multicast_hardware(group: Ipv6Addr) -> MacAddr {
    let [
        _,
        _,
        _,
        _,
        _,
        _,
        _,
        _,
        _,
        _,
        _,
        _,
        first,
        second,
        third,
        fourth,
    ] = group.octets();
    MacAddr::new([0x33, 0x33, first, second, third, fourth])
}

/// The frame in `bytes`, if a station with the address `interface` has
/// any use for it.
///
/// This is the whole receive filter, and every one of its answers is
/// `None` rather than an error: a link carries other stations' traffic,
/// and a stack that reported each piece of it would report nothing worth
/// reading. A frame is dropped when it is shorter than its header, when
/// it is longer than a frame may be, when it is addressed elsewhere, and
/// when its type is one no layer of this system reads.
#[must_use]
pub fn receive(bytes: &[u8], interface: MacAddr) -> Option<Frame<'_>> {
    let frame = Frame::parse(bytes).ok()?;
    if !frame.is_for(interface) || !frame.ether_type().is_registered() {
        return None;
    }
    Some(frame)
}
