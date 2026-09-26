// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Two servers on one link: a connection opened, bytes both ways, and a
//! close both ends see.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test reads a ring at known offsets"
)]

use audhsos_abi::Error;
use audhsos_time::Instant;
use net_wire::{IpAddr, Ipv4Addr, MacAddr};
use user_proto::socket::{Direction, Endpoint, Reply, Request, State};

use crate::server::NOBODY;

use super::support::{Host, PORT, against_the_link, exchange, host, process, rng};

/// The hardware address of the host that connects.
const ONE_MAC: MacAddr = MacAddr::new([0x52, 0x54, 0x00, 0x00, 0x00, 1]);

/// The hardware address of the host that listens.
const TWO_MAC: MacAddr = MacAddr::new([0x52, 0x54, 0x00, 0x00, 0x00, 2]);

/// The address of the host that connects.
const ONE: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);

/// The address of the host that listens.
const TWO: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);

/// The badge of the client of each host.
const CLIENT: u64 = 1;

/// The socket a reply opened, or the failure it carried.
fn opened(reply: Reply) -> Result<u32, Error> {
    match reply {
        Reply::Connected(outcome) | Reply::Accepted(outcome) | Reply::Bound(outcome) => {
            outcome.map(|opened| opened.socket)
        }
        Reply::Listening(outcome) => outcome,
        other => panic!("{other:?} opened nothing"),
    }
}

/// Two hosts, one listening on [`PORT`] and one connected to it, with the
/// sockets of each.
fn connected(from: Instant) -> (Host, Host, u32, u32, Instant) {
    let now = from;
    let mut one = host(ONE_MAC, ONE);
    let mut two = host(TWO_MAC, TWO);
    let mut generator = rng(3);
    let listener = opened(two.server.answer(
        CLIENT,
        &Request::TcpListen {
            port: PORT,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a listener");
    let client = opened(one.server.answer(
        CLIENT,
        &Request::TcpConnect {
            remote: Endpoint::new(IpAddr::V4(TWO), PORT),
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a connection");
    let now = exchange(&mut one, &mut two, now, &mut generator);
    let accepted = opened(two.server.answer(
        CLIENT,
        &Request::TcpAccept { socket: listener },
        now,
        &mut generator,
    ))
    .expect("the listener took the connection");
    (one, two, client, accepted, now)
}

/// Where a connection stands.
fn state(host: &mut Host, socket: u32, now: Instant) -> State {
    let mut generator = rng(5);
    match host
        .server
        .answer(CLIENT, &Request::TcpState { socket }, now, &mut generator)
    {
        Reply::State(Ok(state)) => state,
        other => panic!("{other:?} is no state"),
    }
}

#[test]
fn the_interface_reports_the_address_of_the_device() {
    let now = Instant::from_micros(0);
    let mut one = host(ONE_MAC, ONE);
    let mut generator = rng(1);
    match one
        .server
        .answer(CLIENT, &Request::Interface, now, &mut generator)
    {
        Reply::Interface(Ok(interface)) => {
            assert_eq!(interface.mac, ONE_MAC);
            assert_eq!(interface.addresses.iter().next(), Some(IpAddr::V4(ONE)));
            assert!(!interface.lease, "nothing granted one");
        }
        other => panic!("{other:?} is no interface"),
    }
}

#[test]
fn an_accept_with_nothing_to_take_answers_would_block() {
    let now = Instant::from_micros(0);
    let mut two = host(TWO_MAC, TWO);
    let mut generator = rng(2);
    let listener = opened(two.server.answer(
        CLIENT,
        &Request::TcpListen {
            port: PORT,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a listener");
    match two.server.answer(
        CLIENT,
        &Request::TcpAccept { socket: listener },
        now,
        &mut generator,
    ) {
        Reply::Accepted(Err(error)) => assert_eq!(error, Error::WouldBlock),
        other => panic!("{other:?} took a connection nobody opened"),
    }
}

#[test]
fn a_connection_is_opened_and_both_ends_call_it_established() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, client, accepted, now) = connected(now);
    assert_eq!(state(&mut one, client, now), State::Established);
    assert_eq!(state(&mut two, accepted, now), State::Established);
}

#[test]
fn bytes_go_both_ways_through_the_rings() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, client, accepted, now) = connected(now);
    let mut generator = rng(7);

    let sent = b"a request of the client";
    assert_eq!(one.pages[0].outbound.write(sent), sent.len());
    let moved = match one.server.answer(
        CLIENT,
        &Request::TcpSend {
            socket: client,
            len: u32::try_from(sent.len()).unwrap_or(0),
        },
        now,
        &mut generator,
    ) {
        Reply::Sent(Ok(moved)) => moved,
        other => panic!("{other:?} sent nothing"),
    };
    assert_eq!(usize::try_from(moved).unwrap_or(0), sent.len());
    let now = exchange(&mut one, &mut two, now, &mut generator);

    // The exchange pumped the bytes into the ring already.
    assert_eq!(
        usize::try_from(two.pages[0].inbound.held()).unwrap_or(0),
        sent.len()
    );
    let mut into = [0u8; 64];
    let taken = two.pages[0].inbound.read(&mut into);
    assert_eq!(&into[..taken], sent);

    let answer = b"an answer of the server";
    assert_eq!(two.pages[0].outbound.write(answer), answer.len());
    let _sent = two.server.answer(
        CLIENT,
        &Request::TcpSend {
            socket: accepted,
            len: u32::try_from(answer.len()).unwrap_or(0),
        },
        now,
        &mut generator,
    );
    let now = exchange(&mut one, &mut two, now, &mut generator);
    let _received = one.server.answer(
        CLIENT,
        &Request::TcpRecv { socket: client },
        now,
        &mut generator,
    );
    let taken = one.pages[0].inbound.read(&mut into);
    assert_eq!(&into[..taken], answer);
}

#[test]
fn a_shutdown_of_one_end_reaches_the_other_as_a_peer_that_closed() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, client, accepted, now) = connected(now);
    let mut generator = rng(11);
    match one.server.answer(
        CLIENT,
        &Request::TcpShutdown {
            socket: client,
            direction: Direction::Write,
        },
        now,
        &mut generator,
    ) {
        Reply::ShutDown(Ok(())) => {}
        other => panic!("{other:?} closed nothing"),
    }
    let now = exchange(&mut one, &mut two, now, &mut generator);
    assert_eq!(state(&mut two, accepted, now), State::PeerClosed);
}

#[test]
fn a_socket_that_was_closed_is_answered_for_nobody() {
    let now = Instant::from_micros(0);
    let (mut one, _two, client, _accepted, now) = connected(now);
    let mut generator = rng(13);
    match one.server.answer(
        CLIENT,
        &Request::TcpClose { socket: client },
        now,
        &mut generator,
    ) {
        Reply::Closed(Ok(())) => {}
        other => panic!("{other:?} closed nothing"),
    }
    assert_eq!(one.server.open(), 0);
    match one.server.answer(
        CLIENT,
        &Request::TcpState { socket: client },
        now,
        &mut generator,
    ) {
        Reply::State(Err(error)) => assert_eq!(error, Error::NotFound),
        other => panic!("{other:?} answered for a socket that is gone"),
    }
}

#[test]
fn a_client_that_is_gone_gives_up_every_socket_it_held() {
    let now = Instant::from_micros(0);
    let (mut one, _two, _client, _accepted, _reached) = connected(now);
    assert_eq!(one.server.open(), 1);
    one.server.forget(CLIENT, now);
    assert_eq!(one.server.open(), 0);
}

#[test]
fn a_client_that_never_reads_fills_its_ring_and_the_server_stops_taking_bytes() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, client, accepted, now) = connected(now);
    let mut generator = rng(17);
    let capacity = usize::try_from(user_proto::ring::RING_CAPACITY).unwrap_or(0);
    // The sender writes more than the reader's ring holds, in as many
    // rounds as the windows take.
    let bulk = vec![0x5Au8; 1024];
    let mut now = now;
    for _ in 0..16 {
        let taken = one.pages[0].outbound.write(&bulk);
        let _sent = one.server.answer(
            CLIENT,
            &Request::TcpSend {
                socket: client,
                len: u32::try_from(taken).unwrap_or(0),
            },
            now,
            &mut generator,
        );
        now = exchange(&mut one, &mut two, now, &mut generator);
    }
    let held = two.pages[0].inbound.held();
    assert!(
        usize::try_from(held).unwrap_or(0) <= capacity,
        "the ring took more than it holds"
    );
    assert!(
        usize::try_from(held).unwrap_or(0) >= capacity - 1024,
        "the ring did not fill: {held} of {capacity}"
    );
    assert_eq!(
        two.pages[0].inbound.take_dropped(),
        0,
        "the server wrote past the ring rather than stopping"
    );
    // Reading makes room, and the connection carries on: what the sender
    // writes next arrives rather than being refused.
    let mut into = vec![0u8; 512];
    assert_eq!(two.pages[0].inbound.read(&mut into), 512);
    let now = exchange(&mut one, &mut two, now, &mut generator);
    let _received = two.server.answer(
        CLIENT,
        &Request::TcpRecv { socket: accepted },
        now,
        &mut generator,
    );
    assert!(
        two.pages[0].inbound.held() > held - 512,
        "the connection did not carry on"
    );
}

#[test]
fn a_request_for_a_socket_of_another_client_is_refused() {
    let now = Instant::from_micros(0);
    let (mut one, _two, client, _accepted, now) = connected(now);
    let mut generator = rng(19);
    match one.server.answer(
        2,
        &Request::TcpState { socket: client },
        now,
        &mut generator,
    ) {
        Reply::State(Err(error)) => assert_eq!(error, Error::NotFound),
        other => panic!("{other:?} answered another client's socket"),
    }
}

#[test]
fn a_resolution_with_no_name_server_is_refused_rather_than_left_running() {
    let now = Instant::from_micros(0);
    let mut one = host(ONE_MAC, ONE);
    let mut generator = rng(23);
    let name = user_proto::socket::Name::new(b"example.test").expect("a short name");
    match one
        .server
        .answer(CLIENT, &Request::Resolve { name }, now, &mut generator)
    {
        Reply::Resolved(Err(error)) => assert_eq!(error, Error::Unavailable),
        other => panic!("{other:?} resolved without a name server"),
    }
}

#[test]
fn a_datagram_socket_is_opened_and_given_back() {
    let now = Instant::from_micros(0);
    let mut one = host(ONE_MAC, ONE);
    let mut generator = rng(29);
    let socket = opened(one.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 5353,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a datagram socket");
    assert_eq!(one.server.open(), 1);
    match one
        .server
        .answer(CLIENT, &Request::UdpClose { socket }, now, &mut generator)
    {
        Reply::UdpClosed(Ok(())) => {}
        other => panic!("{other:?} closed nothing"),
    }
    assert_eq!(one.server.open(), 0);
}

#[test]
fn a_connection_request_on_a_datagram_socket_is_refused_by_type() {
    let now = Instant::from_micros(0);
    let mut one = host(ONE_MAC, ONE);
    let mut generator = rng(31);
    let socket = opened(one.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 5353,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a datagram socket");
    match one
        .server
        .answer(CLIENT, &Request::TcpRecv { socket }, now, &mut generator)
    {
        Reply::Received(Err(error)) => assert_eq!(error, Error::WrongObjectType),
        other => panic!("{other:?} read a datagram socket as a connection"),
    }
}

#[test]
fn a_region_shorter_than_the_pieces_makes_no_server() {
    use crate::memory::REGION_BYTES;
    use crate::server::Rings;
    use crate::sockets::MAX_SOCKETS;
    use audhsos_abi::Handle;
    use net_stack::Config;
    use user_proto::ring::SocketPage;

    let bytes: &'static mut [u8] = Box::leak(vec![0u8; REGION_BYTES - 1].into_boxed_slice());
    let pages: [&'static SocketPage; MAX_SOCKETS] =
        core::array::from_fn(|_| &*Box::leak(Box::new(SocketPage::new())));
    let rings: [Rings<'static>; MAX_SOCKETS] = core::array::from_fn(|index| Rings {
        page: pages[index],
        object: Handle::new(u32::try_from(index).unwrap_or(0), 1).expect("a handle"),
    });
    assert!(crate::server::Server::new(Config::new(ONE_MAC), bytes, rings).is_none());
}

#[test]
fn every_refusal_of_the_stack_becomes_one_of_the_interface() {
    use crate::server::refusal;
    use net_stack::StackError;
    assert_eq!(refusal(StackError::Full), Error::OutOfMemory);
    assert_eq!(refusal(StackError::Busy), Error::Busy);
    assert_eq!(refusal(StackError::Stale), Error::InvalidHandle);
    assert_eq!(refusal(StackError::Unknown), Error::InvalidHandle);
    assert_eq!(refusal(StackError::NoAddress), Error::Unavailable);
    assert_eq!(refusal(StackError::Unresolved), Error::Unavailable);
    assert_eq!(refusal(StackError::Unreachable), Error::InvalidArgument);
}

#[test]
fn a_listen_on_a_host_with_no_address_is_refused() {
    let now = Instant::from_micros(0);
    let bytes: &'static mut [u8] =
        Box::leak(vec![0u8; crate::memory::REGION_BYTES].into_boxed_slice());
    let pages: [&'static user_proto::ring::SocketPage; crate::sockets::MAX_SOCKETS] =
        core::array::from_fn(|_| &*Box::leak(Box::new(user_proto::ring::SocketPage::new())));
    let rings: [crate::server::Rings<'static>; crate::sockets::MAX_SOCKETS] =
        core::array::from_fn(|index| crate::server::Rings {
            page: pages[index],
            object: audhsos_abi::Handle::new(u32::try_from(index).unwrap_or(0), 1)
                .expect("a handle"),
        });
    let mut server = crate::server::Server::new(net_stack::Config::new(ONE_MAC), bytes, rings)
        .expect("the region is long enough");
    let mut generator = rng(37);
    match server.answer(
        CLIENT,
        &Request::TcpListen {
            port: PORT,
            process: process(),
        },
        now,
        &mut generator,
    ) {
        Reply::Listening(Err(error)) => assert_eq!(error, Error::Unavailable),
        other => panic!("{other:?} listened without an address"),
    }
}

#[test]
fn every_message_that_names_a_socket_refuses_one_nobody_holds() {
    let now = Instant::from_micros(0);
    let mut one = host(ONE_MAC, ONE);
    let mut generator = rng(41);
    let unknown = 0xDEAD_BEEF;
    let refusals = [
        matches!(
            one.server.answer(
                CLIENT,
                &Request::TcpRecv { socket: unknown },
                now,
                &mut generator
            ),
            Reply::Received(Err(Error::NotFound))
        ),
        matches!(
            one.server.answer(
                CLIENT,
                &Request::TcpSend {
                    socket: unknown,
                    len: 1
                },
                now,
                &mut generator
            ),
            Reply::Sent(Err(Error::NotFound))
        ),
        matches!(
            one.server.answer(
                CLIENT,
                &Request::TcpClose { socket: unknown },
                now,
                &mut generator
            ),
            Reply::Closed(Err(Error::NotFound))
        ),
        matches!(
            one.server.answer(
                CLIENT,
                &Request::UdpClose { socket: unknown },
                now,
                &mut generator
            ),
            Reply::UdpClosed(Err(Error::NotFound))
        ),
        matches!(
            one.server.answer(
                CLIENT,
                &Request::TcpAccept { socket: unknown },
                now,
                &mut generator
            ),
            Reply::Accepted(Err(Error::NotFound))
        ),
        matches!(
            one.server.answer(
                CLIENT,
                &Request::TcpShutdown {
                    socket: unknown,
                    direction: Direction::Both
                },
                now,
                &mut generator
            ),
            Reply::ShutDown(Err(Error::NotFound))
        ),
        matches!(
            one.server.answer(
                CLIENT,
                &Request::UdpSendTo {
                    socket: unknown,
                    remote: Endpoint::new(IpAddr::V4(TWO), 9),
                    len: 4
                },
                now,
                &mut generator
            ),
            Reply::UdpSent(Err(Error::NotFound))
        ),
    ];
    assert!(
        refusals.into_iter().all(|refused| refused),
        "a message answered for a socket nobody holds"
    );
}

#[test]
fn an_accept_of_something_that_is_no_listener_is_refused_by_state() {
    let now = Instant::from_micros(0);
    let (mut one, _two, client, _accepted, _reached) = connected(now);
    let mut generator = rng(43);
    match one.server.answer(
        CLIENT,
        &Request::TcpAccept { socket: client },
        now,
        &mut generator,
    ) {
        Reply::Accepted(Err(error)) => assert_eq!(error, Error::InvalidState),
        other => panic!("{other:?} accepted a connection that was already open"),
    }
}

#[test]
fn a_shutdown_of_the_reading_half_alone_changes_nothing_on_the_link() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, client, accepted, now) = connected(now);
    let mut generator = rng(47);
    match one.server.answer(
        CLIENT,
        &Request::TcpShutdown {
            socket: client,
            direction: Direction::Read,
        },
        now,
        &mut generator,
    ) {
        Reply::ShutDown(Ok(())) => {}
        other => panic!("{other:?} refused to stop reading"),
    }
    let now = exchange(&mut one, &mut two, now, &mut generator);
    assert_eq!(state(&mut two, accepted, now), State::Established);
}

#[test]
fn a_datagram_socket_sends_what_the_client_put_into_the_ring() {
    let now = Instant::from_micros(0);
    let mut one = host(ONE_MAC, ONE);
    let mut generator = rng(53);
    let socket = opened(one.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 0,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a datagram socket on a port of the dynamic range");
    let payload = b"a datagram";
    assert_eq!(one.pages[0].outbound.write(payload), payload.len());
    match one.server.answer(
        CLIENT,
        &Request::UdpSendTo {
            socket,
            remote: Endpoint::new(IpAddr::V4(TWO), 9),
            len: u32::try_from(payload.len()).unwrap_or(0),
        },
        now,
        &mut generator,
    ) {
        Reply::UdpSent(Ok(moved)) => {
            assert_eq!(usize::try_from(moved).unwrap_or(0), payload.len());
        }
        other => panic!("{other:?} sent no datagram"),
    }
}

#[test]
fn the_sockets_run_out_and_the_next_one_is_refused() {
    let now = Instant::from_micros(0);
    let mut one = host(ONE_MAC, ONE);
    let mut generator = rng(59);
    for index in 0..crate::sockets::MAX_SOCKETS {
        let port = 4000 + u16::try_from(index).unwrap_or(0);
        assert!(
            opened(one.server.answer(
                CLIENT,
                &Request::UdpBind {
                    port,
                    process: process(),
                },
                now,
                &mut generator
            ))
            .is_ok(),
            "a slot was not free"
        );
    }
    match one.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 4999,
            process: process(),
        },
        now,
        &mut generator,
    ) {
        Reply::Bound(Err(error)) => assert!(
            error == Error::OutOfHandles || error == Error::OutOfMemory,
            "{error:?}"
        ),
        other => panic!("{other:?} opened a socket with no slot left"),
    }
}

#[test]
fn a_stack_with_nothing_to_do_names_no_instant() {
    let now = Instant::from_micros(0);
    let one = host(ONE_MAC, ONE);
    assert_eq!(one.server.poll_at(now), None);
}

/// A host that has taken a lease from the link host of [`super::link`].
fn leased(from: Instant) -> (Host, Instant, super::support::Rng) {
    let mut one = host(ONE_MAC, ONE);
    let mut generator = rng(61);
    one.server.stack().configure(from);
    let now = against_the_link(&mut one, from, &mut generator);
    (one, now, generator)
}

#[test]
fn the_address_configuration_client_reaches_a_lease_and_the_interface_says_so() {
    let now = Instant::from_micros(0);
    let (mut one, now, mut generator) = leased(now);
    match one
        .server
        .answer(CLIENT, &Request::Interface, now, &mut generator)
    {
        Reply::Interface(Ok(interface)) => {
            assert!(interface.lease, "no lease was taken");
            assert!(
                interface
                    .addresses
                    .iter()
                    .any(|address| address == IpAddr::V4(super::link::LEASED)),
                "the address of the lease is not one of this host's"
            );
            assert_eq!(
                interface.gateway,
                Some(IpAddr::V4(super::link::SERVER)),
                "the router of the lease is not the gateway"
            );
        }
        other => panic!("{other:?} is no interface"),
    }
}

#[test]
fn a_name_the_server_knows_is_resolved_to_its_address() {
    let now = Instant::from_micros(0);
    let (mut one, now, mut generator) = leased(now);
    let name =
        user_proto::socket::Name::new(super::link::KNOWN_NAME.as_bytes()).expect("a short name");
    // The first ask starts the resolution and answers `WouldBlock`.
    match one
        .server
        .answer(CLIENT, &Request::Resolve { name }, now, &mut generator)
    {
        Reply::Resolved(Err(error)) => assert_eq!(error, Error::WouldBlock),
        other => panic!("{other:?} answered before it asked"),
    }
    let mut now = now;
    for _ in 0..32 {
        now = against_the_link(&mut one, now, &mut generator);
        match one
            .server
            .answer(CLIENT, &Request::Resolve { name }, now, &mut generator)
        {
            Reply::Resolved(Ok(addresses)) => {
                assert!(
                    addresses
                        .iter()
                        .any(|address| address == IpAddr::V4(super::link::KNOWN_ADDRESS)),
                    "the name resolved to {addresses:?}"
                );
                return;
            }
            Reply::Resolved(Err(Error::WouldBlock)) => {}
            other => panic!("{other:?} resolved nothing"),
        }
    }
    panic!("the resolution never ended");
}

#[test]
fn a_second_client_asking_while_a_resolution_runs_is_told_the_server_is_busy() {
    let now = Instant::from_micros(0);
    let (mut one, now, mut generator) = leased(now);
    let name =
        user_proto::socket::Name::new(super::link::KNOWN_NAME.as_bytes()).expect("a short name");
    let _started = one
        .server
        .answer(CLIENT, &Request::Resolve { name }, now, &mut generator);
    match one
        .server
        .answer(CLIENT + 1, &Request::Resolve { name }, now, &mut generator)
    {
        Reply::Resolved(Err(error)) => assert_eq!(error, Error::Busy),
        other => panic!("{other:?} ran two resolutions at once"),
    }
}

#[test]
fn a_name_nobody_answers_ends_as_not_found_and_the_socket_goes_back() {
    let now = Instant::from_micros(0);
    let (mut one, now, mut generator) = leased(now);
    let name = user_proto::socket::Name::new(b"nobody.test").expect("a short name");
    let _started = one
        .server
        .answer(CLIENT, &Request::Resolve { name }, now, &mut generator);
    assert_eq!(one.server.open(), 0, "the resolver's socket is a client's");
    // The link host answers nothing, so the resolver runs out of tries.
    let mut now = now;
    for _ in 0..64 {
        now = against_the_link(&mut one, now, &mut generator);
    }
    match one
        .server
        .answer(CLIENT, &Request::Resolve { name }, now, &mut generator)
    {
        Reply::Resolved(Err(error)) => assert!(
            error == Error::NotFound || error == Error::WouldBlock,
            "{error:?}"
        ),
        other => panic!("{other:?} resolved a name nobody answers"),
    }
}

#[test]
fn a_client_that_is_gone_ends_the_resolution_it_started() {
    let now = Instant::from_micros(0);
    let (mut one, now, mut generator) = leased(now);
    let name =
        user_proto::socket::Name::new(super::link::KNOWN_NAME.as_bytes()).expect("a short name");
    let _started = one
        .server
        .answer(CLIENT, &Request::Resolve { name }, now, &mut generator);
    one.server.forget(CLIENT, now);
    // The next client's resolution starts rather than meeting a busy
    // server, which is what says the first one was given up.
    match one
        .server
        .answer(CLIENT + 1, &Request::Resolve { name }, now, &mut generator)
    {
        Reply::Resolved(Err(error)) => assert_eq!(error, Error::WouldBlock),
        other => panic!("{other:?} did not start a resolution"),
    }
}

#[test]
fn a_datagram_arrives_in_the_ring_behind_the_record_of_where_it_came_from() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, _client, _accepted, now) = connected(now);
    let mut generator = rng(67);
    let _taking = opened(one.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 9,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a datagram socket");
    let sender = opened(two.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 0,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a datagram socket");
    let payload = b"a datagram of the link";
    assert_eq!(two.pages[1].outbound.write(payload), payload.len());
    let _sent = two.server.answer(
        CLIENT,
        &Request::UdpSendTo {
            socket: sender,
            remote: Endpoint::new(IpAddr::V4(ONE), 9),
            len: u32::try_from(payload.len()).unwrap_or(0),
        },
        now,
        &mut generator,
    );
    let _reached = exchange(&mut one, &mut two, now, &mut generator);
    let mut header = [0u8; user_proto::socket::DATAGRAM_HEADER_LEN];
    assert_eq!(
        one.pages[1].inbound.read(&mut header),
        user_proto::socket::DATAGRAM_HEADER_LEN,
        "no record arrived"
    );
    let (from, len) =
        user_proto::socket::read_datagram_header(&header).expect("a record of a datagram");
    assert_eq!(from.address, IpAddr::V4(TWO));
    assert_eq!(len, payload.len());
    let mut body = vec![0u8; len];
    assert_eq!(one.pages[1].inbound.read(&mut body), len);
    assert_eq!(body, payload);
}

#[test]
fn a_name_that_exists_and_has_no_address_is_not_found() {
    let now = Instant::from_micros(0);
    let (mut one, now, mut generator) = leased(now);
    let name =
        user_proto::socket::Name::new(super::link::EMPTY_NAME.as_bytes()).expect("a short name");
    let mut now = now;
    for _ in 0..32 {
        match one
            .server
            .answer(CLIENT, &Request::Resolve { name }, now, &mut generator)
        {
            Reply::Resolved(Err(Error::WouldBlock)) => {}
            Reply::Resolved(Err(error)) => {
                assert_eq!(error, Error::NotFound);
                return;
            }
            other => panic!("{other:?} found an address for a name that has none"),
        }
        now = against_the_link(&mut one, now, &mut generator);
    }
    panic!("the resolution never ended");
}

#[test]
fn a_socket_of_one_kind_is_refused_by_the_messages_of_the_other() {
    let now = Instant::from_micros(0);
    let (mut one, _two, client, _accepted, now) = connected(now);
    let mut generator = rng(71);
    let datagram = opened(one.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 5000,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a datagram socket");
    match one.server.answer(
        CLIENT,
        &Request::TcpClose { socket: datagram },
        now,
        &mut generator,
    ) {
        Reply::Closed(Err(error)) => assert_eq!(error, Error::WrongObjectType),
        other => panic!("{other:?} closed a datagram socket as a connection"),
    }
    match one.server.answer(
        CLIENT,
        &Request::UdpClose { socket: client },
        now,
        &mut generator,
    ) {
        Reply::UdpClosed(Err(error)) => assert_eq!(error, Error::WrongObjectType),
        other => panic!("{other:?} closed a connection as a datagram socket"),
    }
    match one.server.answer(
        CLIENT,
        &Request::TcpState { socket: datagram },
        now,
        &mut generator,
    ) {
        Reply::State(Err(error)) => assert_eq!(error, Error::WrongObjectType),
        other => panic!("{other:?} answered a state for a datagram socket"),
    }
}

#[test]
fn a_send_with_an_empty_ring_moves_nothing() {
    let now = Instant::from_micros(0);
    let (mut one, _two, client, _accepted, now) = connected(now);
    let mut generator = rng(73);
    match one.server.answer(
        CLIENT,
        &Request::TcpSend {
            socket: client,
            len: 16,
        },
        now,
        &mut generator,
    ) {
        Reply::Sent(Ok(moved)) => assert_eq!(moved, 0),
        other => panic!("{other:?} sent bytes nobody wrote"),
    }
    match one.server.answer(
        CLIENT,
        &Request::TcpRecv { socket: client },
        now,
        &mut generator,
    ) {
        Reply::Received(Ok(held)) => assert_eq!(held, 0),
        other => panic!("{other:?} received bytes nobody sent"),
    }
}

#[test]
fn a_connection_to_a_port_nobody_listens_on_is_refused() {
    let now = Instant::from_micros(0);
    let mut one = host(ONE_MAC, ONE);
    let mut two = host(TWO_MAC, TWO);
    let mut generator = rng(79);
    let client = opened(one.server.answer(
        CLIENT,
        &Request::TcpConnect {
            remote: Endpoint::new(IpAddr::V4(TWO), 9),
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a connection was opened");
    let mut now = now;
    for _ in 0..8 {
        now = exchange(&mut one, &mut two, now, &mut generator);
        if state(&mut one, client, now) == State::Refused {
            return;
        }
    }
    panic!("the connection was never refused");
}

#[test]
fn a_datagram_send_on_a_connection_is_refused_by_type() {
    let now = Instant::from_micros(0);
    let (mut one, _two, client, _accepted, now) = connected(now);
    let mut generator = rng(83);
    match one.server.answer(
        CLIENT,
        &Request::UdpSendTo {
            socket: client,
            remote: Endpoint::new(IpAddr::V4(TWO), 9),
            len: 4,
        },
        now,
        &mut generator,
    ) {
        Reply::UdpSent(Err(error)) => assert_eq!(error, Error::WrongObjectType),
        other => panic!("{other:?} sent a datagram over a connection"),
    }
}

#[test]
fn a_datagram_the_clients_ring_has_no_room_for_waits_rather_than_being_torn() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, _client, _accepted, now) = connected(now);
    let mut generator = rng(89);
    let _taking = opened(one.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 9,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a datagram socket");
    let sender = opened(two.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 0,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a datagram socket");
    // The client of the receiving host reads nothing and its ring is
    // full, so the record has nowhere to go.
    let full = vec![0u8; usize::try_from(user_proto::ring::RING_CAPACITY).unwrap_or(0)];
    assert_eq!(one.pages[1].inbound.write(&full), full.len());
    let payload = b"a datagram with no room";
    assert_eq!(two.pages[1].outbound.write(payload), payload.len());
    let _sent = two.server.answer(
        CLIENT,
        &Request::UdpSendTo {
            socket: sender,
            remote: Endpoint::new(IpAddr::V4(ONE), 9),
            len: u32::try_from(payload.len()).unwrap_or(0),
        },
        now,
        &mut generator,
    );
    let _reached = exchange(&mut one, &mut two, now, &mut generator);
    assert_eq!(
        one.pages[1].inbound.free(),
        0,
        "the server wrote past the ring"
    );
}

#[test]
fn the_lease_is_renewed_when_its_first_timer_passes() {
    let now = Instant::from_micros(0);
    let (mut one, _reached, mut generator) = leased(now);
    let granted = one
        .server
        .stack()
        .lease()
        .map(|lease| lease.granted)
        .expect("a lease");
    let renew = one
        .server
        .stack()
        .lease()
        .map(|lease| lease.renew)
        .expect("a lease");
    assert!(renew > granted, "T1 is not ahead of the grant");
    // The clock moves past T1, which is when RFC 2131 has the client
    // renew, and the station answers the request with an acknowledgment.
    let at = Instant::from_micros(renew.as_micros().saturating_add(1_000_000));
    let _reached = against_the_link(&mut one, at, &mut generator);
    let again = one
        .server
        .stack()
        .lease()
        .map(|lease| lease.granted)
        .expect("a lease");
    assert!(again > granted, "the lease was not renewed");
}

#[test]
fn a_host_with_no_address_at_all_asks_for_one() {
    let now = Instant::from_micros(0);
    let mut one = super::support::bare(ONE_MAC);
    let mut generator = rng(97);
    one.server.stack().configure(now);
    let mut tx = [0u8; net_stack::FRAME_LEN];
    let frame = one
        .server
        .poll(now, None, &mut tx, &mut generator)
        .expect("a poll of the server")
        .expect("the address configuration client has a discover to send");
    // A discover is a broadcast frame carrying IPv4, and nothing else the
    // stack says at this point is one.
    assert_eq!(
        frame.get(..6),
        Some([0xFFu8; 6].as_slice()),
        "the first frame is not a broadcast"
    );
    assert_eq!(frame.get(12..14), Some([0x08u8, 0x00].as_slice()));
}
#[test]
fn bytes_and_a_shutdown_in_one_round_both_reach_the_other_end() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, client, accepted, now) = connected(now);
    let mut generator = rng(29);

    let answer = b"HTTP/1.1 200 OK\r\n\r\nhello, world\n";
    assert_eq!(two.pages[0].outbound.write(answer), answer.len());
    let _sent = two.server.answer(
        CLIENT,
        &Request::TcpSend {
            socket: accepted,
            len: u32::try_from(answer.len()).unwrap_or(0),
        },
        now,
        &mut generator,
    );
    let _shut = two.server.answer(
        CLIENT,
        &Request::TcpShutdown {
            socket: accepted,
            direction: Direction::Write,
        },
        now,
        &mut generator,
    );
    let now = exchange(&mut one, &mut two, now, &mut generator);

    let _received = one.server.answer(
        CLIENT,
        &Request::TcpRecv { socket: client },
        now,
        &mut generator,
    );
    let mut into = [0u8; 64];
    let taken = one.pages[0].inbound.read(&mut into);
    assert_eq!(
        &into[..taken],
        answer,
        "the bytes of a peer that closed right after them were lost"
    );
    assert_eq!(state(&mut one, client, now), State::PeerClosed);
}

/// Puts `bytes` into the outbound ring of `socket` and tells the server.
fn give(
    host: &mut Host,
    socket: u32,
    bytes: &[u8],
    now: Instant,
    generator: &mut super::support::Rng,
) {
    assert_eq!(host.pages[0].outbound.write(bytes), bytes.len());
    let _sent = host.server.answer(
        CLIENT,
        &Request::TcpSend {
            socket,
            len: u32::try_from(bytes.len()).unwrap_or(0),
        },
        now,
        generator,
    );
}

/// Asks for what arrived on `socket` and takes it out of the ring.
fn taken(
    host: &mut Host,
    socket: u32,
    now: Instant,
    generator: &mut super::support::Rng,
) -> Vec<u8> {
    let _received = host
        .server
        .answer(CLIENT, &Request::TcpRecv { socket }, now, generator);
    let mut into = [0u8; 512];
    let len = host.pages[0].inbound.read(&mut into);
    into[..len].to_vec()
}

#[test]
fn the_whole_exchange_of_the_reference_machine_runs_through_the_rings() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, client, accepted, now) = connected(now);
    let mut generator = rng(31);

    let echo = b"a line the runner sends to be echoed\n";
    give(&mut one, client, echo, now, &mut generator);
    let now = exchange(&mut one, &mut two, now, &mut generator);
    assert_eq!(taken(&mut two, accepted, now, &mut generator), echo);

    give(&mut two, accepted, echo, now, &mut generator);
    let now = exchange(&mut one, &mut two, now, &mut generator);
    assert_eq!(taken(&mut one, client, now, &mut generator), echo);

    let get = b"GET / HTTP/1.1\r\nHost: 10.0.0.1\r\n\r\n";
    give(&mut two, accepted, get, now, &mut generator);
    // Two short writes in a row: the second waits for the first to be
    // acknowledged, which is one exchange more.
    let now = exchange(&mut one, &mut two, now, &mut generator);
    let now = exchange(&mut one, &mut two, now, &mut generator);
    assert_eq!(taken(&mut one, client, now, &mut generator), get);

    let answer = b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\nhello, world\n";
    give(&mut one, client, answer, now, &mut generator);
    let _shut = one.server.answer(
        CLIENT,
        &Request::TcpShutdown {
            socket: client,
            direction: Direction::Write,
        },
        now,
        &mut generator,
    );
    let now = exchange(&mut one, &mut two, now, &mut generator);
    assert_eq!(
        taken(&mut two, accepted, now, &mut generator),
        answer,
        "the answer of the peer that closed right after it was lost"
    );
    assert_eq!(state(&mut two, accepted, now), State::PeerClosed);
}

#[test]
fn a_second_name_of_the_same_client_while_one_runs_is_refused() {
    let now = Instant::from_micros(0);
    let (mut one, now, mut generator) = leased(now);
    let first =
        user_proto::socket::Name::new(super::link::KNOWN_NAME.as_bytes()).expect("a short name");
    let second = user_proto::socket::Name::new(b"another.test").expect("a short name");
    let _started = one.server.answer(
        CLIENT,
        &Request::Resolve { name: first },
        now,
        &mut generator,
    );
    match one.server.answer(
        CLIENT,
        &Request::Resolve { name: second },
        now,
        &mut generator,
    ) {
        Reply::Resolved(Err(error)) => assert_eq!(
            error,
            Error::Busy,
            "the answer to one name was given for another"
        ),
        other => panic!("{other:?} took a second name"),
    }
}

#[test]
fn a_port_a_datagram_socket_holds_is_refused_and_its_buffer_stays_in_the_pool() {
    let now = Instant::from_micros(0);
    let mut one = host(ONE_MAC, ONE);
    let mut generator = rng(71);
    let _first = opened(one.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 9,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a datagram socket");
    match one.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 9,
            process: process(),
        },
        now,
        &mut generator,
    ) {
        Reply::Bound(Err(error)) => assert_eq!(error, Error::AddressInUse),
        other => panic!("{other:?} bound one port twice"),
    }
    // The refusal came before the pool handed anything over, so three
    // buffers are left and every one of them can still be taken.
    for port in 10..13 {
        let _bound = opened(one.server.answer(
            CLIENT,
            &Request::UdpBind {
                port,
                process: process(),
            },
            now,
            &mut generator,
        ))
        .expect("the pool kept its buffer");
    }
}

#[test]
fn a_port_a_listener_holds_is_refused_and_its_windows_stay_in_the_pool() {
    let now = Instant::from_micros(0);
    let mut two = host(TWO_MAC, TWO);
    let mut generator = rng(73);
    let _listener = opened(two.server.answer(
        CLIENT,
        &Request::TcpListen {
            port: PORT,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a listener");
    match two.server.answer(
        CLIENT,
        &Request::TcpListen {
            port: PORT,
            process: process(),
        },
        now,
        &mut generator,
    ) {
        Reply::Listening(Err(error)) => assert_eq!(error, Error::AddressInUse),
        other => panic!("{other:?} listened on one port twice"),
    }
    for port in [PORT + 1, PORT + 2, PORT + 3] {
        let _listening = opened(two.server.answer(
            CLIENT,
            &Request::TcpListen {
                port,
                process: process(),
            },
            now,
            &mut generator,
        ))
        .expect("the pool kept its windows");
    }
}

#[test]
fn what_a_client_wrote_before_its_connection_opened_waits_in_the_ring() {
    let now = Instant::from_micros(0);
    let mut one = host(ONE_MAC, ONE);
    let mut two = host(TWO_MAC, TWO);
    let mut generator = rng(79);
    let listener = opened(two.server.answer(
        CLIENT,
        &Request::TcpListen {
            port: PORT,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a listener");
    let client = opened(one.server.answer(
        CLIENT,
        &Request::TcpConnect {
            remote: Endpoint::new(IpAddr::V4(TWO), PORT),
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a connection");

    // The handshake has not run, so the connection takes nothing yet.
    let sent = b"a request written before the answer";
    assert_eq!(one.pages[0].outbound.write(sent), sent.len());
    match one.server.answer(
        CLIENT,
        &Request::TcpSend {
            socket: client,
            len: u32::try_from(sent.len()).unwrap_or(0),
        },
        now,
        &mut generator,
    ) {
        Reply::Sent(Ok(moved)) => assert_eq!(moved, 0, "a connection that is not open took bytes"),
        other => panic!("{other:?} sent nothing"),
    }
    assert_eq!(
        usize::try_from(one.pages[0].outbound.held()).unwrap_or(0),
        sent.len(),
        "the bytes left the ring and went nowhere"
    );

    let now = exchange(&mut one, &mut two, now, &mut generator);
    let accepted = opened(two.server.answer(
        CLIENT,
        &Request::TcpAccept { socket: listener },
        now,
        &mut generator,
    ))
    .expect("the listener took the connection");
    let now = exchange(&mut one, &mut two, now, &mut generator);
    assert_eq!(taken(&mut two, accepted, now, &mut generator), sent);
}

#[test]
fn a_datagram_longer_than_one_of_the_link_is_refused_and_the_ring_keeps_it() {
    let now = Instant::from_micros(0);
    let mut one = host(ONE_MAC, ONE);
    let mut generator = rng(83);
    let socket = opened(one.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 9,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a datagram socket");
    let payload = [7u8; 1473];
    assert_eq!(one.pages[0].outbound.write(&payload), payload.len());
    match one.server.answer(
        CLIENT,
        &Request::UdpSendTo {
            socket,
            remote: Endpoint::new(IpAddr::V4(TWO), 9),
            len: u32::try_from(payload.len()).unwrap_or(0),
        },
        now,
        &mut generator,
    ) {
        Reply::UdpSent(Err(error)) => assert_eq!(error, Error::BufferTooSmall),
        other => panic!("{other:?} sent a datagram that does not fit one"),
    }
    assert_eq!(
        usize::try_from(one.pages[0].outbound.held()).unwrap_or(0),
        payload.len(),
        "part of the datagram left the ring and the rest stayed"
    );
}

#[test]
fn a_datagram_of_more_than_one_chunk_arrives_whole() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, _client, _accepted, now) = connected(now);
    let mut generator = rng(89);
    let _taking = opened(one.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 9,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a datagram socket");
    let sender = opened(two.server.answer(
        CLIENT,
        &Request::UdpBind {
            port: 0,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a datagram socket");
    let payload: Vec<u8> = (0..1200u32)
        .map(|byte| u8::try_from(byte % 251).unwrap_or(0))
        .collect();
    assert_eq!(two.pages[1].outbound.write(&payload), payload.len());
    let sent = two.server.answer(
        CLIENT,
        &Request::UdpSendTo {
            socket: sender,
            remote: Endpoint::new(IpAddr::V4(ONE), 9),
            len: u32::try_from(payload.len()).unwrap_or(0),
        },
        now,
        &mut generator,
    );
    let _reached = exchange(&mut one, &mut two, now, &mut generator);
    let mut header = [0u8; user_proto::socket::DATAGRAM_HEADER_LEN];
    assert_eq!(
        one.pages[1].inbound.read(&mut header),
        user_proto::socket::DATAGRAM_HEADER_LEN,
        "no record arrived, and the send answered {sent:?}"
    );
    let (_from, len) =
        user_proto::socket::read_datagram_header(&header).expect("a record of a datagram");
    assert_eq!(len, payload.len(), "the record says a shorter datagram");
    let mut body = vec![0u8; len];
    assert_eq!(one.pages[1].inbound.read(&mut body), len);
    assert_eq!(body, payload, "the datagram was torn");
}

#[test]
fn what_the_client_wrote_goes_before_the_shutdown_it_asked_for() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, client, accepted, now) = connected(now);
    let mut generator = rng(97);
    let last = b"the last of what the client had to say";
    assert_eq!(one.pages[0].outbound.write(last), last.len());
    match one.server.answer(
        CLIENT,
        &Request::TcpShutdown {
            socket: client,
            direction: Direction::Write,
        },
        now,
        &mut generator,
    ) {
        Reply::ShutDown(Ok(())) => {}
        other => panic!("{other:?} closed nothing"),
    }
    let now = exchange(&mut one, &mut two, now, &mut generator);
    assert_eq!(
        taken(&mut two, accepted, now, &mut generator),
        last,
        "the bytes of the ring went nowhere when the client closed"
    );
    assert_eq!(state(&mut two, accepted, now), State::PeerClosed);
}

#[test]
fn a_request_under_no_badge_is_refused_and_opens_nothing() {
    let now = Instant::from_micros(0);
    let mut one = host(ONE_MAC, ONE);
    let mut generator = rng(101);
    match one
        .server
        .answer(NOBODY, &Request::Interface, now, &mut generator)
    {
        Reply::Interface(Err(error)) => assert_eq!(error, Error::AccessDenied),
        other => panic!("{other:?} answered nobody"),
    }
    match one.server.answer(
        NOBODY,
        &Request::UdpBind {
            port: 9,
            process: process(),
        },
        now,
        &mut generator,
    ) {
        Reply::Bound(Err(error)) => assert_eq!(error, Error::AccessDenied),
        other => panic!("{other:?} opened a socket for nobody"),
    }
    assert_eq!(one.server.open(), 0);
}

#[test]
fn a_second_connect_to_the_same_remote_keeps_the_window_pair() {
    let now = Instant::from_micros(0);
    let (mut one, _two, _client, _accepted, now) = connected(now);
    let mut generator = rng(103);
    for _ in 0..4 {
        match one.server.answer(
            CLIENT,
            &Request::TcpConnect {
                remote: Endpoint::new(IpAddr::V4(TWO), PORT),
                process: process(),
            },
            now,
            &mut generator,
        ) {
            Reply::Connected(Err(error)) => assert_eq!(error, Error::AddressInUse),
            other => panic!("{other:?} joined two connections to one pair of ends"),
        }
    }
    // One pair is in use; the other three are still in the pool.
    for port in [8, 9, 10] {
        let _listener = opened(one.server.answer(
            CLIENT,
            &Request::TcpListen {
                port,
                process: process(),
            },
            now,
            &mut generator,
        ))
        .expect("a window pair is left");
    }
}

#[test]
fn the_slot_of_a_client_that_is_gone_is_offered_to_the_next() {
    let now = Instant::from_micros(0);
    let mut one = host(ONE_MAC, ONE);
    let mut generator = rng(107);
    let bind = |host: &mut Host, badge: u64, generator: &mut super::support::Rng| {
        host.server.answer(
            badge,
            &Request::UdpBind {
                port: 0,
                process: process(),
            },
            now,
            generator,
        )
    };
    for badge in 1..=4 {
        let socket = opened(bind(&mut one, badge, &mut generator)).expect("a free slot");
        let _closed = one
            .server
            .answer(badge, &Request::UdpClose { socket }, now, &mut generator);
    }
    assert_eq!(
        opened(bind(&mut one, 5, &mut generator)),
        Err(Error::OutOfHandles)
    );
    assert!(one.server.holds(1));
    one.server.forget(1, now);
    assert!(!one.server.holds(1));
    let _socket = opened(bind(&mut one, 5, &mut generator)).expect("the slot of the first");
    assert!(one.server.holds(5));
}

#[test]
fn a_resolution_nobody_collects_gives_way_to_the_next_client() {
    let now = Instant::from_micros(0);
    let (mut one, now, mut generator) = leased(now);
    let name =
        user_proto::socket::Name::new(super::link::KNOWN_NAME.as_bytes()).expect("a short name");
    let _started = one
        .server
        .answer(CLIENT, &Request::Resolve { name }, now, &mut generator);
    let mut now = now;
    for _ in 0..32 {
        if matches!(
            one.server.stack().resolution(),
            Some(net_dns::Status::Done | net_dns::Status::Failed(_))
        ) {
            break;
        }
        now = against_the_link(&mut one, now, &mut generator);
    }
    assert!(
        matches!(one.server.stack().resolution(), Some(net_dns::Status::Done)),
        "the resolution never ended"
    );
    // The first client never asks again; the second starts its own.
    match one
        .server
        .answer(CLIENT + 1, &Request::Resolve { name }, now, &mut generator)
    {
        Reply::Resolved(Err(error)) => assert_eq!(error, Error::WouldBlock),
        other => panic!("{other:?} did not start a resolution"),
    }
}

#[test]
fn a_shutdown_waits_until_every_byte_of_the_ring_is_in_the_connection() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, client, accepted, now) = connected(now);
    let mut generator = rng(109);
    let payload: Vec<u8> = (0..2000u32)
        .map(|byte| u8::try_from(byte % 251).unwrap_or(0))
        .collect();
    assert_eq!(one.pages[0].outbound.write(&payload), payload.len());
    let mut now = now;
    let mut received = Vec::new();
    let mut into = [0u8; 4096];
    let mut shut = false;
    for _ in 0..16 {
        match one.server.answer(
            CLIENT,
            &Request::TcpShutdown {
                socket: client,
                direction: Direction::Write,
            },
            now,
            &mut generator,
        ) {
            Reply::ShutDown(Ok(())) => shut = true,
            Reply::ShutDown(Err(Error::WouldBlock)) => {}
            other => panic!("{other:?} is no answer to a shutdown"),
        }
        now = exchange(&mut one, &mut two, now, &mut generator);
        let len = two.pages[0].inbound.read(&mut into);
        received.extend_from_slice(&into[..len]);
        if shut {
            break;
        }
    }
    assert!(shut, "the shutdown never went through");
    now = exchange(&mut one, &mut two, now, &mut generator);
    let len = two.pages[0].inbound.read(&mut into);
    received.extend_from_slice(&into[..len]);
    assert_eq!(received, payload, "bytes of the ring were lost");
    assert_eq!(state(&mut two, accepted, now), State::PeerClosed);
}

#[test]
fn a_close_of_an_open_connection_resets_the_peer() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, client, accepted, now) = connected(now);
    let mut generator = rng(113);
    match one.server.answer(
        CLIENT,
        &Request::TcpClose { socket: client },
        now,
        &mut generator,
    ) {
        Reply::Closed(Ok(())) => {}
        other => panic!("{other:?} closed nothing"),
    }
    let now = exchange(&mut one, &mut two, now, &mut generator);
    assert_eq!(state(&mut two, accepted, now), State::Refused);
}

#[test]
fn a_receive_answers_the_bytes_it_moved_not_the_bytes_waiting() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, client, accepted, now) = connected(now);
    let mut generator = rng(127);
    give(&mut two, accepted, &[5u8; 500], now, &mut generator);
    let now = exchange(&mut one, &mut two, now, &mut generator);
    assert_eq!(one.pages[0].inbound.held(), 500);
    match one.server.answer(
        CLIENT,
        &Request::TcpRecv { socket: client },
        now,
        &mut generator,
    ) {
        Reply::Received(Ok(moved)) => assert_eq!(moved, 0),
        other => panic!("{other:?} is no count"),
    }
}

#[test]
fn a_send_moves_no_more_than_the_client_asked() {
    let now = Instant::from_micros(0);
    let (mut one, _two, client, _accepted, now) = connected(now);
    let mut generator = rng(131);
    assert_eq!(one.pages[0].outbound.write(&[3u8; 300]), 300);
    match one.server.answer(
        CLIENT,
        &Request::TcpSend {
            socket: client,
            len: 100,
        },
        now,
        &mut generator,
    ) {
        Reply::Sent(Ok(moved)) => assert_eq!(moved, 100),
        other => panic!("{other:?} is no count"),
    }
    assert_eq!(one.pages[0].outbound.held(), 200);
}

#[test]
fn a_close_right_after_a_shutdown_drains_before_it_resets() {
    let now = Instant::from_micros(0);
    let (mut one, mut two, client, _accepted, now) = connected(now);
    let mut generator = rng(137);
    let last = b"the tail the peer has not acknowledged yet";
    assert_eq!(one.pages[0].outbound.write(last), last.len());
    for request in [
        Request::TcpShutdown {
            socket: client,
            direction: Direction::Write,
        },
        Request::TcpClose { socket: client },
    ] {
        match one.server.answer(CLIENT, &request, now, &mut generator) {
            Reply::ShutDown(Ok(())) | Reply::Closed(Ok(())) => {}
            other => panic!("{other:?} for {request:?}"),
        }
    }
    // The number names nothing from the close on.
    match one.server.answer(
        CLIENT,
        &Request::TcpState { socket: client },
        now,
        &mut generator,
    ) {
        Reply::State(Err(error)) => assert_eq!(error, Error::NotFound),
        other => panic!("{other:?} answered for a closed socket"),
    }
    let _reached = exchange(&mut one, &mut two, now, &mut generator);
    assert_eq!(one.server.open(), 0, "the drained connection stayed");
    let mut into = [0u8; 64];
    let len = two.pages[0].inbound.read(&mut into);
    assert_eq!(&into[..len], last, "the tail was lost to the reset");
}

#[test]
fn another_client_that_is_gone_leaves_the_resolution_running() {
    let now = Instant::from_micros(0);
    let (mut one, now, mut generator) = leased(now);
    let name =
        user_proto::socket::Name::new(super::link::KNOWN_NAME.as_bytes()).expect("a short name");
    let _started = one
        .server
        .answer(CLIENT, &Request::Resolve { name }, now, &mut generator);
    one.server.forget(CLIENT + 1, now);
    match one
        .server
        .answer(CLIENT + 1, &Request::Resolve { name }, now, &mut generator)
    {
        Reply::Resolved(Err(error)) => assert_eq!(error, Error::Busy),
        other => panic!("{other:?} ended the resolution of another client"),
    }
}

#[test]
fn a_second_shutdown_of_a_closing_connection_answers_at_once() {
    let now = Instant::from_micros(0);
    let (mut one, _two, client, _accepted, now) = connected(now);
    let mut generator = rng(139);
    let shutdown = Request::TcpShutdown {
        socket: client,
        direction: Direction::Write,
    };
    for _ in 0..2 {
        match one.server.answer(CLIENT, &shutdown, now, &mut generator) {
            Reply::ShutDown(Ok(())) => {}
            other => panic!("{other:?} for a shutdown"),
        }
    }
    // A byte written after the shutdown stays in the ring.
    assert_eq!(one.pages[0].outbound.write(b"x"), 1);
    match one.server.answer(CLIENT, &shutdown, now, &mut generator) {
        Reply::ShutDown(Ok(())) => {}
        other => panic!("{other:?} for a shutdown of a closing connection"),
    }
    assert_eq!(one.pages[0].outbound.held(), 1);
}

#[test]
fn a_listener_that_is_closed_gives_its_slot_back_at_once() {
    let now = Instant::from_micros(0);
    let mut two = host(TWO_MAC, TWO);
    let mut generator = rng(149);
    let listener = opened(two.server.answer(
        CLIENT,
        &Request::TcpListen {
            port: PORT,
            process: process(),
        },
        now,
        &mut generator,
    ))
    .expect("a listener");
    match two.server.answer(
        CLIENT,
        &Request::TcpClose { socket: listener },
        now,
        &mut generator,
    ) {
        Reply::Closed(Ok(())) => {}
        other => panic!("{other:?} closed nothing"),
    }
    assert_eq!(two.server.open(), 0);
}
