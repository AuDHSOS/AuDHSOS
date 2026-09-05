// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The way out: route, resolve, fragment, emit.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test reads a frame at known offsets"
)]

use audhsos_time::Instant;
use net_eth::{EthError, Frame, NeighborCache, receive};
use net_wire::{EtherType, IpAddr, IpCidr, Ipv4Addr, Ipv4Cidr, MacAddr, Protocol};

use crate::error::IpError;
use crate::header::{Datagram, MIN_HEADER_LEN};
use crate::route::{Route, RoutingTable};
use crate::send::{Interface, Outgoing, Sender, Sent};

/// This host.
const HARDWARE: MacAddr = MacAddr::new([0x02, 0x00, 0x5E, 0x00, 0x00, 0x01]);

/// The neighbor it talks to.
const PEER_HARDWARE: MacAddr = MacAddr::new([0x02, 0x00, 0x5E, 0x00, 0x00, 0x02]);

/// The MTU everything here is cut to.
const MTU: usize = 1500;

/// This host's address.
fn source() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 168, 1, 7))
}

/// A host on the same link.
fn on_link() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 168, 1, 9))
}

/// A host somewhere else.
fn far_away() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))
}

/// The router that reaches it.
fn gateway() -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 168, 1, 254))
}

/// The interface under test.
fn interface() -> Interface {
    Interface {
        hardware: HARDWARE,
        mtu: MTU,
    }
}

/// A table with the local subnet and a default route.
fn routes() -> RoutingTable<4> {
    let mut table = RoutingTable::new();
    table
        .add(Route::on_link(IpCidr::V4(
            Ipv4Cidr::parse("192.168.1.0/24").expect("a network"),
        )))
        .expect("room");
    table
        .add(Route::via(
            IpCidr::V4(Ipv4Cidr::parse("0.0.0.0/0").expect("a network")),
            gateway(),
        ))
        .expect("room");
    table
}

/// What is being sent in most of these tests.
fn outgoing(destination: IpAddr) -> Outgoing {
    Outgoing {
        source: source(),
        destination,
        protocol: Protocol::UDP,
        identification: 0x1C46,
        dont_fragment: false,
    }
}

/// The frames a send produced, each read back as a frame and a datagram.
fn collect(frames: &[Vec<u8>]) -> Vec<(MacAddr, Ipv4Addr, Vec<u8>)> {
    frames
        .iter()
        .map(|bytes| {
            let frame = receive(bytes, PEER_HARDWARE).expect("a frame for the peer");
            assert_eq!(frame.ether_type(), EtherType::IPV4);
            assert_eq!(frame.source(), HARDWARE);
            let datagram = Datagram::parse(frame.payload()).expect("a datagram");
            (
                frame.destination(),
                datagram.destination(),
                datagram.payload().to_vec(),
            )
        })
        .collect()
}

#[test]
fn a_datagram_to_a_known_neighbor_on_this_link_goes_out_at_once() {
    let mut neighbors = NeighborCache::<4, 2048>::new();
    neighbors.on_confirmed(on_link(), PEER_HARDWARE, Instant::ZERO);
    let table = routes();
    let mut sender = Sender {
        interface: interface(),
        routes: &table,
        neighbors: &mut neighbors,
    };
    let mut buffer = [0u8; MTU];
    let mut frames = Vec::new();
    let sent = sender
        .send(
            outgoing(on_link()),
            b"hello",
            Instant::ZERO,
            &mut buffer,
            |frame| {
                frames.push(frame.to_vec());
                Ok(())
            },
        )
        .expect("it goes");
    assert_eq!(sent, Sent::Frames(1));

    let read = collect(&frames);
    assert_eq!(read.len(), 1);
    // The frame goes to the neighbor, and the datagram to the host.
    assert_eq!(read[0].0, PEER_HARDWARE);
    assert_eq!(read[0].1, Ipv4Addr::new(192, 168, 1, 9));
    assert_eq!(read[0].2, b"hello");
}

#[test]
fn a_datagram_to_a_far_host_goes_to_the_router_and_keeps_its_own_address() {
    let mut neighbors = NeighborCache::<4, 2048>::new();
    neighbors.on_confirmed(gateway(), PEER_HARDWARE, Instant::ZERO);
    let table = routes();
    let mut sender = Sender {
        interface: interface(),
        routes: &table,
        neighbors: &mut neighbors,
    };
    let mut buffer = [0u8; MTU];
    let mut frames = Vec::new();
    let sent = sender
        .send(
            outgoing(far_away()),
            b"hello",
            Instant::ZERO,
            &mut buffer,
            |frame| {
                frames.push(frame.to_vec());
                Ok(())
            },
        )
        .expect("it goes");
    assert_eq!(sent, Sent::Frames(1));

    let read = collect(&frames);
    // The frame is addressed to the router; the datagram is not.
    assert_eq!(read[0].0, PEER_HARDWARE);
    assert_eq!(read[0].1, Ipv4Addr::new(9, 9, 9, 9));
}

#[test]
fn a_datagram_to_an_unknown_neighbor_waits_and_goes_when_the_answer_comes() {
    let mut neighbors = NeighborCache::<4, 2048>::new();
    let table = routes();
    let mut buffer = [0u8; MTU];
    let mut frames = Vec::new();
    {
        let mut sender = Sender {
            interface: interface(),
            routes: &table,
            neighbors: &mut neighbors,
        };
        let sent = sender
            .send(
                outgoing(on_link()),
                b"hello",
                Instant::ZERO,
                &mut buffer,
                |frame| {
                    frames.push(frame.to_vec());
                    Ok(())
                },
            )
            .expect("it waits");
        assert_eq!(sent, Sent::Resolving);
    }
    assert!(frames.is_empty());
    // The cache asks for the address; that is the caller's ARP request.
    assert!(neighbors.poll(Instant::ZERO).is_some());

    neighbors.on_confirmed(on_link(), PEER_HARDWARE, Instant::ZERO);
    let mut sender = Sender {
        interface: interface(),
        routes: &table,
        neighbors: &mut neighbors,
    };
    let count = sender
        .send_pending(on_link(), outgoing(on_link()), &mut buffer, |frame| {
            frames.push(frame.to_vec());
            Ok(())
        })
        .expect("what was waiting goes");
    assert_eq!(count, 1);

    let read = collect(&frames);
    assert_eq!(read[0].2, b"hello");
    // And it goes once.
    let count = sender
        .send_pending(on_link(), outgoing(on_link()), &mut buffer, |_| Ok(()))
        .expect("nothing is left");
    assert_eq!(count, 0);
}

#[test]
fn a_datagram_longer_than_the_mtu_leaves_in_several_frames() {
    let mut neighbors = NeighborCache::<4, 2048>::new();
    neighbors.on_confirmed(on_link(), PEER_HARDWARE, Instant::ZERO);
    let table = routes();
    let mut sender = Sender {
        interface: interface(),
        routes: &table,
        neighbors: &mut neighbors,
    };
    let payload = vec![0xA5u8; MTU];
    let mut buffer = [0u8; MTU];
    let mut frames = Vec::new();
    let sent = sender
        .send(
            outgoing(on_link()),
            &payload,
            Instant::ZERO,
            &mut buffer,
            |frame| {
                frames.push(frame.to_vec());
                Ok(())
            },
        )
        .expect("it is cut up");
    assert_eq!(sent, Sent::Frames(2));

    let read = collect(&frames);
    let rejoined: Vec<u8> = read
        .iter()
        .flat_map(|(_, _, bytes)| bytes.clone())
        .collect();
    assert_eq!(rejoined, payload);
    // Every frame is one an Ethernet carries.
    for frame in &frames {
        assert!(frame.len() <= net_eth::MAX_FRAME_LEN, "{}", frame.len());
    }
}

#[test]
fn a_datagram_that_may_not_be_cut_up_is_an_error_and_not_a_truncation() {
    let mut neighbors = NeighborCache::<4, 2048>::new();
    neighbors.on_confirmed(on_link(), PEER_HARDWARE, Instant::ZERO);
    let table = routes();
    let mut sender = Sender {
        interface: interface(),
        routes: &table,
        neighbors: &mut neighbors,
    };
    let payload = vec![0xA5u8; MTU];
    let mut buffer = [0u8; MTU];
    let mut out = outgoing(on_link());
    out.dont_fragment = true;
    let mut frames = 0usize;
    assert_eq!(
        sender.send(out, &payload, Instant::ZERO, &mut buffer, |_| {
            frames += 1;
            Ok(())
        }),
        Err(IpError::WouldFragment {
            length: MTU,
            mtu: MTU
        })
    );
    assert_eq!(frames, 0);
}

#[test]
fn a_destination_with_no_route_is_an_error() {
    let mut neighbors = NeighborCache::<4, 2048>::new();
    let table = RoutingTable::<4>::new();
    let mut sender = Sender {
        interface: interface(),
        routes: &table,
        neighbors: &mut neighbors,
    };
    let mut buffer = [0u8; MTU];
    assert_eq!(
        sender.send(
            outgoing(far_away()),
            b"hello",
            Instant::ZERO,
            &mut buffer,
            |_| Ok(())
        ),
        Err(IpError::NoRoute)
    );
}

#[test]
fn an_ipv6_pair_leaves_through_the_other_crate_and_is_refused_here() {
    let mut neighbors = NeighborCache::<4, 2048>::new();
    let table = routes();
    let mut sender = Sender {
        interface: interface(),
        routes: &table,
        neighbors: &mut neighbors,
    };
    let six = IpAddr::V6(net_wire::Ipv6Addr::LOCALHOST);
    let mut out = outgoing(six);
    out.source = six;
    let mut buffer = [0u8; MTU];
    assert_eq!(
        sender.send(out, b"hello", Instant::ZERO, &mut buffer, |_| Ok(())),
        Err(IpError::NoRoute)
    );
    assert_eq!(
        sender.send_pending(six, out, &mut buffer, |_| Ok(())),
        Ok(0)
    );
}

#[test]
fn a_datagram_the_cache_will_not_hold_is_dropped_and_said_so() {
    // The cache holds sixteen bytes a neighbor; the datagram is longer.
    let mut neighbors = NeighborCache::<2, 16>::new();
    let table = routes();
    let mut sender = Sender {
        interface: interface(),
        routes: &table,
        neighbors: &mut neighbors,
    };
    let mut buffer = [0u8; MTU];
    let sent = sender
        .send(
            outgoing(on_link()),
            &[0u8; 32],
            Instant::ZERO,
            &mut buffer,
            |_| Ok(()),
        )
        .expect("it answers");
    assert_eq!(sent, Sent::Dropped);
}

#[test]
fn what_the_emitter_refuses_comes_back_to_the_caller() {
    let mut neighbors = NeighborCache::<4, 2048>::new();
    neighbors.on_confirmed(on_link(), PEER_HARDWARE, Instant::ZERO);
    let table = routes();
    let mut sender = Sender {
        interface: interface(),
        routes: &table,
        neighbors: &mut neighbors,
    };
    let mut buffer = [0u8; MTU];
    assert_eq!(
        sender.send(
            outgoing(on_link()),
            b"hello",
            Instant::ZERO,
            &mut buffer,
            |_| Err(IpError::TooLarge(0))
        ),
        Err(IpError::TooLarge(0))
    );
}

#[test]
fn a_buffer_smaller_than_the_mtu_is_refused_before_anything_is_written() {
    let mut neighbors = NeighborCache::<4, 2048>::new();
    neighbors.on_confirmed(on_link(), PEER_HARDWARE, Instant::ZERO);
    let table = routes();
    let mut sender = Sender {
        interface: interface(),
        routes: &table,
        neighbors: &mut neighbors,
    };
    let mut buffer = [0u8; MIN_HEADER_LEN];
    assert_eq!(
        sender.send(
            outgoing(on_link()),
            b"hello",
            Instant::ZERO,
            &mut buffer,
            |_| Ok(())
        ),
        Err(IpError::TooLarge(MTU))
    );
}

#[test]
fn a_frame_this_host_writes_is_one_a_reader_accepts() {
    // The frame goes out to the peer, so the peer receives it and this
    // host does not.
    let mut neighbors = NeighborCache::<4, 2048>::new();
    neighbors.on_confirmed(on_link(), PEER_HARDWARE, Instant::ZERO);
    let table = routes();
    let mut sender = Sender {
        interface: interface(),
        routes: &table,
        neighbors: &mut neighbors,
    };
    let mut buffer = [0u8; MTU];
    let mut frames = Vec::new();
    sender
        .send(
            outgoing(on_link()),
            b"hello",
            Instant::ZERO,
            &mut buffer,
            |frame| {
                frames.push(frame.to_vec());
                Ok(())
            },
        )
        .expect("it goes");
    assert!(receive(&frames[0], PEER_HARDWARE).is_some());
    assert!(receive(&frames[0], HARDWARE).is_none());
    assert!(matches!(
        Frame::parse(&frames[0]),
        Ok(frame) if frame.ether_type() == EtherType::IPV4
    ));
    // And nothing here produces an `EthError` the caller has to read.
    let _ = EthError::NotEthernetIpv4;
}
