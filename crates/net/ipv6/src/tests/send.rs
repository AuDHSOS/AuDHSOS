// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The way out: the route, the neighbor, the path MTU, and the frames.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test reads a frame at known offsets"
)]

use net_eth::{Frame, NeighborCache, multicast_hardware};
use net_ip::{Interface, IpError, Reassembler, Route, RoutingTable, Sent};
use net_wire::{EtherType, IpAddr, IpCidr, Ipv6Addr, Ipv6Cidr, Protocol};

use super::{HARDWARE, HOST, PEER, PEER_HARDWARE, at};
use crate::error::Ipv6Error;
use crate::fragment::reassemble;
use crate::header::{FRAGMENT_HEADER_LEN, HEADER_LEN, MIN_MTU, Packet};
use crate::pmtu::PathMtu;
use crate::send::{Outgoing, Sender};

/// The tables a sender borrows here.
type Routes = RoutingTable<4>;
/// The cache a sender borrows here.
type Cache = NeighborCache<4, 2048>;
/// The estimates a sender borrows here.
type Paths = PathMtu<2>;

/// What the interface is in these tests.
fn interface() -> Interface {
    Interface {
        hardware: HARDWARE,
        mtu: net_eth::MTU,
    }
}

/// A table that puts the peer's prefix on this link.
fn routes() -> Routes {
    let mut routes = Routes::new();
    let prefix = Ipv6Cidr::new(
        Ipv6Addr::from_octets([0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        64,
    )
    .expect("a /64");
    routes
        .add(Route::on_link(IpCidr::V6(prefix)))
        .expect("room for one route");
    routes
}

/// What is being sent in most of these tests.
fn outgoing() -> Outgoing {
    Outgoing {
        source: HOST,
        destination: PEER,
        protocol: Protocol::UDP,
        identification: 0x0102_0304,
    }
}

/// Sends `payload` and collects the frames it became.
fn send(
    routes: &Routes,
    neighbors: &mut Cache,
    path: &Paths,
    outgoing: Outgoing,
    payload: &[u8],
) -> Result<(Sent, Vec<Vec<u8>>), Ipv6Error> {
    let mut frames = Vec::new();
    let mut buffer = [0u8; net_eth::MTU];
    let mut sender = Sender {
        interface: interface(),
        routes,
        neighbors,
        path,
    };
    let sent = sender.send(outgoing, payload, at(0), &mut buffer, |frame| {
        frames.push(frame.to_vec());
        Ok(())
    })?;
    Ok((sent, frames))
}

/// The IPv6 packet inside a frame.
fn packet_of(frame: &[u8]) -> Vec<u8> {
    let frame = Frame::parse(frame).expect("a well-formed frame");
    assert_eq!(frame.ether_type(), EtherType::IPV6);
    frame.payload().to_vec()
}

#[test]
fn a_packet_that_fits_goes_out_in_one_frame_with_no_fragment_header() {
    let routes = routes();
    let mut neighbors = Cache::new();
    neighbors.on_confirmed(IpAddr::V6(PEER), PEER_HARDWARE, at(0));
    let paths = Paths::new();

    let payload = [0xABu8; 100];
    let (sent, frames) =
        send(&routes, &mut neighbors, &paths, outgoing(), &payload).expect("it fits");
    assert_eq!(sent, Sent::Frames(1));
    assert_eq!(frames.len(), 1);

    let frame = Frame::parse(&frames[0]).expect("a well-formed frame");
    assert_eq!(frame.destination(), PEER_HARDWARE);
    assert_eq!(frame.source(), HARDWARE);
    assert_eq!(frame.ether_type(), EtherType::IPV6);

    let packet = Packet::parse(frame.payload()).expect("a well-formed packet");
    assert_eq!(packet.source(), HOST);
    assert_eq!(packet.destination(), PEER);
    // No fragment header at all, which is where this differs from IPv4:
    // the fields are in an extension header and a whole datagram carries
    // none of them.
    assert_eq!(packet.next_header(), Protocol::UDP);
    let upper = packet.upper_layer().expect("no chain");
    assert_eq!(upper.fragment, None);
    assert_eq!(upper.payload, &payload);
}

#[test]
fn a_packet_that_does_not_fit_is_cut_and_reassembles_to_itself() {
    let routes = routes();
    let mut neighbors = Cache::new();
    neighbors.on_confirmed(IpAddr::V6(PEER), PEER_HARDWARE, at(0));
    let paths = Paths::new();

    let payload: Vec<u8> = (0..3000usize)
        .map(|index| u8::try_from(index % 251).unwrap_or(0))
        .collect();
    let (sent, frames) =
        send(&routes, &mut neighbors, &paths, outgoing(), &payload).expect("it is cut up");
    assert_eq!(sent, Sent::Frames(3));
    assert_eq!(frames.len(), 3);

    let mut buffers = Reassembler::<1, 4096>::new();
    let mut assembled = None;
    let packets: Vec<Vec<u8>> = frames.iter().map(|frame| packet_of(frame)).collect();
    for (index, bytes) in packets.iter().enumerate() {
        let packet = Packet::parse(bytes).expect("a well-formed packet");
        assert_eq!(packet.next_header(), Protocol::FRAGMENT);
        let upper = packet.upper_layer().expect("a fragment header");
        let fragment = upper.fragment.expect("a fragment header");
        assert_eq!(fragment.identification, 0x0102_0304);
        assert_eq!(fragment.more, index + 1 < packets.len());
        assert_eq!(upper.protocol, Protocol::UDP);
        // Every piece but the last is a multiple of eight, which is what
        // the offset field counts in.
        if fragment.more {
            assert_eq!(upper.payload.len() % 8, 0);
        }
        assert!(packet.total_len() <= net_eth::MTU);
        if let Some(whole) = reassemble(&mut buffers, packet, upper, at(1)).expect("no overlap") {
            assembled = Some(whole.to_vec());
        }
    }
    assert_eq!(assembled.as_deref(), Some(payload.as_slice()));
}

#[test]
fn the_path_mtu_and_not_the_link_decides_how_the_packet_is_cut() {
    let routes = routes();
    let mut neighbors = Cache::new();
    neighbors.on_confirmed(IpAddr::V6(PEER), PEER_HARDWARE, at(0));
    let mut paths = Paths::new();
    assert!(paths.on_packet_too_big(PEER, 1280, at(0)));

    let payload = [0x5Au8; 1400];
    let (sent, frames) =
        send(&routes, &mut neighbors, &paths, outgoing(), &payload).expect("it is cut up");
    // Without the estimate this would have gone out in one frame.
    assert_eq!(sent, Sent::Frames(2));
    for frame in &frames {
        let packet = packet_of(frame);
        assert!(
            packet.len() <= MIN_MTU,
            "a packet larger than the path is known to carry"
        );
    }
}

#[test]
fn an_unresolved_neighbor_holds_the_packet_and_it_goes_when_the_answer_comes() {
    let routes = routes();
    let mut neighbors = Cache::new();
    let paths = Paths::new();
    let payload = [1u8; 64];

    let (sent, frames) =
        send(&routes, &mut neighbors, &paths, outgoing(), &payload).expect("it waits");
    assert_eq!(sent, Sent::Resolving);
    assert!(frames.is_empty());

    // The advertisement arrives.
    neighbors.on_confirmed(IpAddr::V6(PEER), PEER_HARDWARE, at(1));
    let mut buffer = [0u8; net_eth::MTU];
    let mut frames = Vec::new();
    let mut sender = Sender {
        interface: interface(),
        routes: &routes,
        neighbors: &mut neighbors,
        path: &paths,
    };
    let count = sender
        .send_pending(PEER, outgoing(), &mut buffer, |frame| {
            frames.push(frame.to_vec());
            Ok(())
        })
        .expect("the held packet goes");
    assert_eq!(count, 1);
    let packet = packet_of(&frames[0]);
    let parsed = Packet::parse(&packet).expect("a well-formed packet");
    assert_eq!(parsed.payload(), &payload);

    // And it goes once: the cache does not hold it any more.
    let mut sender = Sender {
        interface: interface(),
        routes: &routes,
        neighbors: &mut neighbors,
        path: &paths,
    };
    assert_eq!(
        sender.send_pending(PEER, outgoing(), &mut buffer, |_| Ok(())),
        Ok(0)
    );
}

#[test]
fn a_packet_longer_than_the_cache_holds_is_dropped_rather_than_half_kept() {
    let routes = routes();
    let mut neighbors = NeighborCache::<4, 16>::new();
    let paths = Paths::new();
    let payload = [1u8; 64];
    let mut buffer = [0u8; net_eth::MTU];
    let mut sender = Sender {
        interface: interface(),
        routes: &routes,
        neighbors: &mut neighbors,
        path: &paths,
    };
    assert_eq!(
        sender.send(outgoing(), &payload, at(0), &mut buffer, |_| Ok(())),
        Ok(Sent::Dropped)
    );
}

#[test]
fn a_multicast_destination_needs_no_route_and_no_neighbor() {
    // Which is what lets a solicitation go to a station this host knows
    // nothing about — including its own group during duplicate address
    // detection, when it has no address to be answered at.
    let routes = Routes::new();
    let mut neighbors = Cache::new();
    let paths = Paths::new();
    let group = PEER.solicited_node();
    let outgoing = Outgoing {
        source: Ipv6Addr::UNSPECIFIED,
        destination: group,
        protocol: Protocol::ICMPV6,
        identification: 0,
    };

    let (sent, frames) =
        send(&routes, &mut neighbors, &paths, outgoing, &[0; 24]).expect("it goes at once");
    assert_eq!(sent, Sent::Frames(1));
    assert!(neighbors.is_empty(), "nothing was resolved");
    let frame = Frame::parse(&frames[0]).expect("a well-formed frame");
    assert_eq!(frame.destination(), multicast_hardware(group));
    assert!(frame.destination().is_multicast());
}

#[test]
fn a_destination_no_route_reaches_is_refused() {
    let routes = Routes::new();
    let mut neighbors = Cache::new();
    let paths = Paths::new();
    assert_eq!(
        send(&routes, &mut neighbors, &paths, outgoing(), &[0; 8]),
        Err(Ipv6Error::Ip(IpError::NoRoute))
    );
}

#[test]
fn a_packet_goes_through_the_router_when_the_destination_is_not_on_this_link() {
    let router = Ipv6Addr::from_octets([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01]);
    let mut table = Routes::new();
    table
        .add(Route::via(
            IpCidr::V6(Ipv6Cidr::new(Ipv6Addr::UNSPECIFIED, 0).expect("a default route")),
            IpAddr::V6(router),
        ))
        .expect("room");
    let mut neighbors = Cache::new();
    neighbors.on_confirmed(IpAddr::V6(router), PEER_HARDWARE, at(0));
    let paths = Paths::new();

    let (sent, frames) =
        send(&table, &mut neighbors, &paths, outgoing(), &[9; 32]).expect("it goes");
    assert_eq!(sent, Sent::Frames(1));
    let frame = Frame::parse(&frames[0]).expect("a well-formed frame");
    // The frame goes to the router; the packet still goes to the peer.
    assert_eq!(frame.destination(), PEER_HARDWARE);
    let packet = Packet::parse(frame.payload()).expect("a well-formed packet");
    assert_eq!(packet.destination(), PEER);
}

#[test]
fn a_buffer_too_small_for_the_path_is_refused() {
    let routes = routes();
    let mut neighbors = Cache::new();
    neighbors.on_confirmed(IpAddr::V6(PEER), PEER_HARDWARE, at(0));
    let paths = Paths::new();
    let mut buffer = [0u8; 64];
    let mut sender = Sender {
        interface: interface(),
        routes: &routes,
        neighbors: &mut neighbors,
        path: &paths,
    };
    assert_eq!(
        sender.send(outgoing(), &[0; 8], at(0), &mut buffer, |_| Ok(())),
        Err(Ipv6Error::WouldFragment {
            length: 8,
            mtu: net_eth::MTU,
        })
    );
}

#[test]
fn an_interface_with_no_room_behind_the_two_headers_cannot_cut_a_packet() {
    let routes = routes();
    let mut neighbors = Cache::new();
    neighbors.on_confirmed(IpAddr::V6(PEER), PEER_HARDWARE, at(0));
    let paths = Paths::new();
    let mut buffer = [0u8; net_eth::MTU];
    let mut sender = Sender {
        interface: Interface {
            hardware: HARDWARE,
            mtu: HEADER_LEN + FRAGMENT_HEADER_LEN,
        },
        routes: &routes,
        neighbors: &mut neighbors,
        path: &paths,
    };
    let payload = [0u8; 128];
    assert_eq!(
        sender.send(outgoing(), &payload, at(0), &mut buffer, |_| Ok(())),
        Err(Ipv6Error::Ip(IpError::WouldFragment {
            length: payload.len(),
            mtu: HEADER_LEN + FRAGMENT_HEADER_LEN,
        })),
        "a link with no room behind the two headers carries nothing"
    );
}

#[test]
fn what_emit_answers_is_what_send_answers() {
    let routes = routes();
    let mut neighbors = Cache::new();
    neighbors.on_confirmed(IpAddr::V6(PEER), PEER_HARDWARE, at(0));
    let paths = Paths::new();
    let mut buffer = [0u8; net_eth::MTU];
    let mut sender = Sender {
        interface: interface(),
        routes: &routes,
        neighbors: &mut neighbors,
        path: &paths,
    };
    assert_eq!(
        sender.send(outgoing(), &[0; 8], at(0), &mut buffer, |_| Err(
            Ipv6Error::BadIcmp
        )),
        Err(Ipv6Error::BadIcmp)
    );
}

#[test]
fn nothing_waiting_sends_nothing() {
    let routes = routes();
    let mut neighbors = Cache::new();
    neighbors.on_confirmed(IpAddr::V6(PEER), PEER_HARDWARE, at(0));
    let paths = Paths::new();
    let mut buffer = [0u8; net_eth::MTU];
    let mut sender = Sender {
        interface: interface(),
        routes: &routes,
        neighbors: &mut neighbors,
        path: &paths,
    };
    assert_eq!(
        sender.send_pending(PEER, outgoing(), &mut buffer, |_| Ok(())),
        Ok(0)
    );
}

#[test]
fn an_empty_payload_is_one_frame_and_not_none() {
    let routes = routes();
    let mut neighbors = Cache::new();
    neighbors.on_confirmed(IpAddr::V6(PEER), PEER_HARDWARE, at(0));
    let paths = Paths::new();
    let (sent, frames) = send(&routes, &mut neighbors, &paths, outgoing(), &[]).expect("it goes");
    assert_eq!(sent, Sent::Frames(1));
    let packet = packet_of(&frames[0]);
    assert_eq!(packet.len(), HEADER_LEN);
}
