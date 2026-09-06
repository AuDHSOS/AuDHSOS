// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The socket table: which port a datagram finds, which it does not, and
//! where an ephemeral port comes from.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test builds a datagram at known offsets"
)]

use crypto_rng::doubles::ScriptedRng;
use net_wire::{IpAddr, Ipv4Addr, Ipv6Addr, Port, Writer};

use crate::datagram::{ChecksumPolicy, Datagram};
use crate::error::UdpError;
use crate::ring::DropReason;
use crate::socket::{Delivery, SocketId, Sockets};

/// The other host.
const PEER: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));

/// This host.
const HERE: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10));

/// A second address of this host.
const ALSO_HERE: IpAddr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

/// The broadcast address of this link.
const BROADCAST: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 255));

/// This host, over IPv6.
const HERE_V6: IpAddr = IpAddr::V6(Ipv6Addr::from_octets([
    0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01,
]));

/// The other host, over IPv6.
const PEER_V6: IpAddr = IpAddr::V6(Ipv6Addr::from_octets([
    0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x02,
]));

/// Writes a datagram into `buffer` and returns how long it is.
fn datagram(
    source: IpAddr,
    destination: IpAddr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
    buffer: &mut [u8],
) -> usize {
    let datagram = Datagram {
        source_port: Port::new(source_port),
        destination_port: Port::new(destination_port),
        payload,
    };
    let mut writer = Writer::new(buffer);
    datagram
        .write(&mut writer, source, destination, ChecksumPolicy::Computed)
        .expect("room for the datagram");
    writer.position()
}

#[test]
fn a_datagram_reaches_the_socket_that_holds_its_port() {
    let mut memory = [0u8; 128];
    let mut sockets = Sockets::<4>::new();
    assert!(sockets.is_empty());
    let id = sockets
        .bind(Some(HERE), Port::new(53), &mut memory)
        .expect("a free port");
    assert_eq!(sockets.len(), 1);
    assert_eq!(sockets.get(id).expect("the socket").port(), Port::new(53));
    assert_eq!(sockets.get(id).expect("the socket").local(), Some(HERE));

    let mut bytes = [0u8; 64];
    let len = datagram(PEER, HERE, 1024, 53, b"answer", &mut bytes);
    assert_eq!(
        sockets.receive(PEER, HERE, &bytes[..len]),
        Delivery::Delivered(id)
    );

    let socket = sockets.get(id).expect("the socket");
    assert_eq!(socket.len(), 1);
    let received = socket.peek().expect("a datagram");
    assert_eq!(received.source, PEER);
    assert_eq!(received.destination, HERE);
    assert_eq!(received.port, Port::new(1024));
    assert_eq!(received.payload, b"answer");
    assert!(sockets.get_mut(id).expect("the socket").discard());
    assert!(sockets.get(id).expect("the socket").is_empty());
}

#[test]
fn a_datagram_for_a_port_nobody_holds_asks_for_an_icmp_error() {
    let mut memory = [0u8; 128];
    let mut sockets = Sockets::<4>::new();
    sockets
        .bind(Some(HERE), Port::new(53), &mut memory)
        .expect("a free port");
    let mut bytes = [0u8; 64];
    let len = datagram(PEER, HERE, 1024, 54, b"nobody", &mut bytes);
    assert_eq!(
        sockets.receive(PEER, HERE, &bytes[..len]),
        Delivery::PortUnreachable
    );
}

#[test]
fn a_datagram_that_is_not_one_delivers_nothing() {
    let mut memory = [0u8; 128];
    let mut sockets = Sockets::<4>::new();
    sockets
        .bind(None, Port::new(53), &mut memory)
        .expect("a free port");
    let mut bytes = [0u8; 64];
    let len = datagram(PEER, HERE, 1024, 53, b"four", &mut bytes);
    bytes[7] ^= 0x01;
    let carried = u16::from_be_bytes([bytes[6], bytes[7]]);
    assert_eq!(
        sockets.receive(PEER, HERE, &bytes[..len]),
        Delivery::Malformed(UdpError::Checksum(carried))
    );
}

#[test]
fn a_wildcard_socket_takes_what_no_address_of_its_own_would() {
    let mut memory = [0u8; 256];
    let mut sockets = Sockets::<4>::new();
    let id = sockets
        .bind(None, Port::new(68), &mut memory)
        .expect("a free port");
    for destination in [HERE, ALSO_HERE, BROADCAST] {
        let mut bytes = [0u8; 64];
        let len = datagram(PEER, destination, 67, 68, b"offer", &mut bytes);
        assert_eq!(
            sockets.receive(PEER, destination, &bytes[..len]),
            Delivery::Delivered(id)
        );
    }
    assert_eq!(sockets.get(id).expect("the socket").len(), 3);
}

#[test]
fn a_socket_bound_to_one_address_takes_only_what_names_it() {
    let mut mine = [0u8; 128];
    let mut sockets = Sockets::<4>::new();
    let id = sockets
        .bind(Some(HERE), Port::new(53), &mut mine)
        .expect("a free port");
    let mut bytes = [0u8; 64];
    let len = datagram(PEER, ALSO_HERE, 1024, 53, b"elsewhere", &mut bytes);
    assert_eq!(
        sockets.receive(PEER, ALSO_HERE, &bytes[..len]),
        Delivery::PortUnreachable
    );
    assert!(sockets.get(id).expect("the socket").is_empty());
}

#[test]
fn a_port_is_held_once() {
    let mut first = [0u8; 64];
    let mut second = [0u8; 64];
    let mut sockets = Sockets::<4>::new();
    sockets
        .bind(Some(HERE), Port::new(53), &mut first)
        .expect("a free port");
    assert_eq!(
        sockets.bind(Some(ALSO_HERE), Port::new(53), &mut second),
        Err(UdpError::PortInUse(Port::new(53)))
    );
}

#[test]
fn port_zero_names_no_service() {
    let mut memory = [0u8; 64];
    let mut sockets = Sockets::<4>::new();
    assert_eq!(
        sockets.bind(None, Port::UNSPECIFIED, &mut memory),
        Err(UdpError::UnspecifiedPort)
    );
}

#[test]
fn a_full_table_takes_no_further_socket() {
    let mut first = [0u8; 32];
    let mut second = [0u8; 32];
    let mut third = [0u8; 32];
    let mut sockets = Sockets::<2>::new();
    sockets
        .bind(None, Port::new(1), &mut first)
        .expect("a free slot");
    sockets
        .bind(None, Port::new(2), &mut second)
        .expect("a free slot");
    assert_eq!(
        sockets.bind(None, Port::new(3), &mut third),
        Err(UdpError::NoSocket)
    );
}

#[test]
fn a_closed_socket_gives_its_memory_and_its_slot_back() {
    let mut memory = [0u8; 64];
    let mut sockets = Sockets::<2>::new();
    let id = sockets
        .bind(None, Port::new(53), &mut memory)
        .expect("a free port");
    let returned = sockets.close(id).expect("an open socket");
    assert_eq!(returned.len(), 64);
    assert!(sockets.is_empty());
    assert!(!sockets.is_bound(Port::new(53)));
    assert_eq!(sockets.close(id), Err(UdpError::UnknownSocket));
    assert!(sockets.get(id).is_none());
    // The slot is free again.
    sockets
        .bind(None, Port::new(53), returned)
        .expect("the slot came back");
}

#[test]
fn an_identifier_beyond_the_table_names_no_socket() {
    let mut memory = [0u8; 64];
    let mut sockets = Sockets::<1>::new();
    let id = sockets
        .bind(None, Port::new(53), &mut memory)
        .expect("a free port");
    assert_eq!(id.index(), 0);
    let beyond = SocketId::new(7);
    assert!(sockets.get(beyond).is_none());
    assert!(sockets.get_mut(beyond).is_none());
    assert_eq!(sockets.close(beyond), Err(UdpError::UnknownSocket));
    assert!(sockets.get_mut(id).is_some());
}

#[test]
fn a_datagram_the_ring_will_not_take_is_reported_as_dropped() {
    let mut memory = [0u8; 20];
    let mut sockets = Sockets::<2>::new();
    let id = sockets
        .bind(None, Port::new(53), &mut memory)
        .expect("a free port");
    let mut bytes = [0u8; 64];
    let len = datagram(PEER, HERE, 1024, 53, &[0u8; 16], &mut bytes);
    assert_eq!(
        sockets.receive(PEER, HERE, &bytes[..len]),
        Delivery::Dropped {
            socket: id,
            reason: DropReason::TooLarge,
        }
    );
    assert_eq!(sockets.get(id).expect("the socket").dropped(), 1);
}

#[test]
fn an_ephemeral_port_is_one_of_the_dynamic_range_and_is_free() {
    let mut memory = [0u8; 64];
    let mut sockets = Sockets::<4>::new();
    let mut rng = ScriptedRng::new(&[0x12, 0x34]);
    let id = sockets
        .bind_ephemeral(None, &mut rng, &mut memory)
        .expect("a free port");
    let port = sockets.get(id).expect("the socket").port();
    assert!(port.is_ephemeral(), "{port:?} is outside the dynamic range");
    assert_eq!(port, Port::new(0xC000 | 0x1234));
}

#[test]
fn the_low_fourteen_bits_are_what_name_the_port() {
    let mut memory = [0u8; 64];
    let mut sockets = Sockets::<4>::new();
    // The two high bits are not part of the range and are dropped.
    let mut rng = ScriptedRng::new(&[0xFF, 0xFF]);
    let id = sockets
        .bind_ephemeral(None, &mut rng, &mut memory)
        .expect("a free port");
    assert_eq!(
        sockets.get(id).expect("the socket").port(),
        Port::EPHEMERAL_LAST
    );
}

#[test]
fn a_port_already_taken_is_answered_by_drawing_again() {
    let mut first = [0u8; 64];
    let mut second = [0u8; 64];
    let mut sockets = Sockets::<4>::new();
    let taken = Port::new(0xC000 | 0x1234);
    sockets.bind(None, taken, &mut first).expect("a free port");
    let mut rng = ScriptedRng::new(&[0x12, 0x34, 0x12, 0x34, 0x00, 0x07]);
    let id = sockets
        .bind_ephemeral(None, &mut rng, &mut second)
        .expect("a free port");
    assert_eq!(
        sockets.get(id).expect("the socket").port(),
        Port::new(0xC000 | 0x0007)
    );
}

#[test]
fn a_search_that_only_ever_meets_taken_ports_gives_up() {
    let mut first = [0u8; 64];
    let mut second = [0u8; 64];
    let mut sockets = Sockets::<4>::new();
    let taken = Port::new(0xC000 | 0x1234);
    sockets.bind(None, taken, &mut first).expect("a free port");
    let script = [0x12, 0x34].repeat(8);
    let mut rng = ScriptedRng::new(&script);
    assert_eq!(
        sockets.bind_ephemeral(None, &mut rng, &mut second),
        Err(UdpError::NoPort)
    );
}

#[test]
fn a_generator_that_fails_leaves_the_socket_unbound() {
    let mut memory = [0u8; 64];
    let mut sockets = Sockets::<4>::new();
    let mut rng = ScriptedRng::new(&[]);
    assert_eq!(
        sockets.bind_ephemeral(None, &mut rng, &mut memory),
        Err(UdpError::Rng(crypto_rng::RngError::Exhausted))
    );
    assert!(sockets.is_empty());
}

#[test]
fn one_socket_serves_both_families() {
    let mut memory = [0u8; 256];
    let mut sockets = Sockets::<4>::new();
    let id = sockets
        .bind(None, Port::new(53), &mut memory)
        .expect("a free port");
    let mut over_v4 = [0u8; 64];
    let len = datagram(PEER, HERE, 1024, 53, b"four", &mut over_v4);
    assert_eq!(
        sockets.receive(PEER, HERE, &over_v4[..len]),
        Delivery::Delivered(id)
    );
    let mut over_v6 = [0u8; 64];
    let len = datagram(PEER_V6, HERE_V6, 1024, 53, b"six", &mut over_v6);
    assert_eq!(
        sockets.receive(PEER_V6, HERE_V6, &over_v6[..len]),
        Delivery::Delivered(id)
    );
    let socket = sockets.get(id).expect("the socket");
    assert_eq!(socket.len(), 2);
    assert_eq!(socket.peek().expect("a datagram").source, PEER);
}

#[test]
fn a_table_made_by_default_is_the_empty_one() {
    let sockets = Sockets::<4>::default();
    assert!(sockets.is_empty());
    assert!(!sockets.is_bound(Port::new(53)));
}
