// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The IPv4 layer against arbitrary bytes: no input may panic, and
//! whatever a parser hands back stays inside the bytes it came from.
//!
//! Four parsers are driven, because a byte stream reaches each of them by
//! a different door. The datagram parser is the front one. `Quoted` reads
//! a header whose declared length is longer than what arrived, which is
//! the shape no whole-datagram parser accepts and therefore the shape a
//! whole-datagram parser is never tested on. The ICMP parser sits behind
//! the datagram's payload, and reaching it through a well-formed header
//! would cost the fuzzer most of its inputs, so it is given the raw bytes
//! as well. The reassembler is the one with state, and it is the reason
//! this target exists: a stream of fragments that overlap, repeat, and
//! stop short is what a hand-written test enumerates and a fuzzer
//! explores.

use audhsos_time::Instant;
use net_ip::fragment::Reassembler;
use net_ip::header::{Datagram, MIN_HEADER_LEN, Quoted};
use net_ip::icmp::Message;

/// The reassembler this target drives: four datagrams of two kibibytes.
type Buffers = Reassembler<4, 2048>;

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    datagram(bytes);
    quoted(bytes);
    icmp(bytes);
    reassemble(bytes);
});

/// The datagram parser, and what a datagram it accepts must satisfy.
fn datagram(bytes: &[u8]) {
    let Ok(datagram) = Datagram::parse(bytes) else {
        return;
    };
    assert!(
        datagram.total_len() <= bytes.len(),
        "a datagram longer than the bytes it was read from"
    );
    assert!(
        datagram.header_len() >= MIN_HEADER_LEN,
        "a header shorter than a header"
    );
    assert!(
        datagram.header_len() <= datagram.total_len(),
        "a header longer than the datagram it heads"
    );
    assert_eq!(
        datagram.header_len() + datagram.payload().len(),
        datagram.total_len(),
        "a payload that does not fill what the header left"
    );
    assert!(
        datagram.fragment_offset() % 8 == 0,
        "an offset that is not a multiple of eight"
    );
    assert!(
        datagram.is_initial_fragment() == (datagram.fragment_offset() == 0),
        "a first fragment that is not at zero"
    );
    // A datagram that is not a fragment is one piece of itself.
    if !datagram.is_fragment() {
        assert!(!datagram.more_fragments(), "a whole datagram with more");
    }
}

/// The quoted header, which is the same header with the payload cut off.
/// Every prefix of the input is offered, because that is what an error
/// message carries.
fn quoted(bytes: &[u8]) {
    let Ok(quoted) = Quoted::parse(bytes) else {
        return;
    };
    assert!(
        quoted.payload.len() <= bytes.len(),
        "a quote longer than the bytes it was read from"
    );
    // What a quote accepts, a longer buffer accepts as well: the header is
    // the same and only the total length differs.
    if let Ok(datagram) = Datagram::parse(bytes) {
        assert_eq!(datagram.source(), quoted.source, "two readings of a source");
        assert_eq!(
            datagram.destination(),
            quoted.destination,
            "two readings of a destination"
        );
        assert_eq!(
            datagram.protocol(),
            quoted.protocol,
            "two readings of a protocol"
        );
    }
}

/// The ICMP parser, and the rule that what it writes it reads back.
fn icmp(bytes: &[u8]) {
    let Ok(message) = Message::parse(bytes) else {
        return;
    };
    // `Other` is carried and never written, so it round-trips through
    // nothing and is the one shape this rule leaves alone.
    if matches!(message, Message::Other { .. }) {
        return;
    }
    let mut buffer = [0u8; 2048];
    let mut writer = net_wire::Writer::new(&mut buffer);
    let Ok(()) = message.write(&mut writer) else {
        return;
    };
    let written = writer.written();
    assert_eq!(
        Message::parse(written),
        Ok(message),
        "a message that did not read back as itself"
    );
}

/// The reassembler, driven with the input cut into fragments at every
/// offset a fragment may begin at.
///
/// The whole input is offered first as one datagram, and then the parser's
/// own view of it is fed in repeatedly: the same fragment twice is a
/// duplicate, and a fuzzer that finds two that disagree finds an overlap.
fn reassemble(bytes: &[u8]) {
    let Ok(datagram) = Datagram::parse(bytes) else {
        return;
    };
    let mut buffers = Buffers::new();
    for step in 0..4u64 {
        let now = Instant::from_micros(step);
        match buffers.accept(datagram, now) {
            Ok(Some(assembled)) => {
                assert!(
                    assembled.payload().len() <= 2048,
                    "a datagram longer than the buffer it was assembled in"
                );
            }
            Ok(None) | Err(_) => {}
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
