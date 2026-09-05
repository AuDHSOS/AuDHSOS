// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `ICMPv4` against RFC 792, and the restrictions of RFC 1122,
//! section 3.2.2 on when an error may be sent at all.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds a message at known offsets"
)]

use audhsos_time::{Duration, Instant};
use net_wire::{Ipv4Addr, Protocol, Writer, is_valid};

use crate::error::IpError;
use crate::header::{Datagram, Header, MIN_HEADER_LEN, Quoted};
use crate::icmp::{
    HEADER_LEN, Message, QUOTED_PAYLOAD_LEN, RateLimit, TokenBucket, Unreachable,
    may_answer_with_error, quoted_len,
};

/// An echo request: type 8, code 0, the checksum `0x06fa` it sums to,
/// identifier `0x1234`, sequence 1, and four bytes of payload.
const ECHO_REQUEST: [u8; 12] = [
    0x08, 0x00, 0x06, 0xFA, 0x12, 0x34, 0x00, 0x01, 0x70, 0x69, 0x6E, 0x67,
];

/// This interface's broadcast address.
const INTERFACE_BROADCAST: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 255);

/// A datagram from `source` to `destination` carrying `payload`, with the
/// fragment fields the caller asks for.
fn datagram(
    source: Ipv4Addr,
    destination: Ipv4Addr,
    protocol: Protocol,
    payload: &[u8],
    fragment_offset: usize,
    buffer: &mut Vec<u8>,
) -> usize {
    buffer.clear();
    buffer.resize(MIN_HEADER_LEN + payload.len(), 0);
    let mut header = Header::new(source, destination, protocol, payload.len());
    header.fragment_offset = fragment_offset;
    header.more_fragments = fragment_offset > 0;
    let mut writer = Writer::new(buffer);
    header.write(&mut writer).expect("room for a header");
    writer.write_bytes(payload).expect("room for the payload");
    writer.position()
}

#[test]
fn an_echo_request_reads_as_the_fields_rfc_792_names() {
    let message = Message::parse(&ECHO_REQUEST).expect("a well-formed request");
    assert_eq!(
        message,
        Message::EchoRequest {
            identifier: 0x1234,
            sequence: 1,
            payload: b"ping",
        }
    );
    assert!(!message.is_error());
}

#[test]
fn an_echo_request_is_answered_with_the_payload_copied() {
    let request = Message::parse(&ECHO_REQUEST).expect("a request");
    let reply = request.reply().expect("a request deserves a reply");
    assert_eq!(
        reply,
        Message::EchoReply {
            identifier: 0x1234,
            sequence: 1,
            payload: b"ping",
        }
    );

    let mut buffer = [0u8; 12];
    let mut writer = Writer::new(&mut buffer);
    reply.write(&mut writer).expect("room for a reply");
    assert!(is_valid(&buffer));
    assert_eq!(&buffer[..2], &[0x00, 0x00]);
    assert_eq!(&buffer[8..], b"ping");
    assert_eq!(Message::parse(&buffer), Ok(reply));
}

#[test]
fn nothing_but_an_echo_request_deserves_a_reply() {
    let reply = Message::EchoReply {
        identifier: 0,
        sequence: 0,
        payload: &[],
    };
    assert_eq!(reply.reply(), None);
    assert_eq!(
        Message::DestinationUnreachable {
            reason: Unreachable::Port,
            quoted: &[],
        }
        .reply(),
        None
    );
    assert_eq!(
        Message::Other {
            message_type: 9,
            code: 0
        }
        .reply(),
        None
    );
}

#[test]
fn a_destination_unreachable_carries_the_header_that_earned_it() {
    let mut bytes = Vec::new();
    let length = datagram(
        Ipv4Addr::new(192, 168, 1, 7),
        Ipv4Addr::new(192, 168, 1, 1),
        Protocol::UDP,
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
        0,
        &mut bytes,
    );
    let offending = Datagram::parse(&bytes[..length]).expect("a datagram");
    assert_eq!(quoted_len(offending), MIN_HEADER_LEN + QUOTED_PAYLOAD_LEN);

    let quoted = &bytes[..quoted_len(offending)];
    let error = Message::DestinationUnreachable {
        reason: Unreachable::Port,
        quoted,
    };
    assert!(error.is_error());

    let mut buffer = vec![0u8; HEADER_LEN + quoted.len()];
    let mut writer = Writer::new(&mut buffer);
    error.write(&mut writer).expect("room for the error");
    assert!(is_valid(&buffer));

    let read = Message::parse(&buffer).expect("what was just written");
    let Message::DestinationUnreachable { reason, quoted } = read else {
        panic!("a destination unreachable came back as {read:?}");
    };
    assert_eq!(reason, Unreachable::Port);
    // The transport finds its own header in what came back. A quote is
    // short by design — the header says thirty bytes and twenty-eight
    // arrived — so it is read as a quote and not as a whole datagram.
    assert!(matches!(
        Datagram::parse(quoted),
        Err(IpError::TotalLength(30))
    ));
    let embedded = Quoted::parse(quoted).expect("the quoted header parses");
    assert_eq!(embedded.protocol, Protocol::UDP);
    assert_eq!(embedded.destination, Ipv4Addr::new(192, 168, 1, 1));
    assert_eq!(embedded.source, Ipv4Addr::new(192, 168, 1, 7));
    assert_eq!(embedded.payload, &[1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn a_quote_is_shorter_when_the_datagram_was() {
    let mut bytes = Vec::new();
    let length = datagram(
        Ipv4Addr::LOCALHOST,
        Ipv4Addr::LOCALHOST,
        Protocol::UDP,
        &[1, 2, 3],
        0,
        &mut bytes,
    );
    let offending = Datagram::parse(&bytes[..length]).expect("a datagram");
    assert_eq!(quoted_len(offending), MIN_HEADER_LEN + 3);
}

#[test]
fn a_time_exceeded_reads_and_writes_its_code() {
    let quoted = [0x45u8; MIN_HEADER_LEN];
    let message = Message::TimeExceeded {
        code: 1,
        quoted: &quoted,
    };
    assert!(message.is_error());
    let mut buffer = [0u8; HEADER_LEN + MIN_HEADER_LEN];
    let mut writer = Writer::new(&mut buffer);
    message.write(&mut writer).expect("room");
    assert_eq!(Message::parse(&buffer), Ok(message));
    assert_eq!(buffer[0], 11);
    assert_eq!(buffer[1], 1);
}

#[test]
fn the_unreachable_codes_map_both_ways() {
    for (reason, code) in [
        (Unreachable::Network, 0u8),
        (Unreachable::Host, 1),
        (Unreachable::Protocol, 2),
        (Unreachable::Port, 3),
        (Unreachable::FragmentationNeeded, 4),
        (Unreachable::Other(9), 9),
    ] {
        assert_eq!(reason.get(), code, "{reason:?}");
        assert_eq!(Unreachable::new(code), reason, "{reason:?}");
    }
}

#[test]
fn a_type_this_crate_does_not_read_is_carried_and_not_written() {
    let mut bytes = [9u8, 0, 0, 0, 0, 0, 0, 0];
    let sum = net_wire::checksum(&bytes);
    bytes[2..4].copy_from_slice(&sum.to_be_bytes());
    let message = Message::parse(&bytes).expect("a message of a type we do not read");
    assert_eq!(
        message,
        Message::Other {
            message_type: 9,
            code: 0
        }
    );
    assert!(!message.is_error());
    let mut buffer = [0u8; 8];
    let mut writer = Writer::new(&mut buffer);
    assert_eq!(message.write(&mut writer), Err(IpError::BadIcmp));
}

#[test]
fn a_message_that_is_short_or_does_not_verify_is_refused() {
    for length in 0..HEADER_LEN {
        assert_eq!(
            Message::parse(&ECHO_REQUEST[..length]),
            Err(IpError::BadIcmp),
            "{length} bytes"
        );
    }
    let mut bad = ECHO_REQUEST;
    bad[8] = b'q';
    assert_eq!(Message::parse(&bad), Err(IpError::BadIcmp));
}

#[test]
fn a_message_that_does_not_fit_writes_nothing() {
    let message = Message::EchoReply {
        identifier: 1,
        sequence: 1,
        payload: b"ping",
    };
    let mut buffer = [0u8; HEADER_LEN + 3];
    let mut writer = Writer::new(&mut buffer);
    assert!(matches!(message.write(&mut writer), Err(IpError::Wire(_))));
    assert_eq!(writer.position(), 0);
}

#[test]
fn no_error_answers_an_error() {
    // A datagram carrying a destination-unreachable message.
    let inner = Message::DestinationUnreachable {
        reason: Unreachable::Port,
        quoted: &[0x45u8; MIN_HEADER_LEN],
    };
    let mut message = vec![0u8; HEADER_LEN + MIN_HEADER_LEN];
    let mut writer = Writer::new(&mut message);
    inner.write(&mut writer).expect("room");

    let mut bytes = Vec::new();
    let length = datagram(
        Ipv4Addr::new(192, 168, 1, 7),
        Ipv4Addr::new(192, 168, 1, 1),
        Protocol::ICMP,
        &message,
        0,
        &mut bytes,
    );
    let carrying = Datagram::parse(&bytes[..length]).expect("a datagram");
    assert!(!may_answer_with_error(carrying, false, INTERFACE_BROADCAST));

    // An echo request in the same place may be answered with an error,
    // because it is not one.
    let length = datagram(
        Ipv4Addr::new(192, 168, 1, 7),
        Ipv4Addr::new(192, 168, 1, 1),
        Protocol::ICMP,
        &ECHO_REQUEST,
        0,
        &mut bytes,
    );
    let request = Datagram::parse(&bytes[..length]).expect("a datagram");
    assert!(may_answer_with_error(request, false, INTERFACE_BROADCAST));
}

#[test]
fn no_error_answers_a_broadcast_or_a_multicast() {
    let mut bytes = Vec::new();
    for destination in [
        Ipv4Addr::BROADCAST,
        Ipv4Addr::new(224, 0, 0, 1),
        INTERFACE_BROADCAST,
    ] {
        let length = datagram(
            Ipv4Addr::new(192, 168, 1, 7),
            destination,
            Protocol::UDP,
            &[1, 2, 3, 4],
            0,
            &mut bytes,
        );
        let received = Datagram::parse(&bytes[..length]).expect("a datagram");
        assert!(
            !may_answer_with_error(received, false, INTERFACE_BROADCAST),
            "{destination}"
        );
    }
}

#[test]
fn no_error_answers_a_link_layer_broadcast_or_a_later_fragment() {
    let mut bytes = Vec::new();
    let length = datagram(
        Ipv4Addr::new(192, 168, 1, 7),
        Ipv4Addr::new(192, 168, 1, 1),
        Protocol::UDP,
        &[1, 2, 3, 4],
        0,
        &mut bytes,
    );
    let received = Datagram::parse(&bytes[..length]).expect("a datagram");
    // The same datagram, once as unicast and once as a link broadcast.
    assert!(may_answer_with_error(received, false, INTERFACE_BROADCAST));
    assert!(!may_answer_with_error(received, true, INTERFACE_BROADCAST));

    let length = datagram(
        Ipv4Addr::new(192, 168, 1, 7),
        Ipv4Addr::new(192, 168, 1, 1),
        Protocol::UDP,
        &[1, 2, 3, 4, 5, 6, 7, 8],
        8,
        &mut bytes,
    );
    let later = Datagram::parse(&bytes[..length]).expect("a fragment");
    assert!(!later.is_initial_fragment());
    assert!(!may_answer_with_error(later, false, INTERFACE_BROADCAST));
}

#[test]
fn no_error_answers_a_source_that_names_no_single_host() {
    let mut bytes = Vec::new();
    for source in [
        Ipv4Addr::UNSPECIFIED,
        Ipv4Addr::BROADCAST,
        Ipv4Addr::LOCALHOST,
        Ipv4Addr::new(224, 0, 0, 1),
    ] {
        let length = datagram(
            source,
            Ipv4Addr::new(192, 168, 1, 1),
            Protocol::UDP,
            &[1, 2, 3, 4],
            0,
            &mut bytes,
        );
        let received = Datagram::parse(&bytes[..length]).expect("a datagram");
        assert!(
            !may_answer_with_error(received, false, INTERFACE_BROADCAST),
            "{source}"
        );
    }
}

#[test]
fn the_token_bucket_stops_at_its_limit_and_comes_back_as_it_refills() {
    let limit = RateLimit::DEFAULT;
    assert_eq!(RateLimit::default(), limit);
    let mut bucket = TokenBucket::new(limit, Instant::ZERO);
    assert_eq!(bucket.available(Instant::ZERO), limit.capacity);

    for count in 0..limit.capacity {
        assert!(bucket.take(Instant::ZERO), "error {count}");
    }
    assert!(!bucket.take(Instant::ZERO));
    assert_eq!(bucket.available(Instant::ZERO), 0);

    // One interval, one allowance.
    let later = Instant::ZERO.saturating_add(limit.interval);
    assert_eq!(bucket.available(later), 1);
    assert!(bucket.take(later));
    assert!(!bucket.take(later));

    // And it never fills past its capacity, however long it waits.
    let much_later = Instant::ZERO.saturating_add(Duration::from_secs(3600));
    assert_eq!(bucket.available(much_later), limit.capacity);
}

#[test]
fn the_refill_keeps_the_remainder_so_the_rate_does_not_drift() {
    let limit = RateLimit {
        capacity: 2,
        interval: Duration::from_millis(100),
    };
    let mut bucket = TokenBucket::new(limit, Instant::ZERO);
    assert!(bucket.take(Instant::ZERO));
    assert!(bucket.take(Instant::ZERO));
    assert!(!bucket.take(Instant::ZERO));

    // Ninety milliseconds earn nothing, and the ninety are not thrown
    // away: ten more finish the interval.
    let ninety = Instant::ZERO.saturating_add(Duration::from_millis(90));
    assert!(!bucket.take(ninety));
    let hundred = Instant::ZERO.saturating_add(Duration::from_millis(100));
    assert!(bucket.take(hundred));
}

#[test]
fn an_interval_of_zero_fills_the_bucket_every_time() {
    let limit = RateLimit {
        capacity: 3,
        interval: Duration::ZERO,
    };
    let mut bucket = TokenBucket::new(limit, Instant::ZERO);
    for _ in 0..10 {
        assert!(bucket.take(Instant::ZERO));
    }
    assert_eq!(bucket.available(Instant::ZERO), 3);
}
