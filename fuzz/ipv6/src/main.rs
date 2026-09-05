// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The IPv6 layer against arbitrary bytes: no input may panic, and
//! whatever a parser hands back stays inside the bytes it came from.
//!
//! This target exists for the extension header chain. It is the one part
//! of this format a byte stream can drive in circles: a chain is a linked
//! list an attacker writes, every header names the next, and the only
//! things that end a walk over it are the two bounds and the end of the
//! packet. A hand-written test enumerates the chains someone thought of;
//! a fuzzer finds the ones nobody did.
//!
//! Four parsers are driven, because a byte stream reaches each of them by
//! a different door. The packet parser and its chain walk are the front
//! one. The `ICMPv6` parser sits behind the payload, and reaching it
//! through a well-formed chain would cost the fuzzer most of its inputs,
//! so it is given the raw bytes as well — structurally, since a checksum
//! over a pseudo-header is not something a mutator finds. Neighbor
//! Discovery sits behind that again and has an option list of its own,
//! which is a second walk with a second bound. And the reassembler is the
//! one with state.

use audhsos_time::Instant;
use net_ip::Reassembler;
use net_ipv6::fragment::reassemble;
use net_ipv6::header::{HEADER_LEN, Header, MAX_EXTENSION_BYTES, MAX_EXTENSION_HEADERS, Packet};
use net_ipv6::icmp::Message;
use net_ipv6::ndp::{Discovery, NdpOption};

/// The reassembler this target drives: four datagrams of two kibibytes.
type Buffers = Reassembler<4, 2048>;

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    packet(bytes);
    icmp(bytes);
    discovery(bytes);
    reassembly(bytes);
});

/// The packet parser, the chain walk, and what a packet they accept must
/// satisfy.
fn packet(bytes: &[u8]) {
    let Ok(packet) = Packet::parse(bytes) else {
        return;
    };
    assert!(
        packet.total_len() <= bytes.len(),
        "a packet longer than the bytes it was read from"
    );
    assert_eq!(
        packet.header().len(),
        HEADER_LEN,
        "a header that is not forty bytes"
    );
    assert_eq!(
        packet.header().len() + packet.payload().len(),
        packet.total_len(),
        "a payload that does not fill what the header left"
    );

    // What the header carries, written back, is the header again. The
    // traffic class and the flow label are not carried, so the first six
    // bytes are outside the comparison.
    let header = Header {
        source: packet.source(),
        destination: packet.destination(),
        next_header: packet.next_header(),
        hop_limit: packet.hop_limit(),
        payload_len: packet.payload().len(),
    };
    let mut buffer = [0u8; HEADER_LEN];
    let mut writer = net_wire::Writer::new(&mut buffer);
    header.write(&mut writer).expect("forty bytes are free");
    assert_eq!(
        writer.written().get(6..),
        bytes.get(6..HEADER_LEN),
        "a header that did not write back as itself"
    );

    let Ok(upper) = packet.upper_layer() else {
        return;
    };
    assert!(
        upper.headers <= MAX_EXTENSION_HEADERS,
        "a chain accepted past the header bound"
    );
    assert!(
        upper.bytes <= MAX_EXTENSION_BYTES,
        "a chain accepted past the byte bound"
    );
    assert!(
        upper.payload.len() <= packet.payload().len(),
        "a chain walk that grew the payload"
    );
    assert!(
        upper.bytes + upper.payload.len() <= packet.payload().len(),
        "a chain and a payload that do not fit what carried them"
    );
    if let Some(fragment) = upper.fragment {
        assert!(
            fragment.offset % 8 == 0,
            "an offset that is not a multiple of eight"
        );
        assert!(
            upper.is_fragment() == (fragment.offset != 0 || fragment.more),
            "an atomic fragment read as a piece"
        );
    } else {
        assert!(!upper.is_fragment(), "a piece with no fragment header");
    }
}

/// The `ICMPv6` parser, and the rule that what it writes it reads back.
///
/// The structural parser is the one driven here. `parse_checked` verifies
/// a checksum over a pseudo-header the mutator has no way to satisfy, so
/// driving it would test one branch and nothing behind it; what this does
/// instead is check that every message the writer produces carries a sum
/// that verifies.
fn icmp(bytes: &[u8]) {
    let Ok(message) = Message::parse(bytes) else {
        return;
    };
    // `Other` is carried and never written, so it round-trips through
    // nothing and is the one shape this rule leaves alone.
    if matches!(message, Message::Other { .. }) {
        return;
    }
    let source = net_wire::Ipv6Addr::LOCALHOST;
    let destination = net_wire::Ipv6Addr::ALL_NODES;
    let mut buffer = [0u8; 2048];
    let mut writer = net_wire::Writer::new(&mut buffer);
    let Ok(()) = message.write(source, destination, &mut writer) else {
        return;
    };
    let written = writer.written();
    assert_eq!(
        Message::parse(written),
        Ok(message),
        "a message that did not read back as itself"
    );
    assert!(
        net_ipv6::icmp::verify(source, destination, written),
        "a message this crate wrote whose checksum does not verify"
    );
}

/// The Neighbor Discovery parser and its option walk, which is the second
/// bounded walk in this format.
fn discovery(bytes: &[u8]) {
    let Ok(message) = Discovery::parse(bytes) else {
        return;
    };
    let mut options = message.options();
    let mut seen = 0usize;
    let mut walked = 0usize;
    while let Some(option) = options.next_option() {
        seen += 1;
        assert!(
            seen <= bytes.len(),
            "more options than there are bytes to hold them"
        );
        match option {
            NdpOption::Other { body, .. } => walked += body.len(),
            NdpOption::Rdnss(servers) => {
                assert_eq!(
                    servers.len(),
                    servers.iter().count(),
                    "a server count that does not match the addresses"
                );
                assert!(!servers.is_empty(), "an option that names no server");
            }
            _ => {}
        }
    }
    assert!(
        walked <= bytes.len(),
        "options whose bodies outgrew the message"
    );
}

/// The reassembler, driven with whatever fragment header the input
/// carries.
///
/// The same piece offered four times is a duplicate three times over, and
/// a fuzzer that finds two pieces which disagree finds an overlap.
fn reassembly(bytes: &[u8]) {
    let Ok(packet) = Packet::parse(bytes) else {
        return;
    };
    let Ok(upper) = packet.upper_layer() else {
        return;
    };
    let mut buffers = Buffers::new();
    for step in 0..4u64 {
        let now = Instant::from_micros(step);
        if let Ok(Some(payload)) = reassemble(&mut buffers, packet, upper, now) {
            if upper.is_fragment() {
                assert!(
                    payload.len() <= 2048,
                    "a datagram longer than the buffer it was assembled in"
                );
            } else {
                // A packet that is no piece is borrowed where it landed
                // and is not copied into a buffer at all, so its length
                // is the packet's and not the reassembler's.
                assert_eq!(
                    payload, upper.payload,
                    "a whole datagram that was not handed back as it arrived"
                );
            }
        }
    }
    // Time only ever moves forward, and the buffers empty when it passes
    // every deadline.
    let far = Instant::from_micros(u64::MAX);
    let _ = buffers.poll(far);
    assert!(buffers.is_empty(), "a buffer that outlived every deadline");
    assert_eq!(
        buffers.poll_at(),
        None,
        "a deadline in an empty reassembler"
    );
}
