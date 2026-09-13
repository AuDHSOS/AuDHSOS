// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::socket`.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut, SIZE};
use audhsos_abi::{Error, Handle};
use net_wire::{IpAddr, Ipv4Addr, Ipv6Addr, MacAddr};

use crate::label::{Label, ProtoError, Protocol};
use crate::socket::{
    Addresses, DATAGRAM_HEADER_LEN, Direction, Endpoint, INTERFACE, Interface, MAX_ADDRESSES, Name,
    Opened, Reply, Request, State, TCP_STATE, datagram_header, read_datagram_header,
};

/// A buffer of zeros to work on.
fn buffer() -> [u8; SIZE] {
    [0; SIZE]
}

/// Encodes `request` and reads it back.
fn round_trip(request: &Request) -> Result<Request, ProtoError> {
    let mut bytes = buffer();
    request.encode(&mut BufferMut::new(&mut bytes))?;
    Request::decode(Buffer::new(&bytes))
}

/// Encodes `reply` and reads it back.
fn round_trip_reply(reply: &Reply) -> Result<Reply, ProtoError> {
    let mut bytes = buffer();
    reply.encode(&mut BufferMut::new(&mut bytes))?;
    Reply::decode(Buffer::new(&bytes))
}

/// A handle a reply carries.
fn handle() -> Handle {
    Handle::new(3, 1).expect("a handle")
}

/// The endpoint the tests connect to.
fn endpoint() -> Endpoint {
    Endpoint::new(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 2)), 7)
}

/// One address of each family.
fn addresses() -> Addresses {
    let mut list = Addresses::new();
    assert!(list.push(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 15))));
    assert!(list.push(IpAddr::V6(Ipv6Addr::from_octets([
        0x20, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1
    ]))));
    list
}

/// Every request of the protocol.
fn requests() -> Vec<Request> {
    vec![
        Request::Interface,
        Request::Resolve {
            name: Name::new(b"example.test").expect("a short name"),
        },
        Request::UdpBind { port: 5353 },
        Request::UdpSendTo {
            socket: 2,
            remote: endpoint(),
            len: 64,
        },
        Request::UdpClose { socket: 2 },
        Request::TcpConnect {
            remote: Endpoint::new(
                IpAddr::V6(Ipv6Addr::from_octets([
                    0x20, 0x01, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2,
                ])),
                443,
            ),
        },
        Request::TcpListen { port: 7 },
        Request::TcpAccept { socket: 1 },
        Request::TcpSend {
            socket: 1,
            len: 1024,
        },
        Request::TcpRecv { socket: 1 },
        Request::TcpShutdown {
            socket: 1,
            direction: Direction::Write,
        },
        Request::TcpClose { socket: 1 },
        Request::TcpState { socket: 1 },
    ]
}

/// Every reply of the protocol.
fn replies() -> Vec<Reply> {
    vec![
        Reply::Interface(Ok(Interface {
            mac: MacAddr::new([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]),
            addresses: addresses(),
            gateway: Some(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 2))),
            lease: true,
        })),
        Reply::Interface(Err(Error::Unavailable)),
        Reply::Resolved(Ok(addresses())),
        Reply::Resolved(Err(Error::NotFound)),
        Reply::Bound(Ok(Opened {
            socket: 1,
            rings: handle(),
        })),
        Reply::Bound(Err(Error::OutOfMemory)),
        Reply::UdpSent(Ok(64)),
        Reply::UdpSent(Err(Error::Unavailable)),
        Reply::UdpClosed(Ok(())),
        Reply::UdpClosed(Err(Error::NotFound)),
        Reply::Connected(Ok(Opened {
            socket: 2,
            rings: handle(),
        })),
        Reply::Connected(Err(Error::Unavailable)),
        Reply::Listening(Ok(3)),
        Reply::Listening(Err(Error::AddressInUse)),
        Reply::Accepted(Ok(Opened {
            socket: 4,
            rings: handle(),
        })),
        Reply::Accepted(Err(Error::WouldBlock)),
        Reply::Sent(Ok(17)),
        Reply::Sent(Err(Error::InvalidState)),
        Reply::Received(Ok(0)),
        Reply::Received(Err(Error::WrongObjectType)),
        Reply::ShutDown(Ok(())),
        Reply::ShutDown(Err(Error::InvalidState)),
        Reply::Closed(Ok(())),
        Reply::Closed(Err(Error::NotFound)),
        Reply::State(Ok(State::Established)),
        Reply::State(Err(Error::NotFound)),
    ]
}

#[test]
fn every_request_comes_back_as_it_went_out() {
    for request in requests() {
        assert_eq!(round_trip(&request).unwrap(), request, "{request:?}");
    }
}

#[test]
fn every_reply_comes_back_as_it_went_out() {
    for reply in replies() {
        assert_eq!(round_trip_reply(&reply).unwrap(), reply, "{reply:?}");
    }
}

#[test]
fn every_request_carries_its_own_message_number() {
    let mut seen = Vec::new();
    for request in requests() {
        let label = request.label();
        assert_eq!(label.protocol, Protocol::Socket);
        assert!(
            !seen.contains(&label.message),
            "{request:?} shares a number"
        );
        seen.push(label.message);
    }
}

#[test]
fn a_reply_carries_the_number_of_the_request_it_answers() {
    for (request, reply) in requests().into_iter().zip([
        Reply::Interface(Err(Error::Unavailable)),
        Reply::Resolved(Err(Error::NotFound)),
        Reply::Bound(Err(Error::OutOfMemory)),
        Reply::UdpSent(Err(Error::Unavailable)),
        Reply::UdpClosed(Ok(())),
        Reply::Connected(Err(Error::Unavailable)),
        Reply::Listening(Err(Error::AddressInUse)),
        Reply::Accepted(Err(Error::WouldBlock)),
        Reply::Sent(Err(Error::InvalidState)),
        Reply::Received(Err(Error::WrongObjectType)),
        Reply::ShutDown(Ok(())),
        Reply::Closed(Ok(())),
        Reply::State(Err(Error::NotFound)),
    ]) {
        assert_eq!(
            request.label().message,
            reply.label().message,
            "{request:?}"
        );
    }
}

#[test]
fn a_message_of_another_protocol_is_refused() {
    let mut bytes = buffer();
    let mut out = BufferMut::new(&mut bytes);
    out.set_label(Label::new(Protocol::File, INTERFACE).raw());
    out.set_counts(0, 0).expect("a header");
    assert_eq!(
        Request::decode(Buffer::new(&bytes)),
        Err(ProtoError::WrongProtocol {
            expected: Protocol::Socket,
            found: Protocol::File,
        })
    );
}

#[test]
fn a_message_number_this_protocol_does_not_have_is_refused() {
    let mut bytes = buffer();
    let mut out = BufferMut::new(&mut bytes);
    out.set_label(Label::new(Protocol::Socket, 99).raw());
    out.set_counts(0, 0).expect("a header");
    assert_eq!(
        Request::decode(Buffer::new(&bytes)),
        Err(ProtoError::Message(Protocol::Socket, 99))
    );
}

#[test]
fn a_truncated_request_is_refused_rather_than_read_short() {
    let mut bytes = buffer();
    let mut out = BufferMut::new(&mut bytes);
    out.set_label(Label::new(Protocol::Socket, TCP_STATE).raw());
    out.set_counts(0, 0).expect("a header");
    assert!(Request::decode(Buffer::new(&bytes)).is_err());
}

#[test]
fn an_address_family_the_message_does_not_name_is_refused() {
    let mut bytes = buffer();
    Request::TcpConnect { remote: endpoint() }
        .encode(&mut BufferMut::new(&mut bytes))
        .expect("the request fits");
    // The second word of the message is the family; nine names none.
    let mut out = BufferMut::new(&mut bytes);
    assert!(out.set_word(1, 9));
    assert!(Request::decode(Buffer::new(&bytes)).is_err());
}

#[test]
fn a_shutdown_of_a_direction_the_protocol_does_not_have_is_refused() {
    let mut bytes = buffer();
    Request::TcpShutdown {
        socket: 1,
        direction: Direction::Both,
    }
    .encode(&mut BufferMut::new(&mut bytes))
    .expect("the request fits");
    let mut out = BufferMut::new(&mut bytes);
    assert!(out.set_word(1, 9));
    assert!(Request::decode(Buffer::new(&bytes)).is_err());
}

#[test]
fn a_list_of_more_addresses_than_the_protocol_carries_is_refused() {
    let mut bytes = buffer();
    Reply::Resolved(Ok(addresses()))
        .encode(&mut BufferMut::new(&mut bytes))
        .expect("the reply fits");
    let mut out = BufferMut::new(&mut bytes);
    let too_many = u64::try_from(MAX_ADDRESSES).unwrap_or(0) + 1;
    assert!(out.set_word(1, too_many));
    assert!(Reply::decode(Buffer::new(&bytes)).is_err());
}

#[test]
fn every_direction_and_every_state_is_its_own_code() {
    for direction in [Direction::Write, Direction::Read, Direction::Both] {
        assert_eq!(Direction::from_code(direction.code()), Some(direction));
    }
    assert_eq!(Direction::from_code(0), None);
    for state in [
        State::Connecting,
        State::Established,
        State::PeerClosed,
        State::Closed,
        State::Refused,
    ] {
        assert_eq!(State::from_code(state.code()), Some(state));
        assert!(!state.name().is_empty());
    }
    assert_eq!(State::from_code(0), None);
}

#[test]
fn a_list_of_addresses_holds_as_many_as_it_says_and_no_more() {
    let mut list = Addresses::new();
    assert!(list.is_empty());
    for step in 0..MAX_ADDRESSES {
        let octet = u8::try_from(step).unwrap_or(0);
        assert!(list.push(IpAddr::V4(Ipv4Addr::new(10, 0, 0, octet))));
    }
    assert_eq!(list.len(), MAX_ADDRESSES);
    assert!(!list.push(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 99))));
}

#[test]
fn a_datagram_record_carries_the_address_of_either_family() {
    for address in [
        IpAddr::V4(Ipv4Addr::new(10, 0, 2, 3)),
        IpAddr::V6(Ipv6Addr::from_octets([9; 16])),
    ] {
        let from = Endpoint::new(address, 53);
        let header = datagram_header(from, 1234);
        assert_eq!(header.len(), DATAGRAM_HEADER_LEN);
        assert_eq!(read_datagram_header(&header), Some((from, 1234)));
    }
}
