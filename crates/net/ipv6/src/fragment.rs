// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Putting a fragmented datagram back together, through the machinery of
//! `net-ip`.
//!
//! There are no buffers here and no deadlines. The two families differ in
//! where the fragment fields sit — RFC 791 puts them in the header every
//! datagram has, RFC 8200, section 4.5 in an extension header that is
//! there only when it is needed — and in nothing a reassembly buffer
//! cares about, so `net_ip::Reassembler` holds the storage for both and
//! this module is the translation (D-69).
//!
//! One thing does differ, and it is in the key. RFC 791 names a datagram
//! by source, destination, protocol, and identification; RFC 8200 names
//! it by the three without the protocol, because the next-header value a
//! fragment carries is the fragment header's and not the datagram's. So
//! the piece handed over carries one constant in that field, and the key
//! is the triple the document names.
//!
//! An atomic fragment — a fragment header whose offset is zero and whose
//! more bit is clear — is a whole datagram and is not copied at all,
//! which is what RFC 8200, section 4.5 requires and what keeps a sender
//! from making this host hold a buffer per packet.

use audhsos_time::Instant;
use net_ip::{Piece, Reassembler};
use net_wire::{IpAddr, Protocol};

use crate::error::Ipv6Error;
use crate::header::{Packet, Upper};

/// The piece `packet` is, when it is one.
///
/// The protocol field carries [`Protocol::FRAGMENT`] and not the upper
/// layer's, so that two fragments of one datagram meet in one buffer
/// whatever the fragment header says follows them.
#[must_use]
pub fn piece_of<'a>(packet: Packet<'a>, upper: Upper<'a>) -> Option<Piece<'a>> {
    let fragment = upper.fragment?;
    Some(Piece {
        source: IpAddr::V6(packet.source()),
        destination: IpAddr::V6(packet.destination()),
        protocol: Protocol::FRAGMENT,
        identification: fragment.identification,
        offset: fragment.offset,
        more: fragment.more,
        payload: upper.payload,
    })
}

/// Takes one packet in, and answers the whole datagram's payload when it
/// is one or when this piece completed one.
///
/// # Errors
///
/// [`Ipv6Error::Ip`] carrying whatever `net_ip::Reassembler::accept_piece`
/// returns: an overlap that discards the datagram, a datagram too long
/// for the buffer, or a reassembler with no slot.
pub fn reassemble<'a, const SLOTS: usize, const BYTES: usize>(
    buffers: &'a mut Reassembler<SLOTS, BYTES>,
    packet: Packet<'a>,
    upper: Upper<'a>,
    now: Instant,
) -> Result<Option<&'a [u8]>, Ipv6Error> {
    let Some(piece) = piece_of(packet, upper).filter(|piece| piece.offset != 0 || piece.more)
    else {
        // No fragment header, or an atomic one: the datagram is whole
        // where it landed and nothing is copied.
        return Ok(Some(upper.payload));
    };
    Ok(buffers.accept_piece(piece, now)?)
}

/// Forgets the datagram this packet belongs to, which a caller does once
/// it has read a reassembled one out.
pub fn release<const SLOTS: usize, const BYTES: usize>(
    buffers: &mut Reassembler<SLOTS, BYTES>,
    packet: Packet<'_>,
    upper: Upper<'_>,
) {
    if let Some(piece) = piece_of(packet, upper) {
        buffers.release_piece(piece);
    }
}
