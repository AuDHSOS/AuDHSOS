// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A host on the link that is not a server of this crate: it hands out one
//! address lease and answers one name.
//!
//! What it stands for is the built-in server of QEMU's user-mode network,
//! which is what the machine of 3.1.1 meets. It answers a discover with an
//! offer and a request with an acknowledgment, and a query for one name
//! with one address; everything else it drops.

#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a test reads a frame at known offsets"
)]

use net_dhcp::{Message, MessageType, Op, OptionCode, write_option};
use net_dns::{Name, RecordType};
use net_eth::Frame;

use net_udp::{ChecksumPolicy, Datagram};
use net_wire::{EtherType, IpAddr, Ipv4Addr, MacAddr, Port, Protocol, Writer};

/// The hardware address of the host that answers.
pub(crate) const SERVER_MAC: MacAddr = MacAddr::new([0x52, 0x54, 0x00, 0x00, 0x00, 0x02]);

/// Its address, which is also the router and the name server of the lease.
pub(crate) const SERVER: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 2);

/// The address it hands out.
pub(crate) const LEASED: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 15);

/// The name it answers, and the address it answers with.
pub(crate) const KNOWN_NAME: &str = "example.test";

/// A name it knows and has no address for, which is an answer and not a
/// failure.
pub(crate) const EMPTY_NAME: &str = "empty.test";

/// That address.
pub(crate) const KNOWN_ADDRESS: Ipv4Addr = Ipv4Addr::new(93, 184, 216, 34);

/// How long a lease lasts, in seconds.
const LEASE_SECONDS: u32 = 600;

/// The prefix of the link, as the subnet mask names it.
const MASK: [u8; 4] = [255, 255, 255, 0];

/// The answer to `frame`, or nothing when this host has none.
pub(crate) fn answer(frame: &[u8], into: &mut [u8]) -> Option<usize> {
    let parsed = Frame::parse(frame).ok()?;
    if parsed.ether_type() == EtherType::ARP {
        return arp_reply(parsed.payload(), into);
    }
    if parsed.ether_type() != EtherType::IPV4 {
        return None;
    }
    let inner = net_ip::Datagram::parse(parsed.payload()).ok()?;
    if inner.protocol() != Protocol::UDP {
        return None;
    }
    let datagram = Datagram::parse(
        inner.payload(),
        IpAddr::V4(inner.source()),
        IpAddr::V4(inner.destination()),
    )
    .ok()?;
    let mut payload = [0u8; 512];
    let (len, from, to) = match datagram.destination_port.get() {
        67 => (
            dhcp_reply(datagram.payload, &mut payload)?,
            Port::new(67),
            Port::new(68),
        ),
        53 => (
            dns_reply(datagram.payload, &mut payload)?,
            Port::new(53),
            datagram.source_port,
        ),
        _other => return None,
    };
    let answered = payload.get(..len)?;
    let destination = if datagram.destination_port.get() == 67 {
        Ipv4Addr::BROADCAST
    } else {
        inner.source()
    };
    let hardware = if destination == Ipv4Addr::BROADCAST {
        MacAddr::BROADCAST
    } else {
        parsed.source()
    };
    write_frame(into, hardware, SERVER, destination, from, to, answered)
}

/// The answer to one ARP packet, or nothing for one that asks after
/// another station.
fn arp_reply(payload: &[u8], into: &mut [u8]) -> Option<usize> {
    let packet = net_eth::arp::Packet::parse(payload).ok()?;
    let reply = net_eth::arp::respond(&packet, SERVER_MAC, SERVER)?;
    let mut bytes = [0u8; net_eth::arp::PACKET_LEN];
    let mut writer = Writer::new(&mut bytes);
    reply.write(&mut writer).ok()?;
    let len = writer.position();
    let mut writer = Writer::new(into);
    Frame::write(
        &mut writer,
        packet.sender_hardware,
        SERVER_MAC,
        EtherType::ARP,
        bytes.get(..len)?,
    )
    .ok()?;
    Some(writer.position())
}

/// Writes one frame carrying one datagram, and answers how long it is.
fn write_frame(
    into: &mut [u8],
    hardware: MacAddr,
    source: Ipv4Addr,
    destination: Ipv4Addr,
    from: Port,
    to: Port,
    payload: &[u8],
) -> Option<usize> {
    let mut datagram = [0u8; 640];
    let mut writer = Writer::new(&mut datagram);
    Datagram {
        source_port: from,
        destination_port: to,
        payload,
    }
    .write(
        &mut writer,
        IpAddr::V4(source),
        IpAddr::V4(destination),
        ChecksumPolicy::Computed,
    )
    .ok()?;
    let udp_len = writer.position();

    let mut inner = [0u8; 704];
    let mut writer = Writer::new(&mut inner);
    net_ip::Header::new(source, destination, Protocol::UDP, udp_len)
        .write(&mut writer)
        .ok()?;
    writer.write_bytes(datagram.get(..udp_len)?).ok()?;
    let ip_len = writer.position();

    let mut writer = Writer::new(into);
    Frame::write(
        &mut writer,
        hardware,
        SERVER_MAC,
        EtherType::IPV4,
        inner.get(..ip_len)?,
    )
    .ok()?;
    Some(writer.position())
}

/// The reply to one DHCP message, or nothing for one this host ignores.
fn dhcp_reply(payload: &[u8], into: &mut [u8]) -> Option<usize> {
    let message = Message::parse(payload).ok()?;
    if message.op != Op::REQUEST {
        return None;
    }
    let kind = message.message_type().ok()?;
    let answer = match kind {
        MessageType::DISCOVER => MessageType::OFFER,
        MessageType::REQUEST => MessageType::ACK,
        _other => return None,
    };
    let mut options = [0u8; 64];
    let len = {
        let mut writer = Writer::new(&mut options);
        write_option(&mut writer, OptionCode::MESSAGE_TYPE, &[answer.get()]).ok()?;
        write_option(&mut writer, OptionCode::SERVER_IDENTIFIER, &SERVER.octets()).ok()?;
        write_option(&mut writer, OptionCode::SUBNET_MASK, &MASK).ok()?;
        write_option(&mut writer, OptionCode::ROUTER, &SERVER.octets()).ok()?;
        write_option(
            &mut writer,
            OptionCode::DOMAIN_NAME_SERVER,
            &SERVER.octets(),
        )
        .ok()?;
        write_option(
            &mut writer,
            OptionCode::LEASE_TIME,
            &LEASE_SECONDS.to_be_bytes(),
        )
        .ok()?;
        writer.write_u8(OptionCode::END.get()).ok()?;
        writer.position()
    };
    let mut reply = Message::request(message.xid, message.hardware);
    reply.op = Op::REPLY;
    reply.yours = LEASED;
    reply.options = options.get(..len)?;
    let mut writer = Writer::new(into);
    reply.write(&mut writer).ok()?;
    Some(writer.position())
}

/// The reply to one DNS query, or nothing for a name this host does not
/// know.
fn dns_reply(payload: &[u8], into: &mut [u8]) -> Option<usize> {
    let message = net_dns::Message::parse(payload).ok()?;
    let question = message.question().ok()??;
    let known = Name::from_ascii(KNOWN_NAME).ok()?;
    let empty = Name::from_ascii(EMPTY_NAME).ok()?;
    let carries = if question.name == known {
        true
    } else if question.name == empty {
        false
    } else {
        return None;
    };
    let mut writer = Writer::new(into);
    let answers = u16::from(carries && question.record_type == RecordType::A);
    net_dns::Header {
        id: message.header.id,
        flags: message.header.flags.as_response(),
        questions: 1,
        answers,
        authorities: 0,
        additionals: 0,
    }
    .write(&mut writer)
    .ok()?;
    question.write(&mut writer).ok()?;
    if answers == 1 {
        question.name.write(&mut writer).ok()?;
        writer.write_u16(RecordType::A.get()).ok()?;
        writer.write_u16(1).ok()?;
        writer.write_u32(300).ok()?;
        writer.write_u16(4).ok()?;
        writer.write_bytes(&KNOWN_ADDRESS.octets()).ok()?;
    }
    Some(writer.position())
}
