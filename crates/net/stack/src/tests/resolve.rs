// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Connecting to a list of addresses, and to a name.

#![allow(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "a test reads a frame at known offsets"
)]

use audhsos_time::Instant;
use crypto_rng::doubles::ScriptedRng;
use net_dns::{Name, RecordType};
use net_wire::{IpAddr, Ipv4Addr, Port, Protocol, Writer};

use crate::error::StackError;
use crate::resolve::Connecting;
use crate::stack::{Config, FRAME_LEN, Stack};
use crate::tests::harness::{
    HERE, MAC, PEER, PEER_MAC, ROUTER, arp_request_from, carries, datagram_of, dhcp_frame, dhcp_of,
    udp_from,
};

/// A script of bytes that do not repeat.
const SCRIPT: [u8; 512] = {
    let mut bytes = [0u8; 512];
    let mut at = 0usize;
    let mut value = 13u8;
    while at < 512 {
        bytes[at] = value;
        value = value.wrapping_add(29);
        at += 1;
    }
    bytes
};

/// A generator for the tests.
fn rng() -> ScriptedRng<'static> {
    ScriptedRng::new(&SCRIPT)
}

/// Everything the stack has to say at `now`.
fn drain<R: crypto_rng::Rng + ?Sized>(
    stack: &mut Stack<'_, 4, 2>,
    rx: Option<&[u8]>,
    now: Instant,
    rng: &mut R,
) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut tx = [0u8; FRAME_LEN];
    let mut input = rx;
    let mut turns = 0usize;
    loop {
        let answer = stack.poll(now, input.take(), &mut tx, rng).expect("a poll");
        turns = turns.saturating_add(1);
        assert!(turns <= 32, "one instant produced frames without end");
        if let Some(frame) = answer {
            out.push(frame.to_vec());
            continue;
        }
        if stack.poll_at(now).is_some_and(|at| at <= now) {
            continue;
        }
        break;
    }
    out
}

/// A stack that has been through the four-message exchange and knows the
/// router as its neighbor and as its name server.
fn leased<'a>(outgoing: &'a mut [u8], rng: &mut ScriptedRng<'_>) -> Stack<'a, 4, 2> {
    let now = Instant::from_micros(0);
    let mut stack = Stack::new(Config::new(MAC), outgoing);
    stack.configure(now);
    let frames = drain(&mut stack, None, now, rng);
    let xid = dhcp_of(&frames[0]).xid;
    drain(
        &mut stack,
        Some(&dhcp_frame(net_dhcp::MessageType::OFFER, xid)),
        now,
        rng,
    );
    drain(
        &mut stack,
        Some(&dhcp_frame(net_dhcp::MessageType::ACK, xid)),
        now,
        rng,
    );
    // The router and the peer both answer for themselves, so nothing
    // waits for ARP.
    for (hardware, address) in [(PEER_MAC, ROUTER), (PEER_MAC, PEER)] {
        drain(
            &mut stack,
            Some(&arp_request_from(hardware, address, HERE)),
            now,
            rng,
        );
    }
    stack
}

/// The DNS answer the fake server sends, for `name` of `record_type`.
fn dns_answer(
    port: u16,
    id: u16,
    name: &str,
    record_type: RecordType,
    address: Option<Ipv4Addr>,
) -> Vec<u8> {
    let owner = Name::from_ascii(name).expect("a name");
    let header = net_dns::Header {
        id,
        flags: net_dns::Flags::QUERY.as_response(),
        questions: 1,
        answers: u16::from(address.is_some()),
        authorities: 0,
        additionals: 0,
    };
    let mut bytes = [0u8; 512];
    let mut writer = Writer::new(&mut bytes);
    header.write(&mut writer).expect("room");
    net_dns::Question::new(owner, record_type)
        .write(&mut writer)
        .expect("room");
    if let Some(address) = address {
        net_dns::Record {
            name: owner,
            record_type,
            class: net_dns::Class::IN,
            ttl: 300,
            data: net_dns::RecordData::A(address),
        }
        .write(&mut writer)
        .expect("room");
    }
    let len = writer.position();
    udp_from(ROUTER, HERE, 53, port, &bytes[..len])
}

#[test]
fn a_list_of_addresses_is_tried_in_the_order_it_is_given() {
    let mut outgoing = [0u8; 8192];
    let mut send = [0u8; 512];
    let mut receive = [0u8; 512];
    let mut rng = rng();
    let mut stack: Stack<'_, 4, 2> = leased(&mut outgoing, &mut rng);
    let now = Instant::from_micros(0);
    let first = Ipv4Addr::new(203, 0, 113, 1);
    let second = PEER;

    stack
        .connect_to_any(
            &[IpAddr::V4(first), IpAddr::V4(second)],
            Port::new(80),
            &mut rng,
            &mut send,
            &mut receive,
            now,
        )
        .expect("a connection");
    assert!(matches!(stack.connecting(), Connecting::Open(_)));
    let frames = drain(&mut stack, None, now, &mut rng);
    // The first candidate is off this link, so what goes out is the
    // question of who the router is — which is already known — and the
    // `SYN` behind it.
    let syn = frames
        .iter()
        .find(|frame| carries(frame, Protocol::TCP))
        .expect("a segment");
    assert_eq!(datagram_of(syn).destination(), first);

    // The first candidate refuses, and the second is tried.
    let reset = refuse(syn);
    let frames = drain(&mut stack, Some(&reset), now, &mut rng);
    let syn = frames
        .iter()
        .find(|frame| carries(frame, Protocol::TCP))
        .expect("a segment");
    assert_eq!(datagram_of(syn).destination(), second);
}

/// The reset that refuses the `SYN` inside `frame`.
///
/// A reset is believed in `SYN-SENT` only when it acknowledges the `SYN`
/// (RFC 9293, section 3.10.7.3), which is what keeps a blind one from
/// tearing a connection down.
fn refuse(frame: &[u8]) -> Vec<u8> {
    let datagram = datagram_of(frame);
    let segment = net_tcp::Segment::parse(
        datagram.payload(),
        IpAddr::V4(datagram.source()),
        IpAddr::V4(datagram.destination()),
    )
    .expect("a segment");
    let mut reset = net_tcp::Segment::new(
        segment.destination_port,
        segment.source_port,
        net_tcp::Flags::RST.with(net_tcp::Flags::ACK),
    );
    reset.ack = segment.seq.add(1);
    let mut bytes = vec![0u8; 128];
    let mut writer = Writer::new(&mut bytes);
    reset
        .write(
            &mut writer,
            IpAddr::V4(datagram.destination()),
            IpAddr::V4(datagram.source()),
        )
        .expect("room");
    let len = writer.position();
    crate::tests::harness::frame(
        MAC,
        PEER_MAC,
        net_wire::EtherType::IPV4,
        &crate::tests::harness::ipv4(
            datagram.destination(),
            datagram.source(),
            Protocol::TCP,
            &bytes[..len],
        ),
    )
}

#[test]
fn an_empty_list_reaches_nobody_and_a_second_connection_waits() {
    let mut outgoing = [0u8; 8192];
    let mut send = [0u8; 512];
    let mut receive = [0u8; 512];
    let mut rng = rng();
    let mut stack: Stack<'_, 4, 2> = leased(&mut outgoing, &mut rng);
    let now = Instant::from_micros(0);
    assert_eq!(stack.connecting(), Connecting::Idle);
    stack
        .connect_to_any(
            &[IpAddr::V4(PEER)],
            Port::new(80),
            &mut rng,
            &mut send,
            &mut receive,
            now,
        )
        .expect("a connection");
    let mut other_send = [0u8; 512];
    let mut other_receive = [0u8; 512];
    assert_eq!(
        stack.connect_to_any(
            &[IpAddr::V4(PEER)],
            Port::new(81),
            &mut rng,
            &mut other_send,
            &mut other_receive,
            now
        ),
        Err(StackError::Busy)
    );
    let handle = stack.take_connection().expect("a connection");
    assert_eq!(stack.connecting(), Connecting::Idle);
    assert!(stack.connection(handle).is_ok());
}

#[test]
fn every_candidate_that_refuses_leaves_the_attempt_failed_and_the_buffers_come_back() {
    let mut outgoing = [0u8; 8192];
    let mut send = [0u8; 512];
    let mut receive = [0u8; 512];
    let mut rng = rng();
    let mut stack: Stack<'_, 4, 2> = leased(&mut outgoing, &mut rng);
    let now = Instant::from_micros(0);
    stack
        .connect_to_any(
            &[IpAddr::V4(PEER)],
            Port::new(80),
            &mut rng,
            &mut send,
            &mut receive,
            now,
        )
        .expect("a connection");
    let frames = drain(&mut stack, None, now, &mut rng);
    let syn = frames
        .iter()
        .find(|frame| carries(frame, Protocol::TCP))
        .expect("a segment");
    let reset = refuse(syn);
    drain(&mut stack, Some(&reset), now, &mut rng);
    assert_eq!(stack.connecting(), Connecting::Failed);
    let buffers = stack.abandon().expect("the buffers");
    assert_eq!(buffers.0.len(), 512);
    assert_eq!(stack.connecting(), Connecting::Idle);
}

#[test]
fn a_name_is_resolved_and_then_connected_to() {
    let mut outgoing = [0u8; 8192];
    let mut datagrams = [0u8; 1024];
    let mut send = [0u8; 512];
    let mut receive = [0u8; 512];
    let mut rng = rng();
    let mut stack: Stack<'_, 4, 2> = leased(&mut outgoing, &mut rng);
    let now = Instant::from_micros(0);
    let socket = stack
        .bind_ephemeral(Some(IpAddr::V4(HERE)), &mut rng, &mut datagrams)
        .expect("a socket");
    let port = stack.socket(socket).expect("it").port().get();
    let name = Name::from_ascii("example.com").expect("a name");
    stack
        .connect_to_name(&name, socket, Port::new(80), &mut send, &mut receive, now)
        .expect("a resolution");
    assert_eq!(stack.connecting(), Connecting::Trying);

    // Two questions go out, one of each type.
    let frames = drain(&mut stack, None, now, &mut rng);
    let queries: Vec<_> = frames
        .iter()
        .filter(|frame| carries(frame, Protocol::UDP))
        .collect();
    assert_eq!(queries.len(), 2);
    let ids: Vec<u16> = queries.iter().map(|frame| dns_id(frame)).collect();

    // Both are answered, one with an address and one with nothing.
    let answer = dns_answer(port, ids[0], "example.com", RecordType::A, Some(PEER));
    drain(&mut stack, Some(&answer), now, &mut rng);
    let empty = dns_answer(port, ids[1], "example.com", RecordType::AAAA, None);
    let frames = drain(&mut stack, Some(&empty), now, &mut rng);

    // And the connection to what came back goes out.
    let syn = frames
        .iter()
        .find(|frame| carries(frame, Protocol::TCP))
        .expect("a segment");
    assert_eq!(datagram_of(syn).destination(), PEER);
    assert!(matches!(stack.connecting(), Connecting::Open(_)));
    assert!(!stack.resolving());
}

/// The transaction id of the DNS query inside `frame`.
fn dns_id(frame: &[u8]) -> u16 {
    let datagram = datagram_of(frame);
    let udp = net_udp::Datagram::parse(
        datagram.payload(),
        IpAddr::V4(datagram.source()),
        IpAddr::V4(datagram.destination()),
    )
    .expect("a datagram");
    net_dns::Message::parse(udp.payload)
        .expect("a message")
        .header
        .id
}

#[test]
fn a_resolution_with_no_server_to_ask_is_refused() {
    let mut outgoing = [0u8; 4096];
    let mut datagrams = [0u8; 256];
    let mut rng = rng();
    let mut stack: Stack<'_, 4, 2> = Stack::new(Config::new(MAC), &mut outgoing);
    let _ = &mut rng;
    stack.add_address(IpAddr::V4(HERE)).expect("room");
    let socket = stack
        .bind(Some(IpAddr::V4(HERE)), Port::new(5300), &mut datagrams)
        .expect("a socket");
    let name = Name::from_ascii("example.com").expect("a name");
    assert_eq!(
        stack.resolve(&name, socket, Instant::from_micros(0)),
        Err(StackError::Unresolved)
    );
    assert!(stack.resolution().is_none());
    assert_eq!(stack.resolved().count(), 0);
}

#[test]
fn a_second_resolution_waits_for_the_first() {
    let mut outgoing = [0u8; 8192];
    let mut datagrams = [0u8; 1024];
    let mut rng = rng();
    let mut stack: Stack<'_, 4, 2> = leased(&mut outgoing, &mut rng);
    let now = Instant::from_micros(0);
    let socket = stack
        .bind_ephemeral(Some(IpAddr::V4(HERE)), &mut rng, &mut datagrams)
        .expect("a socket");
    let name = Name::from_ascii("example.com").expect("a name");
    stack.resolve(&name, socket, now).expect("a resolution");
    assert_eq!(stack.resolve(&name, socket, now), Err(StackError::Busy));
    assert_eq!(stack.resolution(), Some(net_dns::Status::Asking));
    stack.forget_resolution();
    assert!(stack.resolution().is_none());
}

#[test]
fn a_connection_to_no_address_at_all_is_refused() {
    let mut outgoing = [0u8; 8192];
    let mut send = [0u8; 512];
    let mut receive = [0u8; 512];
    let mut rng = rng();
    let mut stack: Stack<'_, 4, 2> = leased(&mut outgoing, &mut rng);
    let now = Instant::from_micros(0);
    assert_eq!(
        stack.connect_to_any(&[], Port::new(80), &mut rng, &mut send, &mut receive, now),
        Err(StackError::Unresolved)
    );
    assert_eq!(stack.connecting(), Connecting::Idle);
    assert!(stack.abandon().is_none());
}

#[test]
fn a_server_of_the_other_family_is_not_one_this_resolution_asks() {
    let mut outgoing = [0u8; 8192];
    let mut datagrams = [0u8; 1024];
    let mut rng = rng();
    let mut stack: Stack<'_, 4, 2> = leased(&mut outgoing, &mut rng);
    let now = Instant::from_micros(0);
    // A router advertisement adds a name server of the other family; the
    // lease already added one of this one.
    let mut options =
        crate::tests::harness::prefix_option(crate::tests::harness::PREFIX, 7200, 3600);
    options.extend_from_slice(&crate::tests::harness::rdnss_option(
        600,
        crate::tests::harness::PEER6,
    ));
    drain(
        &mut stack,
        Some(&crate::tests::harness::router_advertisement_frame(
            1800, &options,
        )),
        now,
        &mut rng,
    );
    assert_eq!(stack.name_servers().count(), 2);

    let socket = stack
        .bind_ephemeral(Some(IpAddr::V4(HERE)), &mut rng, &mut datagrams)
        .expect("a socket");
    let name = Name::from_ascii("example.com").expect("a name");
    stack.resolve(&name, socket, now).expect("a resolution");
    // Both questions went to the one server of this resolver's family.
    let frames = drain(&mut stack, None, now, &mut rng);
    let queries: Vec<_> = frames
        .iter()
        .filter(|frame| carries(frame, Protocol::UDP))
        .collect();
    assert_eq!(queries.len(), 2);
    for query in queries {
        assert_eq!(datagram_of(query).destination(), ROUTER);
    }
}

#[test]
fn a_connection_by_name_waits_for_the_one_being_made() {
    let mut outgoing = [0u8; 8192];
    let mut datagrams = [0u8; 1024];
    let mut send = [0u8; 512];
    let mut receive = [0u8; 512];
    let mut rng = rng();
    let mut stack: Stack<'_, 4, 2> = leased(&mut outgoing, &mut rng);
    let now = Instant::from_micros(0);
    let socket = stack
        .bind_ephemeral(Some(IpAddr::V4(HERE)), &mut rng, &mut datagrams)
        .expect("a socket");
    stack
        .connect_to_any(
            &[IpAddr::V4(PEER)],
            Port::new(80),
            &mut rng,
            &mut send,
            &mut receive,
            now,
        )
        .expect("a connection");
    let mut other_send = [0u8; 512];
    let mut other_receive = [0u8; 512];
    let name = Name::from_ascii("example.com").expect("a name");
    assert_eq!(
        stack.connect_to_name(
            &name,
            socket,
            Port::new(80),
            &mut other_send,
            &mut other_receive,
            now
        ),
        Err(StackError::Busy)
    );
}

#[test]
fn a_name_that_resolves_to_nothing_leaves_the_connection_failed() {
    let mut outgoing = [0u8; 8192];
    let mut datagrams = [0u8; 1024];
    let mut send = [0u8; 512];
    let mut receive = [0u8; 512];
    let mut rng = rng();
    let mut stack: Stack<'_, 4, 2> = leased(&mut outgoing, &mut rng);
    let now = Instant::from_micros(0);
    let socket = stack
        .bind_ephemeral(Some(IpAddr::V4(HERE)), &mut rng, &mut datagrams)
        .expect("a socket");
    let port = stack.socket(socket).expect("it").port().get();
    let name = Name::from_ascii("nowhere.example").expect("a name");
    stack
        .connect_to_name(&name, socket, Port::new(80), &mut send, &mut receive, now)
        .expect("a resolution");
    let frames = drain(&mut stack, None, now, &mut rng);
    let ids: Vec<u16> = frames
        .iter()
        .filter(|frame| carries(frame, Protocol::UDP))
        .map(|frame| dns_id(frame))
        .collect();
    // Both questions come back with nothing, so there is nothing to
    // connect to.
    // The questions go out in the order of the two record types.
    for (id, kind) in ids.iter().zip([RecordType::A, RecordType::AAAA]) {
        let answer = dns_answer(port, *id, "nowhere.example", kind, None);
        drain(&mut stack, Some(&answer), now, &mut rng);
    }
    assert_eq!(stack.connecting(), Connecting::Failed);
    assert!(stack.abandon().is_some());
}
