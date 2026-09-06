// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The IPv6 half of the facade: what a packet reaches, what Neighbor
//! Discovery teaches, and what a router advertisement configures.

#![allow(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "a test reads a frame at known offsets"
)]

use audhsos_time::{Duration, Instant};
use crypto_rng::doubles::ScriptedRng;
use net_wire::{EtherType, IpAddr, IpCidr, Ipv6Addr, Ipv6Cidr, MacAddr, Port, Protocol};

use crate::drive::solicited_node;
use crate::stack::{Config, FRAME_LEN, Stack};
use crate::tests::harness::{
    ALL_NODES, DISCOVERY_HOP_LIMIT, HERE6, MAC, PEER_MAC, PEER6, PREFIX, advertisement_frame,
    carries6, echo6_frame, ipv6_frame, packet_of, prefix_option, rdnss_option,
    router_advertisement_frame, solicitation_frame, syn6_frame, udp6_frame,
};

/// A script of bytes that do not repeat.
const SCRIPT: [u8; 512] = {
    let mut bytes = [0u8; 512];
    let mut at = 0usize;
    let mut value = 3u8;
    while at < 512 {
        bytes[at] = value;
        value = value.wrapping_add(11);
        at += 1;
    }
    bytes
};

/// A generator for the tests.
fn rng() -> ScriptedRng<'static> {
    ScriptedRng::new(&SCRIPT)
}

/// The instant a test starts at.
fn start() -> Instant {
    Instant::from_micros(0)
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

/// A stack that holds an IPv6 address and a route to the prefix.
fn configured(outgoing: &mut [u8]) -> Stack<'_, 4, 2> {
    let mut stack = Stack::new(Config::new(MAC), outgoing);
    stack.add_address(IpAddr::V6(HERE6)).expect("room");
    let prefix = Ipv6Cidr::new(PREFIX, 64).expect("a prefix");
    stack
        .add_route(net_ip::Route::on_link(IpCidr::V6(prefix)))
        .expect("room");
    stack
}

#[test]
fn an_echo_request_over_ipv6_is_answered() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    // The neighbor answers for itself, so the reply does not wait.
    drain(
        &mut stack,
        Some(&advertisement_frame(PEER6, HERE6, PEER6, PEER_MAC, false)),
        start(),
        &mut rng,
    );
    let frames = drain(
        &mut stack,
        Some(&echo6_frame(PEER6, HERE6)),
        start(),
        &mut rng,
    );
    assert_eq!(frames.len(), 1);
    let packet = packet_of(&frames[0]);
    assert_eq!(packet.source(), HERE6);
    assert_eq!(packet.destination(), PEER6);
    assert_eq!(packet.next_header(), Protocol::ICMPV6);
    let message = net_ipv6::icmp::Message::parse(packet.payload()).expect("a message");
    assert!(matches!(message, net_ipv6::icmp::Message::EchoReply { .. }));
}

#[test]
fn a_packet_for_a_foreign_address_is_dropped() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let stranger = Ipv6Addr::from_octets([
        0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x99,
    ]);
    assert!(
        drain(
            &mut stack,
            Some(&echo6_frame(PEER6, stranger)),
            start(),
            &mut rng
        )
        .is_empty()
    );
}

#[test]
fn a_datagram_over_ipv6_reaches_the_socket_that_holds_its_port() {
    let mut outgoing = [0u8; 8192];
    let mut datagrams = [0u8; 512];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let socket = stack
        .bind(Some(IpAddr::V6(HERE6)), Port::new(4711), &mut datagrams)
        .expect("a socket");
    let frame = udp6_frame(PEER6, HERE6, 1024, 4711, b"hello");
    assert!(drain(&mut stack, Some(&frame), start(), &mut rng).is_empty());
    let received = stack.socket(socket).expect("it");
    let datagram = received.peek().expect("a datagram");
    assert_eq!(datagram.payload, b"hello");
    assert_eq!(datagram.source, IpAddr::V6(PEER6));
}

#[test]
fn a_segment_over_ipv6_to_no_connection_is_answered_with_a_reset() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    drain(
        &mut stack,
        Some(&advertisement_frame(PEER6, HERE6, PEER6, PEER_MAC, false)),
        start(),
        &mut rng,
    );
    let frames = drain(
        &mut stack,
        Some(&syn6_frame(PEER6, HERE6, 1024, 80)),
        start(),
        &mut rng,
    );
    let reset = frames
        .iter()
        .find(|frame| carries6(frame, Protocol::TCP))
        .expect("a segment");
    let packet = packet_of(reset);
    let segment = net_tcp::Segment::parse(
        packet.payload(),
        IpAddr::V6(packet.source()),
        IpAddr::V6(packet.destination()),
    )
    .expect("a segment");
    assert!(segment.flags.has(net_tcp::Flags::RST));
}

#[test]
fn a_solicitation_for_this_hosts_address_is_answered() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let frame = solicitation_frame(PEER6, solicited_node(HERE6), HERE6, Some(PEER_MAC));
    let frames = drain(&mut stack, Some(&frame), start(), &mut rng);
    let answer = frames
        .iter()
        .find(|frame| carries6(frame, Protocol::ICMPV6))
        .expect("an advertisement");
    let packet = packet_of(answer);
    assert_eq!(packet.hop_limit(), DISCOVERY_HOP_LIMIT);
    let discovery = net_ipv6::ndp::receive(packet, packet.payload()).expect("a message");
    let net_ipv6::Discovery::NeighborAdvertisement {
        target, solicited, ..
    } = discovery
    else {
        panic!("not an advertisement");
    };
    assert_eq!(target, HERE6);
    assert!(solicited);
    // And the sender is in the cache.
    assert_eq!(
        stack.neighbors().hardware(IpAddr::V6(PEER6)),
        Some(PEER_MAC)
    );
}

#[test]
fn a_solicitation_for_another_address_is_not_answered() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let other = Ipv6Addr::from_octets([
        0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x77,
    ]);
    let frame = solicitation_frame(PEER6, solicited_node(other), other, Some(PEER_MAC));
    assert!(drain(&mut stack, Some(&frame), start(), &mut rng).is_empty());
}

#[test]
fn a_discovery_message_whose_hop_limit_is_not_255_is_not_one() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    // The same solicitation, one hop short of what the protocol requires.
    let mut message = vec![0u8; 128];
    let mut writer = net_wire::Writer::new(&mut message);
    net_ipv6::ndp::write_neighbor_solicitation(
        &mut writer,
        PEER6,
        solicited_node(HERE6),
        HERE6,
        Some(PEER_MAC),
    )
    .expect("room");
    let len = writer.position();
    let frame = ipv6_frame(
        PEER6,
        solicited_node(HERE6),
        Protocol::ICMPV6,
        254,
        &message[..len],
    );
    assert!(drain(&mut stack, Some(&frame), start(), &mut rng).is_empty());
    assert_eq!(stack.neighbors().hardware(IpAddr::V6(PEER6)), None);
}

#[test]
fn a_send_to_an_unknown_neighbor_asks_the_solicited_node_group_and_then_goes() {
    let mut outgoing = [0u8; 8192];
    let mut datagrams = [0u8; 512];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let socket = stack
        .bind(Some(IpAddr::V6(HERE6)), Port::new(4711), &mut datagrams)
        .expect("a socket");
    stack
        .send_to(socket, IpAddr::V6(PEER6), Port::new(53), b"query", start())
        .expect("a send");
    let frames = drain(&mut stack, None, start(), &mut rng);
    let question = frames
        .iter()
        .find(|frame| carries6(frame, Protocol::ICMPV6))
        .expect("a solicitation");
    assert_eq!(packet_of(question).destination(), solicited_node(PEER6));

    // The answer arrives and the datagram follows it.
    let frames = drain(
        &mut stack,
        Some(&advertisement_frame(PEER6, HERE6, PEER6, PEER_MAC, true)),
        start(),
        &mut rng,
    );
    let sent = frames
        .iter()
        .find(|frame| carries6(frame, Protocol::UDP))
        .expect("a datagram");
    let packet = packet_of(sent);
    let udp = net_udp::Datagram::parse(
        packet.payload(),
        IpAddr::V6(packet.source()),
        IpAddr::V6(packet.destination()),
    )
    .expect("a datagram");
    assert_eq!(udp.payload, b"query");
}

#[test]
fn a_router_advertisement_configures_the_interface_after_the_address_is_checked() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = Stack::new(Config::new(MAC), &mut outgoing);
    let mut rng = rng();
    let mut options = prefix_option(PREFIX, 7200, 3600);
    options.extend_from_slice(&rdnss_option(600, PEER6));
    let frames = drain(
        &mut stack,
        Some(&router_advertisement_frame(1800, &options)),
        start(),
        &mut rng,
    );

    // The prefix and the router are routes at once; the address is not
    // this host's until nobody has claimed it.
    assert_eq!(stack.routes().len(), 2);
    assert_eq!(stack.addresses().count(), 0);
    assert_eq!(
        stack.name_servers().collect::<Vec<_>>(),
        vec![IpAddr::V6(PEER6)]
    );

    // The probe goes to the solicited-node group of the address being
    // checked, from nowhere at all.
    let probe = frames
        .iter()
        .find(|frame| carries6(frame, Protocol::ICMPV6))
        .expect("a probe");
    let packet = packet_of(probe);
    assert_eq!(packet.source(), Ipv6Addr::UNSPECIFIED);
    assert_eq!(packet.hop_limit(), DISCOVERY_HOP_LIMIT);
    assert_eq!(
        destination_hardware(probe),
        net_eth::multicast_hardware(packet.destination())
    );

    // Nobody claims it, so it becomes this host's.
    let later = start().saturating_add(Duration::from_secs(2));
    drain(&mut stack, None, later, &mut rng);
    assert_eq!(stack.addresses().count(), 1);
    let held = stack.addresses().next().expect("an address");
    assert!(matches!(held, IpAddr::V6(_)));
}

/// The destination hardware address of a frame.
fn destination_hardware(frame: &[u8]) -> MacAddr {
    MacAddr::new([frame[0], frame[1], frame[2], frame[3], frame[4], frame[5]])
}

#[test]
fn an_address_somebody_else_claims_is_not_used() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = Stack::new(Config::new(MAC), &mut outgoing);
    let mut rng = rng();
    let options = prefix_option(PREFIX, 7200, 3600);
    let frames = drain(
        &mut stack,
        Some(&router_advertisement_frame(1800, &options)),
        start(),
        &mut rng,
    );
    let probe = frames
        .iter()
        .find(|frame| carries6(frame, Protocol::ICMPV6))
        .expect("a probe");
    let tentative = net_ipv6::ndp::receive(packet_of(probe), packet_of(probe).payload())
        .ok()
        .and_then(|discovery| match discovery {
            net_ipv6::Discovery::NeighborSolicitation { target, .. } => Some(target),
            _ => None,
        })
        .expect("a target");

    // Somebody answers for it, so this host is left without one.
    drain(
        &mut stack,
        Some(&advertisement_frame(
            tentative, ALL_NODES, tentative, PEER_MAC, false,
        )),
        start(),
        &mut rng,
    );
    let later = start().saturating_add(Duration::from_secs(2));
    drain(&mut stack, None, later, &mut rng);
    assert_eq!(stack.addresses().count(), 0);
}

#[test]
fn a_prefix_that_runs_out_takes_its_route_and_its_address_with_it() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = Stack::new(Config::new(MAC), &mut outgoing);
    let mut rng = rng();
    let options = prefix_option(PREFIX, 5, 4);
    drain(
        &mut stack,
        Some(&router_advertisement_frame(1800, &options)),
        start(),
        &mut rng,
    );
    let checked = start().saturating_add(Duration::from_secs(2));
    drain(&mut stack, None, checked, &mut rng);
    assert_eq!(stack.addresses().count(), 1);
    assert!(!stack.routes().is_empty());

    // The prefix was valid for five seconds and is gone.
    let over = start().saturating_add(Duration::from_secs(6));
    drain(&mut stack, None, over, &mut rng);
    assert_eq!(stack.addresses().count(), 0);
}

#[test]
fn a_multicast_packet_is_taken_and_one_for_a_stranger_is_not() {
    let mut outgoing = [0u8; 8192];
    let mut datagrams = [0u8; 512];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let socket = stack
        .bind(None, Port::new(4711), &mut datagrams)
        .expect("a socket");
    let frame = udp6_frame(PEER6, ALL_NODES, 1024, 4711, b"everyone");
    drain(&mut stack, Some(&frame), start(), &mut rng);
    assert_eq!(stack.socket(socket).expect("it").len(), 1);
}

#[test]
fn a_frame_of_a_type_this_stack_does_not_read_is_dropped() {
    let mut outgoing = [0u8; 4096];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let frame = crate::tests::harness::frame(
        MAC,
        PEER_MAC,
        EtherType::new(0x88CC),
        b"a link discovery frame",
    );
    assert!(drain(&mut stack, Some(&frame), start(), &mut rng).is_empty());
    // And so are bytes that are no frame at all.
    assert!(drain(&mut stack, Some(&[0u8; 4]), start(), &mut rng).is_empty());
}

#[test]
fn a_packet_that_is_not_one_is_dropped() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    // A frame that claims IPv6 and carries too little of it.
    let broken = crate::tests::harness::frame(MAC, PEER_MAC, EtherType::IPV6, &[0x60u8; 8]);
    assert!(drain(&mut stack, Some(&broken), start(), &mut rng).is_empty());
    // A packet whose extension chain cannot be walked: a next header of
    // hop-by-hop options and nothing behind it.
    let broken = ipv6_frame(PEER6, HERE6, Protocol::HOP_BY_HOP, 64, &[0u8; 4]);
    assert!(drain(&mut stack, Some(&broken), start(), &mut rng).is_empty());
    // And an `ICMPv6` message at the discovery hop limit that is no
    // discovery message at all is read as the message it is.
    drain(
        &mut stack,
        Some(&advertisement_frame(PEER6, HERE6, PEER6, PEER_MAC, false)),
        start(),
        &mut rng,
    );
    let mut message = vec![128u8, 0, 0, 0, 0, 9, 0, 1];
    message.extend_from_slice(b"ping");
    crate::tests::harness::checksummed(PEER6, HERE6, &mut message);
    let frame = ipv6_frame(
        PEER6,
        HERE6,
        Protocol::ICMPV6,
        DISCOVERY_HOP_LIMIT,
        &message,
    );
    assert_eq!(drain(&mut stack, Some(&frame), start(), &mut rng).len(), 1);
}

#[test]
fn a_duplicate_address_probe_for_this_hosts_address_is_answered_to_everyone() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let frame = solicitation_frame(Ipv6Addr::UNSPECIFIED, solicited_node(HERE6), HERE6, None);
    let frames = drain(&mut stack, Some(&frame), start(), &mut rng);
    let answer = frames
        .iter()
        .find(|frame| carries6(frame, Protocol::ICMPV6))
        .expect("an advertisement");
    let packet = packet_of(answer);
    assert_eq!(packet.destination(), ALL_NODES);
    let discovery = net_ipv6::ndp::receive(packet, packet.payload()).expect("a message");
    let net_ipv6::Discovery::NeighborAdvertisement { solicited, .. } = discovery else {
        panic!("not an advertisement");
    };
    // A probe is not a solicitation this host was asked by, so the answer
    // is not marked as one.
    assert!(!solicited);
    // The sender taught the cache nothing: it has no address yet.
    assert_eq!(
        stack
            .neighbors()
            .hardware(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
        None
    );
}

#[test]
fn a_second_advertisement_while_an_address_is_being_checked_changes_nothing() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = Stack::new(Config::new(MAC), &mut outgoing);
    let mut rng = rng();
    let options = prefix_option(PREFIX, 7200, 3600);
    drain(
        &mut stack,
        Some(&router_advertisement_frame(1800, &options)),
        start(),
        &mut rng,
    );
    // The check is running, so the same advertisement again starts no
    // second one.
    let frames = drain(
        &mut stack,
        Some(&router_advertisement_frame(1800, &options)),
        start(),
        &mut rng,
    );
    assert!(frames.is_empty());
    let later = start().saturating_add(Duration::from_secs(2));
    drain(&mut stack, None, later, &mut rng);
    assert_eq!(stack.addresses().count(), 1);
    // And once the address is this host's, a further advertisement starts
    // nothing either.
    let frames = drain(
        &mut stack,
        Some(&router_advertisement_frame(1800, &options)),
        later,
        &mut rng,
    );
    assert!(frames.is_empty());
    assert_eq!(stack.addresses().count(), 1);
}

#[test]
fn an_advertisement_of_a_prefix_no_address_is_formed_from_configures_a_route_and_no_more() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = Stack::new(Config::new(MAC), &mut outgoing);
    let mut rng = rng();
    // An on-link prefix without the autonomous bit: a route and nothing
    // to check.
    let mut option = vec![3u8, 4, 64, 0x80];
    option.extend_from_slice(&7200u32.to_be_bytes());
    option.extend_from_slice(&3600u32.to_be_bytes());
    option.extend_from_slice(&[0; 4]);
    option.extend_from_slice(&PREFIX.octets());
    let frames = drain(
        &mut stack,
        Some(&router_advertisement_frame(1800, &option)),
        start(),
        &mut rng,
    );
    assert!(frames.is_empty());
    assert_eq!(stack.addresses().count(), 0);
    assert!(!stack.routes().is_empty());
}

#[test]
fn an_interface_with_no_address_answers_no_multicast_either() {
    let mut outgoing = [0u8; 4096];
    let mut stack: Stack<'_, 4, 2> = Stack::new(Config::new(MAC), &mut outgoing);
    let mut rng = rng();
    let frame = echo6_frame(PEER6, ALL_NODES);
    assert!(drain(&mut stack, Some(&frame), start(), &mut rng).is_empty());
}
