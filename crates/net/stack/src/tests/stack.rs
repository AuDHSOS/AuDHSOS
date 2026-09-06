// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The facade: what a frame reaches, what a handle is worth, and when the
//! stack next has work.

#![allow(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "a test reads a frame at known offsets"
)]

use audhsos_time::{Duration, Instant};
use crypto_rng::doubles::ScriptedRng;
use net_wire::{EtherType, IpAddr, IpCidr, Ipv4Addr, Ipv4Cidr, MacAddr, Port, Protocol};

use crate::error::StackError;
use crate::stack::{Config, FRAME_LEN, Stack};
use crate::tests::harness::{
    HERE, MAC, MASK, PEER, PEER_MAC, ROUTER, arp_of, arp_request, arp_request_from, carries,
    datagram_of, destination_of, dhcp_frame, dhcp_of, echo_frame, ether_type_of, ipv4_frame,
    syn_frame, udp_frame,
};

/// A script of bytes that do not repeat.
const SCRIPT: [u8; 512] = {
    let mut bytes = [0u8; 512];
    let mut at = 0usize;
    let mut value = 7u8;
    while at < 512 {
        bytes[at] = value;
        value = value.wrapping_add(43);
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

/// Everything the stack has to say at `now`, one frame per call.
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
        // A poll that produced nothing may still have left work due now,
        // which is what `poll_at` is for.
        if stack.poll_at(now).is_some_and(|at| at <= now) {
            continue;
        }
        break;
    }
    out
}

/// A stack that already has this host's address and the link's route.
fn configured(outgoing: &mut [u8]) -> Stack<'_, 4, 2> {
    let mut stack = Stack::new(Config::new(MAC), outgoing);
    stack.add_address(IpAddr::V4(HERE)).expect("room");
    let network = Ipv4Cidr::new(Ipv4Addr::new(192, 168, 1, 0), 24).expect("a network");
    stack
        .add_route(net_ip::Route::on_link(IpCidr::V4(network)))
        .expect("room");
    stack
}

#[test]
fn an_arp_request_for_this_hosts_address_is_answered() {
    let mut outgoing = [0u8; 4096];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let frames = drain(&mut stack, Some(&arp_request(HERE)), start(), &mut rng);
    assert_eq!(frames.len(), 1);
    let reply = &frames[0];
    assert_eq!(ether_type_of(reply), EtherType::ARP);
    assert_eq!(destination_of(reply), PEER_MAC);
    let packet = arp_of(reply);
    assert_eq!(packet.operation, net_eth::Operation::Reply);
    assert_eq!(packet.sender_protocol, HERE);
    assert_eq!(packet.sender_hardware, MAC);
    assert_eq!(packet.target_protocol, PEER);
    // And the sender is in the cache now.
    assert_eq!(stack.neighbors().hardware(IpAddr::V4(PEER)), Some(PEER_MAC));
}

#[test]
fn an_interface_without_an_address_answers_no_arp_request_and_sends_no_datagram() {
    let mut outgoing = [0u8; 4096];
    let mut stack: Stack<'_, 4, 2> = Stack::new(Config::new(MAC), &mut outgoing);
    let mut rng = rng();
    assert!(drain(&mut stack, Some(&arp_request(HERE)), start(), &mut rng).is_empty());
    assert!(drain(&mut stack, Some(&echo_frame(HERE)), start(), &mut rng).is_empty());
    assert_eq!(stack.addresses().count(), 0);
    // A socket may be bound, and a send from it has nowhere to leave
    // from.
    let mut datagrams = [0u8; 256];
    let socket = stack
        .bind(None, Port::new(9999), &mut datagrams)
        .expect("a socket");
    assert_eq!(
        stack.send_to(socket, IpAddr::V4(PEER), Port::new(53), b"x", start()),
        Err(StackError::NoAddress)
    );
}

#[test]
fn an_arp_request_for_another_address_is_not_answered() {
    let mut outgoing = [0u8; 4096];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let other = Ipv4Addr::new(192, 168, 1, 77);
    assert!(drain(&mut stack, Some(&arp_request(other)), start(), &mut rng).is_empty());
}

#[test]
fn a_frame_for_another_station_is_not_read_at_all() {
    let mut outgoing = [0u8; 4096];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let stranger = MacAddr::new([2, 2, 2, 2, 2, 2]);
    let frame = ipv4_frame(stranger, PEER, HERE, Protocol::ICMP, b"whatever");
    assert!(drain(&mut stack, Some(&frame), start(), &mut rng).is_empty());
}

#[test]
fn a_datagram_for_a_foreign_address_is_dropped() {
    let mut outgoing = [0u8; 4096];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let foreign = Ipv4Addr::new(203, 0, 113, 9);
    assert!(drain(&mut stack, Some(&echo_frame(foreign)), start(), &mut rng).is_empty());
}

#[test]
fn an_echo_request_is_answered_from_the_address_it_was_sent_to() {
    let mut outgoing = [0u8; 4096];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    // The neighbor has to be known, or the reply waits for ARP.
    drain(&mut stack, Some(&arp_request(HERE)), start(), &mut rng);
    let frames = drain(&mut stack, Some(&echo_frame(HERE)), start(), &mut rng);
    assert_eq!(frames.len(), 1);
    let datagram = datagram_of(&frames[0]);
    assert_eq!(datagram.source(), HERE);
    assert_eq!(datagram.destination(), PEER);
    assert_eq!(datagram.protocol(), Protocol::ICMP);
    let message = net_ip::Message::parse(datagram.payload()).expect("a message");
    assert!(matches!(
        message,
        net_ip::Message::EchoReply {
            identifier: 7,
            sequence: 1,
            payload: b"ping"
        }
    ));
}

#[test]
fn a_datagram_reaches_the_socket_that_holds_its_port() {
    let mut outgoing = [0u8; 4096];
    let mut datagrams = [0u8; 512];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let socket = stack
        .bind(Some(IpAddr::V4(HERE)), Port::new(4711), &mut datagrams)
        .expect("a socket");
    let frame = udp_frame(HERE, 1024, 4711, b"hello");
    assert!(drain(&mut stack, Some(&frame), start(), &mut rng).is_empty());
    let received = stack.socket(socket).expect("the socket");
    let datagram = received.peek().expect("a datagram");
    assert_eq!(datagram.payload, b"hello");
    assert_eq!(datagram.source, IpAddr::V4(PEER));
    assert_eq!(datagram.port, Port::new(1024));
}

#[test]
fn a_datagram_for_a_port_nobody_holds_reaches_nobody() {
    let mut outgoing = [0u8; 4096];
    let mut datagrams = [0u8; 512];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let socket = stack
        .bind(Some(IpAddr::V4(HERE)), Port::new(4711), &mut datagrams)
        .expect("a socket");
    let frame = udp_frame(HERE, 1024, 9999, b"hello");
    assert!(drain(&mut stack, Some(&frame), start(), &mut rng).is_empty());
    assert!(stack.socket(socket).expect("the socket").is_empty());
}

#[test]
fn a_segment_to_no_connection_is_answered_with_a_reset() {
    let mut outgoing = [0u8; 4096];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    drain(&mut stack, Some(&arp_request(HERE)), start(), &mut rng);
    let frames = drain(
        &mut stack,
        Some(&syn_frame(HERE, 1024, 80)),
        start(),
        &mut rng,
    );
    assert_eq!(frames.len(), 1);
    let datagram = datagram_of(&frames[0]);
    assert_eq!(datagram.protocol(), Protocol::TCP);
    let segment = net_tcp::Segment::parse(
        datagram.payload(),
        IpAddr::V4(datagram.source()),
        IpAddr::V4(datagram.destination()),
    )
    .expect("a segment");
    assert!(segment.flags.has(net_tcp::Flags::RST));
    assert_eq!(segment.source_port, Port::new(80));
    assert_eq!(segment.destination_port, Port::new(1024));
}

#[test]
fn a_segment_for_a_listening_connection_reaches_it() {
    let mut outgoing = [0u8; 4096];
    let mut send = [0u8; 512];
    let mut receive = [0u8; 512];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let connection = stack
        .listen(
            IpAddr::V4(HERE),
            Port::new(80),
            &mut rng,
            &mut send,
            &mut receive,
        )
        .expect("a connection");
    assert_eq!(
        stack.connection(connection).expect("it").state(),
        net_tcp::State::Listen
    );
    drain(&mut stack, Some(&arp_request(HERE)), start(), &mut rng);
    let frames = drain(
        &mut stack,
        Some(&syn_frame(HERE, 1024, 80)),
        start(),
        &mut rng,
    );
    assert_eq!(
        stack.connection(connection).expect("it").state(),
        net_tcp::State::SynReceived
    );
    // And what it owes in answer went out.
    assert_eq!(frames.len(), 1);
    let datagram = datagram_of(&frames[0]);
    let segment = net_tcp::Segment::parse(
        datagram.payload(),
        IpAddr::V4(datagram.source()),
        IpAddr::V4(datagram.destination()),
    )
    .expect("a segment");
    assert!(segment.flags.has(net_tcp::Flags::SYN));
    assert!(segment.flags.has(net_tcp::Flags::ACK));
}

#[test]
fn a_handle_from_a_closed_socket_is_refused_and_the_slot_is_used_again() {
    let mut outgoing = [0u8; 4096];
    let mut datagrams = [0u8; 256];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let first = stack
        .bind(Some(IpAddr::V4(HERE)), Port::new(1000), &mut datagrams)
        .expect("a socket");
    let memory = stack.close(first).expect("the memory");
    assert_eq!(stack.socket(first).err(), Some(StackError::Stale));
    assert_eq!(stack.close(first).err(), Some(StackError::Stale));
    let second = stack
        .bind(Some(IpAddr::V4(HERE)), Port::new(1001), memory)
        .expect("a socket");
    assert_ne!(second, first);
    assert_eq!(stack.socket(second).expect("it").port(), Port::new(1001));
}

#[test]
fn the_socket_table_says_when_it_is_full_rather_than_reusing_a_live_slot() {
    let mut outgoing = [0u8; 4096];
    let mut memory = [[0u8; 128]; 5];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut open = Vec::new();
    let mut rings = memory.iter_mut();
    for port in 0..4u16 {
        let ring = rings.next().expect("a buffer");
        open.push(
            stack
                .bind(Some(IpAddr::V4(HERE)), Port::new(1000 + port), ring)
                .expect("a socket"),
        );
    }
    let last = rings.next().expect("a buffer");
    assert!(
        stack
            .bind(Some(IpAddr::V4(HERE)), Port::new(1004), last)
            .is_err()
    );
    // The four that are open are untouched.
    for (index, handle) in open.iter().enumerate() {
        let port = u16::try_from(index).expect("a small number");
        assert_eq!(
            stack.socket(*handle).expect("it").port(),
            Port::new(1000 + port)
        );
    }
}

#[test]
fn poll_at_is_the_earliest_deadline_and_moves_when_it_is_reached() {
    let mut outgoing = [0u8; 4096];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    // An idle stack has nothing to wait for.
    assert_eq!(stack.poll_at(start()), None);
    stack.configure(start());
    // The client wants to send at once.
    assert_eq!(stack.poll_at(start()), Some(start()));
    let frames = drain(&mut stack, None, start(), &mut rng);
    assert_eq!(frames.len(), 1);
    // And now it waits for the backoff.
    let next = stack.poll_at(start()).expect("a deadline");
    assert!(next > start(), "the deadline did not move");
    assert!(drain(&mut stack, None, start(), &mut rng).is_empty());
    let after = drain(&mut stack, None, next, &mut rng);
    assert_eq!(after.len(), 1);
    assert!(stack.poll_at(next).expect("a deadline") > next);
}

#[test]
fn poll_drains_a_transmit_buffer_of_one_frame_without_loss_or_reorder() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    drain(&mut stack, Some(&arp_request(HERE)), start(), &mut rng);
    // Three echo requests in a row, each answered.
    let mut answers = Vec::new();
    for sequence in 0..3u16 {
        let message = net_ip::Message::EchoRequest {
            identifier: sequence,
            sequence,
            payload: b"x",
        };
        let mut bytes = [0u8; 64];
        let mut writer = net_wire::Writer::new(&mut bytes);
        message.write(&mut writer).expect("room");
        let len = writer.position();
        let frame = ipv4_frame(MAC, PEER, HERE, Protocol::ICMP, &bytes[..len]);
        // Feed all three before taking any out.
        let mut tx = [0u8; FRAME_LEN];
        let taken = stack
            .poll(start(), Some(&frame), &mut tx, &mut rng)
            .expect("a poll");
        if let Some(frame) = taken {
            answers.push(frame.to_vec());
        }
    }
    // Whatever is left comes out in order, one frame per call.
    answers.extend(drain(&mut stack, None, start(), &mut rng));
    assert_eq!(answers.len(), 3, "a frame was lost");
    for (index, frame) in answers.iter().enumerate() {
        let datagram = datagram_of(frame);
        let message = net_ip::Message::parse(datagram.payload()).expect("a message");
        let net_ip::Message::EchoReply { sequence, .. } = message else {
            panic!("not a reply");
        };
        assert_eq!(
            usize::from(sequence),
            index,
            "the answers came out of order"
        );
    }
    assert_eq!(stack.dropped(), 0);
}

#[test]
fn the_four_message_exchange_configures_the_interface_and_then_everything_works() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = Stack::new(Config::new(MAC), &mut outgoing);
    let mut rng = rng();
    let mut now = start();
    stack.configure(now);

    // The discover goes out from nowhere to everyone.
    let frames = drain(&mut stack, None, now, &mut rng);
    assert_eq!(frames.len(), 1);
    assert_eq!(destination_of(&frames[0]), MacAddr::BROADCAST);
    let discover = dhcp_of(&frames[0]);
    assert_eq!(discover.message_type(), Ok(net_dhcp::MessageType::DISCOVER));
    assert!(discover.broadcast);
    let xid = discover.xid;
    let datagram = datagram_of(&frames[0]);
    assert_eq!(datagram.source(), Ipv4Addr::UNSPECIFIED);
    assert_eq!(datagram.destination(), Ipv4Addr::BROADCAST);

    // The offer comes back, and the request goes out.
    let frames = drain(
        &mut stack,
        Some(&dhcp_frame(net_dhcp::MessageType::OFFER, xid)),
        now,
        &mut rng,
    );
    assert_eq!(frames.len(), 1);
    assert_eq!(
        dhcp_of(&frames[0]).message_type(),
        Ok(net_dhcp::MessageType::REQUEST)
    );

    // And the acknowledgment configures the interface.
    now = now.saturating_add(Duration::from_millis(1));
    drain(
        &mut stack,
        Some(&dhcp_frame(net_dhcp::MessageType::ACK, xid)),
        now,
        &mut rng,
    );
    assert_eq!(stack.configuration(), net_dhcp::State::Bound);
    assert_eq!(
        stack.addresses().collect::<Vec<_>>(),
        vec![IpAddr::V4(HERE)]
    );
    let lease = stack.lease().expect("a lease");
    assert_eq!(lease.network.netmask(), MASK);
    assert_eq!(lease.router, Some(ROUTER));
    assert_eq!(
        stack.name_servers().collect::<Vec<_>>(),
        vec![IpAddr::V4(ROUTER)]
    );
    assert_eq!(stack.routes().len(), 2);

    // Now ARP is answered and an echo request comes back.
    let frames = drain(&mut stack, Some(&arp_request(HERE)), now, &mut rng);
    assert_eq!(frames.len(), 1);
    assert_eq!(ether_type_of(&frames[0]), EtherType::ARP);
    let frames = drain(&mut stack, Some(&echo_frame(HERE)), now, &mut rng);
    assert_eq!(frames.len(), 1);
    assert_eq!(datagram_of(&frames[0]).source(), HERE);
}

#[test]
fn a_send_to_an_unknown_neighbor_waits_for_the_answer_and_then_goes() {
    let mut outgoing = [0u8; 8192];
    let mut datagrams = [0u8; 256];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let socket = stack
        .bind(Some(IpAddr::V4(HERE)), Port::new(4711), &mut datagrams)
        .expect("a socket");
    stack
        .send_to(socket, IpAddr::V4(PEER), Port::new(53), b"query", start())
        .expect("a send");
    // Nothing of the datagram went out; what goes out is the question of
    // who holds the address.
    let frames = drain(&mut stack, None, start(), &mut rng);
    assert_eq!(frames.len(), 1);
    assert_eq!(ether_type_of(&frames[0]), EtherType::ARP);
    assert_eq!(arp_of(&frames[0]).operation, net_eth::Operation::Request);

    // The answer arrives, and the datagram follows it.
    let reply = net_eth::Packet {
        operation: net_eth::Operation::Reply,
        sender_hardware: PEER_MAC,
        sender_protocol: PEER,
        target_hardware: MAC,
        target_protocol: HERE,
    };
    let mut payload = [0u8; 64];
    let mut writer = net_wire::Writer::new(&mut payload);
    reply.write(&mut writer).expect("room");
    let len = writer.position();
    let frame = crate::tests::harness::frame(MAC, PEER_MAC, EtherType::ARP, &payload[..len]);
    let frames = drain(&mut stack, Some(&frame), start(), &mut rng);
    assert_eq!(frames.len(), 1);
    assert_eq!(destination_of(&frames[0]), PEER_MAC);
    let datagram = datagram_of(&frames[0]);
    assert_eq!(datagram.protocol(), Protocol::UDP);
    let udp = net_udp::Datagram::parse(
        datagram.payload(),
        IpAddr::V4(datagram.source()),
        IpAddr::V4(datagram.destination()),
    )
    .expect("a datagram");
    assert_eq!(udp.payload, b"query");
}

#[test]
fn addresses_and_routes_go_in_and_come_out_again() {
    let mut outgoing = [0u8; 1024];
    let mut stack: Stack<'_, 4, 2> = Stack::new(Config::new(MAC), &mut outgoing);
    assert!(!stack.is_mine(IpAddr::V4(HERE)));
    stack.add_address(IpAddr::V4(HERE)).expect("room");
    // Adding it twice is not an error and does not add it twice.
    stack.add_address(IpAddr::V4(HERE)).expect("room");
    assert_eq!(stack.addresses().count(), 1);
    assert!(stack.is_mine(IpAddr::V4(HERE)));
    for octet in 0..3u8 {
        stack
            .add_address(IpAddr::V4(Ipv4Addr::new(10, 0, 0, octet)))
            .expect("room");
    }
    assert_eq!(
        stack.add_address(IpAddr::V4(Ipv4Addr::new(10, 0, 1, 1))),
        Err(StackError::TooManyAddresses)
    );
    assert!(stack.remove_address(IpAddr::V4(HERE)));
    assert!(!stack.remove_address(IpAddr::V4(HERE)));

    let network = Ipv4Cidr::new(Ipv4Addr::new(10, 0, 0, 0), 24).expect("a network");
    stack
        .add_route(net_ip::Route::on_link(IpCidr::V4(network)))
        .expect("room");
    assert_eq!(stack.routes().len(), 1);
    assert!(stack.remove_route(IpCidr::V4(network)));
    assert_eq!(stack.routes().len(), 0);
    assert_eq!(stack.config().hardware, MAC);
    assert_eq!(stack.pending(), 0);
    assert_eq!(stack.peek(), None);
}

#[test]
fn a_stack_with_a_backlog_says_it_has_work_now() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    // The frame produces an answer and nothing takes it out.
    let mut tx = [0u8; FRAME_LEN];
    stack
        .poll(start(), Some(&arp_request(HERE)), &mut [0u8; 4], &mut rng)
        .expect("a poll");
    assert_eq!(stack.pending(), 1);
    assert!(stack.peek().is_some());
    assert_eq!(stack.poll_at(start()), Some(start()));
    // A transmit buffer that cannot hold the frame leaves it where it is.
    let taken = stack
        .poll(start(), None, &mut tx, &mut rng)
        .expect("a poll");
    assert!(taken.is_some());
    assert_eq!(stack.pending(), 0);
}

#[test]
fn a_refusal_takes_the_address_and_the_routes_away_again() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = Stack::new(Config::new(MAC), &mut outgoing);
    let mut rng = rng();
    let now = start();
    stack.configure(now);
    let frames = drain(&mut stack, None, now, &mut rng);
    let xid = dhcp_of(&frames[0]).xid;
    drain(
        &mut stack,
        Some(&dhcp_frame(net_dhcp::MessageType::OFFER, xid)),
        now,
        &mut rng,
    );
    drain(
        &mut stack,
        Some(&dhcp_frame(net_dhcp::MessageType::ACK, xid)),
        now,
        &mut rng,
    );
    assert_eq!(stack.addresses().count(), 1);
    assert_eq!(stack.routes().len(), 2);
    // The renewal is a unicast to the server, so the cache has to know
    // it; the server answering for itself is how it comes to.
    drain(
        &mut stack,
        Some(&arp_request_from(PEER_MAC, ROUTER, HERE)),
        now,
        &mut rng,
    );

    // A renewal at T1 that is refused takes it all away.
    let renew = stack.lease().expect("a lease").renew;
    let frames = drain(&mut stack, None, renew, &mut rng);
    let renewal = frames
        .iter()
        .find(|frame| carries(frame, Protocol::UDP))
        .expect("a renewal");
    let xid = dhcp_of(renewal).xid;
    drain(
        &mut stack,
        Some(&dhcp_frame(net_dhcp::MessageType::NAK, xid)),
        renew,
        &mut rng,
    );
    assert_eq!(stack.configuration(), net_dhcp::State::Selecting);
    assert_eq!(stack.addresses().count(), 0);
    assert_eq!(stack.routes().len(), 0);
    assert!(stack.lease().is_none());
    assert_eq!(stack.name_servers().count(), 0);
}

#[test]
fn a_lease_that_runs_out_takes_the_address_with_it() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = Stack::new(Config::new(MAC), &mut outgoing);
    let mut rng = rng();
    let now = start();
    stack.configure(now);
    let frames = drain(&mut stack, None, now, &mut rng);
    let xid = dhcp_of(&frames[0]).xid;
    drain(
        &mut stack,
        Some(&dhcp_frame(net_dhcp::MessageType::OFFER, xid)),
        now,
        &mut rng,
    );
    drain(
        &mut stack,
        Some(&dhcp_frame(net_dhcp::MessageType::ACK, xid)),
        now,
        &mut rng,
    );
    let expires = stack.lease().expect("a lease").expires;
    drain(&mut stack, None, expires, &mut rng);
    assert_eq!(stack.addresses().count(), 0);
    assert_eq!(stack.configuration(), net_dhcp::State::Selecting);
}

#[test]
fn a_datagram_this_host_sends_to_itself_leaves_from_the_address_it_names() {
    let mut outgoing = [0u8; 8192];
    let mut datagrams = [0u8; 512];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let socket = stack
        .bind(None, Port::new(4711), &mut datagrams)
        .expect("a socket");
    // A wildcard socket has no address of its own, so the one the route
    // decides is the one it leaves from.
    drain(&mut stack, Some(&arp_request(HERE)), start(), &mut rng);
    stack
        .send_to(socket, IpAddr::V4(PEER), Port::new(53), b"x", start())
        .expect("a send");
    let frames = drain(&mut stack, None, start(), &mut rng);
    let sent = frames
        .iter()
        .find(|frame| carries(frame, Protocol::UDP))
        .expect("a datagram");
    assert_eq!(datagram_of(sent).source(), HERE);
}

#[test]
fn a_handle_of_the_wrong_kind_names_nothing() {
    let mut outgoing = [0u8; 4096];
    let mut datagrams = [0u8; 256];
    let mut send = [0u8; 256];
    let mut receive = [0u8; 256];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let socket = stack
        .bind(Some(IpAddr::V4(HERE)), Port::new(1000), &mut datagrams)
        .expect("a socket");
    let connection = stack
        .listen(
            IpAddr::V4(HERE),
            Port::new(80),
            &mut rng,
            &mut send,
            &mut receive,
        )
        .expect("a connection");
    // The two tables count their generations apart, so the first handle
    // of each names the first slot of each and nothing more.
    assert!(stack.connection(connection).is_ok());
    assert!(stack.socket(socket).is_ok());
    let buffers = stack.close_connection(connection).expect("the memory");
    assert_eq!(buffers.0.len(), 256);
    assert_eq!(stack.connection(connection).err(), Some(StackError::Stale));
    assert_eq!(
        stack.close_connection(connection).err(),
        Some(StackError::Stale)
    );
    // And the socket is untouched by any of it.
    assert!(stack.socket(socket).is_ok());
}

#[test]
fn bytes_that_are_not_the_thing_they_claim_to_be_are_dropped() {
    let mut outgoing = [0u8; 8192];
    let mut datagrams = [0u8; 256];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let socket = stack
        .bind(Some(IpAddr::V4(HERE)), Port::new(4711), &mut datagrams)
        .expect("a socket");

    // An ARP frame whose payload is not a packet.
    let broken = crate::tests::harness::frame(MAC, PEER_MAC, EtherType::ARP, &[0u8; 8]);
    assert!(drain(&mut stack, Some(&broken), start(), &mut rng).is_empty());

    // A frame that claims IPv4 and carries nothing of the kind.
    let broken = crate::tests::harness::frame(MAC, PEER_MAC, EtherType::IPV4, &[0xFFu8; 24]);
    assert!(drain(&mut stack, Some(&broken), start(), &mut rng).is_empty());

    // A UDP datagram whose checksum does not verify.
    let mut frame = udp_frame(HERE, 1024, 4711, b"hello");
    let last = frame.len() - 1;
    frame[last] ^= 0xFF;
    assert!(drain(&mut stack, Some(&frame), start(), &mut rng).is_empty());
    assert!(stack.socket(socket).expect("it").is_empty());

    // A datagram addressed to port 68 whose checksum does not verify
    // reaches neither the client nor a socket.
    let mut reply = dhcp_frame(net_dhcp::MessageType::OFFER, 1);
    let last = reply.len() - 1;
    reply[last] ^= 0xFF;
    assert!(drain(&mut stack, Some(&reply), start(), &mut rng).is_empty());
}

#[test]
fn a_fragment_that_completes_nothing_is_held_and_answered_when_it_does() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    drain(&mut stack, Some(&arp_request(HERE)), start(), &mut rng);
    // An echo request cut in two: the first half completes nothing.
    let message = {
        let mut bytes = [0u8; 32];
        let mut writer = net_wire::Writer::new(&mut bytes);
        net_ip::Message::EchoRequest {
            identifier: 3,
            sequence: 4,
            payload: b"12345678",
        }
        .write(&mut writer)
        .expect("room");
        writer.finish().to_vec()
    };
    let (first, second) = message.split_at(8);
    // The offset is in bytes here; the header writes it in units of
    // eight.
    for (offset, part, more) in [(0usize, first, true), (8usize, second, false)] {
        let mut header = net_ip::Header::new(PEER, HERE, Protocol::ICMP, part.len());
        header.identification = 42;
        header.more_fragments = more;
        header.fragment_offset = offset;
        let mut bytes = vec![0u8; 128];
        let mut writer = net_wire::Writer::new(&mut bytes);
        header.write(&mut writer).expect("room");
        writer.write_bytes(part).expect("room");
        let len = writer.position();
        let frame = crate::tests::harness::frame(MAC, PEER_MAC, EtherType::IPV4, &bytes[..len]);
        let answers = drain(&mut stack, Some(&frame), start(), &mut rng);
        if more {
            assert!(answers.is_empty(), "half a datagram was answered");
        } else {
            assert_eq!(answers.len(), 1, "the whole of it was not");
        }
    }
}

#[test]
fn an_echo_request_to_the_broadcast_address_is_answered_from_a_chosen_address() {
    let mut outgoing = [0u8; 8192];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    drain(&mut stack, Some(&arp_request(HERE)), start(), &mut rng);
    let message = {
        let mut bytes = [0u8; 32];
        let mut writer = net_wire::Writer::new(&mut bytes);
        net_ip::Message::EchoRequest {
            identifier: 1,
            sequence: 1,
            payload: b"all",
        }
        .write(&mut writer)
        .expect("room");
        writer.finish().to_vec()
    };
    let frame = ipv4_frame(
        MacAddr::BROADCAST,
        PEER,
        Ipv4Addr::BROADCAST,
        Protocol::ICMP,
        &message,
    );
    let frames = drain(&mut stack, Some(&frame), start(), &mut rng);
    assert_eq!(frames.len(), 1);
    assert_eq!(datagram_of(&frames[0]).source(), HERE);
    // And one to a multicast group is taken the same way.
    let frame = ipv4_frame(
        MacAddr::BROADCAST,
        PEER,
        Ipv4Addr::new(224, 0, 0, 1),
        Protocol::ICMP,
        &message,
    );
    assert_eq!(drain(&mut stack, Some(&frame), start(), &mut rng).len(), 1);
}

#[test]
fn a_neighbor_that_never_answers_is_given_up_on() {
    let mut outgoing = [0u8; 8192];
    let mut datagrams = [0u8; 256];
    let mut stack: Stack<'_, 4, 2> = configured(&mut outgoing);
    let mut rng = rng();
    let socket = stack
        .bind(Some(IpAddr::V4(HERE)), Port::new(4711), &mut datagrams)
        .expect("a socket");
    stack
        .send_to(socket, IpAddr::V4(PEER), Port::new(53), b"x", start())
        .expect("a send");
    let mut now = start();
    let mut asked = 0usize;
    for _ in 0..16 {
        asked += drain(&mut stack, None, now, &mut rng)
            .iter()
            .filter(|frame| ether_type_of(frame) == EtherType::ARP)
            .count();
        let Some(next) = stack.poll_at(now) else {
            break;
        };
        now = next.max(now.saturating_add(Duration::from_millis(1)));
    }
    assert!(asked >= 2, "the cache asked only {asked} times");
    assert_eq!(stack.neighbors().hardware(IpAddr::V4(PEER)), None);
    assert_eq!(stack.poll_at(now), None);
}

#[test]
fn an_interface_with_no_address_answers_no_broadcast_either() {
    let mut outgoing = [0u8; 4096];
    let mut stack: Stack<'_, 4, 2> = Stack::new(Config::new(MAC), &mut outgoing);
    let mut rng = rng();
    // The datagram is taken — it is addressed to the whole link — and
    // there is nothing for an answer to leave from.
    let message = {
        let mut bytes = [0u8; 32];
        let mut writer = net_wire::Writer::new(&mut bytes);
        net_ip::Message::EchoRequest {
            identifier: 1,
            sequence: 1,
            payload: b"all",
        }
        .write(&mut writer)
        .expect("room");
        writer.finish().to_vec()
    };
    let frame = ipv4_frame(
        MacAddr::BROADCAST,
        PEER,
        Ipv4Addr::BROADCAST,
        Protocol::ICMP,
        &message,
    );
    assert!(drain(&mut stack, Some(&frame), start(), &mut rng).is_empty());
}
