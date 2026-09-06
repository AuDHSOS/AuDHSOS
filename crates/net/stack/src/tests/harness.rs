// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Frames for the tests to feed in, and a way to read the ones that come
//! out.
//!
//! Everything here is built by project code out of the crates under test,
//! as D-40 requires: nothing was captured from a device.

#![allow(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "a test builds a frame at known offsets"
)]

use net_wire::{
    EtherType, IpAddr, Ipv4Addr, Ipv6Addr, MacAddr, Port, Protocol, Writer, transport_v6,
};

/// This host.
pub(crate) const MAC: MacAddr = MacAddr::new([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);

/// The other station on the link.
pub(crate) const PEER_MAC: MacAddr = MacAddr::new([0x52, 0x54, 0x00, 0xAB, 0xCD, 0xEF]);

/// This host's address, once it has one.
pub(crate) const HERE: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 50);

/// The other station's.
pub(crate) const PEER: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 20);

/// The router and the DHCP server, which are the same box.
pub(crate) const ROUTER: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 1);

/// The mask of the link.
pub(crate) const MASK: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);

/// Where the payload of a frame begins.
pub(crate) const PAYLOAD_AT: usize = 14;

/// This host, over IPv6, once a prefix has been advertised.
pub(crate) const HERE6: Ipv6Addr = Ipv6Addr::from_octets([
    0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0x50, 0x54, 0x00, 0xff, 0xfe, 0x12, 0x34, 0x56,
]);

/// The other station, over IPv6.
pub(crate) const PEER6: Ipv6Addr = Ipv6Addr::from_octets([
    0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x20,
]);

/// The router's link-local address, which is where an advertisement comes
/// from.
pub(crate) const ROUTER6: Ipv6Addr = Ipv6Addr::from_octets([
    0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0x02, 0x1b, 0x44, 0xff, 0xfe, 0x11, 0x3a, 0x01,
]);

/// The prefix the router advertises.
pub(crate) const PREFIX: Ipv6Addr =
    Ipv6Addr::from_octets([0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

/// The all-nodes group of the link.
pub(crate) const ALL_NODES: Ipv6Addr =
    Ipv6Addr::from_octets([0xFF, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

/// The hop limit every Neighbor Discovery message carries.
pub(crate) const DISCOVERY_HOP_LIMIT: u8 = 255;

/// One frame, built around `payload`.
pub(crate) fn frame(
    destination: MacAddr,
    source: MacAddr,
    ether_type: EtherType,
    payload: &[u8],
) -> Vec<u8> {
    let mut bytes = vec![0u8; 1600];
    let mut writer = Writer::new(&mut bytes);
    net_eth::Frame::write(&mut writer, destination, source, ether_type, payload).expect("room");
    writer.finish().to_vec()
}

/// An ARP request from the peer, asking who holds `target`.
pub(crate) fn arp_request(target: Ipv4Addr) -> Vec<u8> {
    arp_request_from(PEER_MAC, PEER, target)
}

/// An ARP request from `sender`, asking who holds `target`.
pub(crate) fn arp_request_from(hardware: MacAddr, sender: Ipv4Addr, target: Ipv4Addr) -> Vec<u8> {
    let packet = net_eth::Packet::request(hardware, sender, target);
    let mut payload = [0u8; 64];
    let mut writer = Writer::new(&mut payload);
    packet.write(&mut writer).expect("room");
    let len = writer.position();
    frame(
        MacAddr::BROADCAST,
        hardware,
        EtherType::ARP,
        &payload[..len],
    )
}

/// A frame carrying a UDP datagram from `source` at `source_port`.
pub(crate) fn udp_from(
    source: Ipv4Addr,
    destination: Ipv4Addr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut bytes = vec![0u8; 1600];
    let mut writer = Writer::new(&mut bytes);
    net_udp::Datagram {
        source_port: Port::new(source_port),
        destination_port: Port::new(destination_port),
        payload,
    }
    .write(
        &mut writer,
        IpAddr::V4(source),
        IpAddr::V4(destination),
        net_udp::ChecksumPolicy::Computed,
    )
    .expect("room");
    let len = writer.position();
    let datagram = ipv4(source, destination, Protocol::UDP, &bytes[..len]);
    frame(MAC, PEER_MAC, EtherType::IPV4, &datagram)
}

/// An IPv4 datagram from `source` to `destination`, carrying `payload`.
pub(crate) fn ipv4(
    source: Ipv4Addr,
    destination: Ipv4Addr,
    protocol: Protocol,
    payload: &[u8],
) -> Vec<u8> {
    let header = net_ip::Header::new(source, destination, protocol, payload.len());
    let mut bytes = vec![0u8; 1600];
    let mut writer = Writer::new(&mut bytes);
    header.write(&mut writer).expect("room");
    writer.write_bytes(payload).expect("room");
    writer.finish().to_vec()
}

/// A frame carrying an IPv4 datagram to this host.
pub(crate) fn ipv4_frame(
    destination_mac: MacAddr,
    source: Ipv4Addr,
    destination: Ipv4Addr,
    protocol: Protocol,
    payload: &[u8],
) -> Vec<u8> {
    let datagram = ipv4(source, destination, protocol, payload);
    frame(destination_mac, PEER_MAC, EtherType::IPV4, &datagram)
}

/// A UDP datagram from the peer to this host.
pub(crate) fn udp_frame(
    destination: Ipv4Addr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    udp_from(PEER, destination, source_port, destination_port, payload)
}

/// A bare `SYN` from the peer to this host.
pub(crate) fn syn_frame(destination: Ipv4Addr, source_port: u16, destination_port: u16) -> Vec<u8> {
    let mut segment = net_tcp::Segment::new(
        Port::new(source_port),
        Port::new(destination_port),
        net_tcp::Flags::SYN,
    );
    segment.seq = net_tcp::SeqNumber::new(1000);
    segment.window = 4096;
    let mut bytes = vec![0u8; 1600];
    let mut writer = Writer::new(&mut bytes);
    segment
        .write(&mut writer, IpAddr::V4(PEER), IpAddr::V4(destination))
        .expect("room");
    let len = writer.position();
    ipv4_frame(MAC, PEER, destination, Protocol::TCP, &bytes[..len])
}

/// An `ICMPv4` echo request from the peer to this host.
pub(crate) fn echo_frame(destination: Ipv4Addr) -> Vec<u8> {
    let message = net_ip::Message::EchoRequest {
        identifier: 7,
        sequence: 1,
        payload: b"ping",
    };
    let mut bytes = vec![0u8; 128];
    let mut writer = Writer::new(&mut bytes);
    message.write(&mut writer).expect("room");
    let len = writer.position();
    ipv4_frame(MAC, PEER, destination, Protocol::ICMP, &bytes[..len])
}

/// The Ethernet type of a frame.
pub(crate) fn ether_type_of(frame: &[u8]) -> EtherType {
    EtherType::new(u16::from_be_bytes([frame[12], frame[13]]))
}

/// The destination hardware address of a frame.
pub(crate) fn destination_of(frame: &[u8]) -> MacAddr {
    MacAddr::new([frame[0], frame[1], frame[2], frame[3], frame[4], frame[5]])
}

/// Whether a frame carries an IPv4 datagram of `protocol`.
pub(crate) fn carries(frame: &[u8], protocol: Protocol) -> bool {
    ether_type_of(frame) == EtherType::IPV4
        && net_ip::Datagram::parse(&frame[PAYLOAD_AT..])
            .is_ok_and(|datagram| datagram.protocol() == protocol)
}

/// The IPv4 datagram inside a frame.
pub(crate) fn datagram_of(frame: &[u8]) -> net_ip::Datagram<'_> {
    net_ip::Datagram::parse(&frame[PAYLOAD_AT..]).expect("a datagram")
}

/// The ARP packet inside a frame.
pub(crate) fn arp_of(frame: &[u8]) -> net_eth::Packet {
    net_eth::Packet::parse(&frame[PAYLOAD_AT..]).expect("a packet")
}

/// The DHCP message inside a frame.
pub(crate) fn dhcp_of(frame: &[u8]) -> net_dhcp::Message<'_> {
    let datagram = net_ip::Datagram::parse(&frame[PAYLOAD_AT..]).expect("a datagram");
    let payload = datagram.payload();
    let udp = net_udp::Datagram::parse(
        payload,
        IpAddr::V4(datagram.source()),
        IpAddr::V4(datagram.destination()),
    )
    .expect("a datagram");
    net_dhcp::Message::parse(udp.payload).expect("a message")
}

/// A reply from the DHCP server, of `kind`, answering `xid`.
pub(crate) fn dhcp_frame(kind: net_dhcp::MessageType, xid: u32) -> Vec<u8> {
    let mut block = [0u8; 128];
    let mut options = Writer::new(&mut block);
    net_dhcp::write_option(
        &mut options,
        net_dhcp::OptionCode::MESSAGE_TYPE,
        &[kind.get()],
    )
    .expect("room");
    net_dhcp::write_option(
        &mut options,
        net_dhcp::OptionCode::SERVER_IDENTIFIER,
        &ROUTER.octets(),
    )
    .expect("room");
    net_dhcp::write_option(
        &mut options,
        net_dhcp::OptionCode::SUBNET_MASK,
        &MASK.octets(),
    )
    .expect("room");
    net_dhcp::write_option(&mut options, net_dhcp::OptionCode::ROUTER, &ROUTER.octets())
        .expect("room");
    net_dhcp::write_option(
        &mut options,
        net_dhcp::OptionCode::DOMAIN_NAME_SERVER,
        &ROUTER.octets(),
    )
    .expect("room");
    net_dhcp::write_option(
        &mut options,
        net_dhcp::OptionCode::LEASE_TIME,
        &3600u32.to_be_bytes(),
    )
    .expect("room");
    options
        .write_u8(net_dhcp::OptionCode::END.get())
        .expect("room");
    let block_len = options.position();

    let mut message = net_dhcp::Message::request(xid, MAC);
    message.op = net_dhcp::Op::REPLY;
    message.yours = HERE;
    message.options = &block[..block_len];
    let mut payload = vec![0u8; 512];
    let mut writer = Writer::new(&mut payload);
    message.write(&mut writer).expect("room");
    let len = writer.position();

    let mut datagram = vec![0u8; 1024];
    let mut writer = Writer::new(&mut datagram);
    net_udp::Datagram {
        source_port: net_dhcp::SERVER_PORT,
        destination_port: net_dhcp::CLIENT_PORT,
        payload: &payload[..len],
    }
    .write(
        &mut writer,
        IpAddr::V4(ROUTER),
        IpAddr::V4(Ipv4Addr::BROADCAST),
        net_udp::ChecksumPolicy::Computed,
    )
    .expect("room");
    let datagram_len = writer.position();
    frame(
        MacAddr::BROADCAST,
        PEER_MAC,
        EtherType::IPV4,
        &ipv4(
            ROUTER,
            Ipv4Addr::BROADCAST,
            Protocol::UDP,
            &datagram[..datagram_len],
        ),
    )
}

/// An IPv6 packet from `source` to `destination`, carrying `payload`.
pub(crate) fn ipv6(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    protocol: Protocol,
    hop_limit: u8,
    payload: &[u8],
) -> Vec<u8> {
    let mut header = net_ipv6::Header::new(source, destination, protocol, payload.len());
    header.hop_limit = hop_limit;
    let mut bytes = vec![0u8; 1600];
    let mut writer = Writer::new(&mut bytes);
    header.write(&mut writer).expect("room");
    writer.write_bytes(payload).expect("room");
    writer.finish().to_vec()
}

/// A frame carrying an IPv6 packet to this host.
pub(crate) fn ipv6_frame(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    protocol: Protocol,
    hop_limit: u8,
    payload: &[u8],
) -> Vec<u8> {
    let packet = ipv6(source, destination, protocol, hop_limit, payload);
    frame(MAC, PEER_MAC, EtherType::IPV6, &packet)
}

/// Puts the checksum of an `ICMPv6` message into it.
pub(crate) fn checksummed(source: Ipv6Addr, destination: Ipv6Addr, message: &mut [u8]) {
    message[2] = 0;
    message[3] = 0;
    let sum = transport_v6(source, destination, Protocol::ICMPV6, message).expect("it fits");
    let bytes = sum.to_be_bytes();
    message[2] = bytes[0];
    message[3] = bytes[1];
}

/// An `ICMPv6` echo request from `source` to `destination`.
pub(crate) fn echo6_frame(source: Ipv6Addr, destination: Ipv6Addr) -> Vec<u8> {
    let mut message = vec![128u8, 0, 0, 0, 0, 9, 0, 1];
    message.extend_from_slice(b"ping");
    checksummed(source, destination, &mut message);
    ipv6_frame(source, destination, Protocol::ICMPV6, 64, &message)
}

/// A UDP datagram over IPv6.
pub(crate) fn udp6_frame(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut bytes = vec![0u8; 1600];
    let mut writer = Writer::new(&mut bytes);
    net_udp::Datagram {
        source_port: Port::new(source_port),
        destination_port: Port::new(destination_port),
        payload,
    }
    .write(
        &mut writer,
        IpAddr::V6(source),
        IpAddr::V6(destination),
        net_udp::ChecksumPolicy::Computed,
    )
    .expect("room");
    let len = writer.position();
    ipv6_frame(source, destination, Protocol::UDP, 64, &bytes[..len])
}

/// A bare `SYN` over IPv6.
pub(crate) fn syn6_frame(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    source_port: u16,
    destination_port: u16,
) -> Vec<u8> {
    let mut segment = net_tcp::Segment::new(
        Port::new(source_port),
        Port::new(destination_port),
        net_tcp::Flags::SYN,
    );
    segment.seq = net_tcp::SeqNumber::new(1000);
    segment.window = 4096;
    let mut bytes = vec![0u8; 1600];
    let mut writer = Writer::new(&mut bytes);
    segment
        .write(&mut writer, IpAddr::V6(source), IpAddr::V6(destination))
        .expect("room");
    let len = writer.position();
    ipv6_frame(source, destination, Protocol::TCP, 64, &bytes[..len])
}

/// A neighbor solicitation for `target`.
pub(crate) fn solicitation_frame(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    target: Ipv6Addr,
    link_layer: Option<MacAddr>,
) -> Vec<u8> {
    let mut message = vec![0u8; 128];
    let mut writer = Writer::new(&mut message);
    net_ipv6::ndp::write_neighbor_solicitation(
        &mut writer,
        source,
        destination,
        target,
        link_layer,
    )
    .expect("room");
    let len = writer.position();
    ipv6_frame(
        source,
        destination,
        Protocol::ICMPV6,
        DISCOVERY_HOP_LIMIT,
        &message[..len],
    )
}

/// A neighbor advertisement for `target`.
pub(crate) fn advertisement_frame(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    target: Ipv6Addr,
    hardware: MacAddr,
    solicited: bool,
) -> Vec<u8> {
    let mut message = vec![0u8; 128];
    let mut writer = Writer::new(&mut message);
    net_ipv6::ndp::write_neighbor_advertisement(
        &mut writer,
        source,
        destination,
        target,
        hardware,
        solicited,
        true,
    )
    .expect("room");
    let len = writer.position();
    ipv6_frame(
        source,
        destination,
        Protocol::ICMPV6,
        DISCOVERY_HOP_LIMIT,
        &message[..len],
    )
}

/// A prefix information option (RFC 4861, section 4.6.2).
pub(crate) fn prefix_option(prefix: Ipv6Addr, valid: u32, preferred: u32) -> Vec<u8> {
    let mut option = vec![3u8, 4, 64, 0xC0];
    option.extend_from_slice(&valid.to_be_bytes());
    option.extend_from_slice(&preferred.to_be_bytes());
    option.extend_from_slice(&[0; 4]);
    option.extend_from_slice(&prefix.octets());
    option
}

/// A recursive DNS server option (RFC 8106, section 5.1).
pub(crate) fn rdnss_option(lifetime: u32, server: Ipv6Addr) -> Vec<u8> {
    let mut option = vec![25u8, 3, 0, 0];
    option.extend_from_slice(&lifetime.to_be_bytes());
    option.extend_from_slice(&server.octets());
    option
}

/// A router advertisement carrying `options`.
pub(crate) fn router_advertisement_frame(lifetime: u16, options: &[u8]) -> Vec<u8> {
    let mut message = vec![134u8, 0, 0, 0, 64, 0];
    message.extend_from_slice(&lifetime.to_be_bytes());
    message.extend_from_slice(&[0; 8]);
    message.extend_from_slice(options);
    checksummed(ROUTER6, ALL_NODES, &mut message);
    let packet = ipv6(
        ROUTER6,
        ALL_NODES,
        Protocol::ICMPV6,
        DISCOVERY_HOP_LIMIT,
        &message,
    );
    frame(
        net_eth::multicast_hardware(ALL_NODES),
        PEER_MAC,
        EtherType::IPV6,
        &packet,
    )
}

/// The IPv6 packet inside a frame.
pub(crate) fn packet_of(frame: &[u8]) -> net_ipv6::Packet<'_> {
    net_ipv6::Packet::parse(&frame[PAYLOAD_AT..]).expect("a packet")
}

/// Whether a frame carries an IPv6 packet whose next header is
/// `protocol`.
pub(crate) fn carries6(frame: &[u8], protocol: Protocol) -> bool {
    ether_type_of(frame) == EtherType::IPV6
        && net_ipv6::Packet::parse(&frame[PAYLOAD_AT..])
            .is_ok_and(|packet| packet.next_header() == protocol)
}
