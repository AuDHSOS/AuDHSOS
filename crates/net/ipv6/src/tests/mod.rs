// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The host tests of the crate, one module per product module, with the
//! packet builders they share.
//!
//! Every packet here is built by project code out of the field layouts
//! the RFCs draw. Nothing is a recording of a foreign device (D-40).

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds a packet at known offsets"
)]

use audhsos_time::{Duration, Instant};
use net_wire::{Ipv6Addr, MacAddr, Protocol, transport_v6};

mod error;
mod fragment;
mod header;
mod icmp;
mod ndp;
mod pmtu;
mod send;
mod slaac;

/// This host's address in the tests that need a unicast one.
const HOST: Ipv6Addr = Ipv6Addr::from_octets([
    0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01,
]);

/// The other end.
const PEER: Ipv6Addr = Ipv6Addr::from_octets([
    0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x02,
]);

/// This host's hardware address.
const HARDWARE: MacAddr = MacAddr::new([0x00, 0x1B, 0x44, 0x11, 0x3A, 0xB7]);

/// The other end's.
const PEER_HARDWARE: MacAddr = MacAddr::new([0x00, 0x1B, 0x44, 0x11, 0x3A, 0xC8]);

/// `micros` microseconds after the origin.
fn at(micros: u64) -> Instant {
    Instant::from_micros(micros)
}

/// `seconds` seconds after the origin.
fn secs(seconds: u64) -> Instant {
    Instant::ZERO.saturating_add(Duration::from_secs(seconds))
}

/// A packet from `source` to `destination` whose first header is
/// `next_header`, carrying `payload`.
///
/// The header is the forty bytes of RFC 8200, section 3, with the traffic
/// class, the flow label, and — there being no such field — no checksum.
fn packet(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    next_header: Protocol,
    hop_limit: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut bytes = Vec::new();
    // Version six in the top four bits; the rest of the byte and the two
    // behind it are the traffic class and the flow label.
    bytes.push(0x60);
    bytes.extend_from_slice(&[0, 0, 0]);
    let length = u16::try_from(payload.len()).expect("a payload that fits the field");
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.push(next_header.get());
    bytes.push(hop_limit);
    bytes.extend_from_slice(&source.octets());
    bytes.extend_from_slice(&destination.octets());
    bytes.extend_from_slice(payload);
    bytes
}

/// An extension header of `kind` whose length field says `units`, filled
/// out to the bytes that implies, naming `next` behind it.
///
/// The body is zero, which is a pad-N option of the right length for the
/// two options headers and reserved bytes for the routing header. Nothing
/// in this crate reads it.
fn extension(next: Protocol, units: u8, body_of: usize) -> Vec<u8> {
    let mut bytes = vec![next.get(), units];
    bytes.resize(body_of, 0);
    bytes
}

/// A hop-by-hop, routing, or destination options header of eight bytes.
fn short_extension(next: Protocol) -> Vec<u8> {
    extension(next, 0, 8)
}

/// The eight bytes of a fragment header (RFC 8200, section 4.5).
fn fragment_header(next: Protocol, offset: usize, more: bool, identification: u32) -> Vec<u8> {
    let mut word = u16::try_from(offset).expect("an offset that fits") & 0xFFF8;
    if more {
        word |= 1;
    }
    let mut bytes = vec![next.get(), 0];
    bytes.extend_from_slice(&word.to_be_bytes());
    bytes.extend_from_slice(&identification.to_be_bytes());
    bytes
}

/// Puts the right checksum into an `ICMPv6` message, which is what a
/// sender does and what every test that feeds one to a parser has to do.
fn checksummed(source: Ipv6Addr, destination: Ipv6Addr, message: &mut [u8]) {
    message[2] = 0;
    message[3] = 0;
    let sum = transport_v6(source, destination, Protocol::ICMPV6, message).expect("it fits");
    let bytes = sum.to_be_bytes();
    message[2] = bytes[0];
    message[3] = bytes[1];
}
