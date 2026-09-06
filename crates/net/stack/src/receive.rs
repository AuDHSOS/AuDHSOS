// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Demultiplexing: which layer a frame that arrived belongs to.
//!
//! Every step is a filter, and a frame that fails one is dropped without
//! an answer. The link layer takes only what was addressed to this
//! station, to the broadcast address, or to a multicast group; the
//! internet layer takes only what was addressed to one of this host's
//! addresses or to the whole link; and the transport layers take only
//! what a socket or a connection holds. An interface with no address
//! therefore answers nothing at all, which is not a special case but what
//! falls out of the second filter when the list it looks in is empty.
//!
//! An answer is written into a buffer of this function's own before the
//! packet it answers is let go of. That is not a copy for its own sake:
//! the reassembled bytes of a datagram live in the reassembler, and
//! sending is what needs the reassembler again.

use audhsos_time::Instant;
use net_eth::{Operation, Packet};
use net_ip::{Datagram, Message};
use net_ipv6::{Discovery, Packet as Ipv6Packet};
use net_wire::{EtherType, IpAddr, Ipv4Addr, Ipv6Addr, Protocol, Writer};

use crate::stack::{FRAME_LEN, MESSAGE_LEN, PAYLOAD_LEN, Stack};

/// The all-nodes group of the link (RFC 4291, section 2.7.1), where the
/// answer to a duplicate address detection probe goes.
const ALL_NODES: Ipv6Addr =
    Ipv6Addr::from_octets([0xFF, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

/// What an arriving packet is to be answered with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Answer {
    /// Which protocol the answer is.
    protocol: Protocol,
    /// How many bytes of the buffer it filled.
    len: usize,
}

impl<const SOCKETS: usize, const CONNECTIONS: usize> Stack<'_, SOCKETS, CONNECTIONS> {
    /// Takes in one frame.
    pub(crate) fn on_frame(&mut self, bytes: &[u8], now: Instant) {
        let Some(frame) = net_eth::receive(bytes, self.config.hardware) else {
            return;
        };
        let payload = frame.payload();
        match frame.ether_type() {
            EtherType::ARP => self.on_arp(payload, now),
            EtherType::IPV4 => self.on_ipv4(payload, now),
            EtherType::IPV6 => self.on_ipv6(payload, now),
            _ => {}
        }
    }

    /// An ARP packet: an answer for one of this host's addresses, and
    /// whatever the sender taught the cache.
    fn on_arp(&mut self, payload: &[u8], now: Instant) {
        let Ok(packet) = Packet::parse(payload) else {
            return;
        };
        let sender = IpAddr::V4(packet.sender_protocol);
        let mine = self.holds_v4(packet.target_protocol);
        if mine {
            // RFC 826: a packet addressed to this host teaches the cache
            // whether it held the sender or not.
            self.neighbors
                .on_confirmed(sender, packet.sender_hardware, now);
        } else {
            // And one that is not only refreshes what is already there.
            self.neighbors
                .on_observed(sender, packet.sender_hardware, now);
        }
        if mine && packet.operation == Operation::Request {
            self.answer_arp(&packet);
        }
        self.flush_pending(sender);
    }

    /// Whether `address` is one of this host's IPv4 addresses.
    fn holds_v4(&self, address: Ipv4Addr) -> bool {
        self.is_mine(IpAddr::V4(address))
    }

    /// Writes the reply to an ARP request.
    fn answer_arp(&mut self, request: &Packet) {
        // The caller has already decided that the target is one of this
        // host's addresses, which is what `arp::respond` decides again;
        // the reply is built from the request either way.
        let reply = net_eth::Packet::reply_to(request, self.config.hardware);
        let mut payload = [0u8; MESSAGE_LEN];
        let mut writer = Writer::new(&mut payload);
        // Twenty-eight bytes into sixty-four: this cannot fail, and a
        // position of zero would put an empty frame on the link, which is
        // what putting nothing there looks like.
        let _ = reply.write(&mut writer);
        let len = writer.position();
        let bytes = payload.get(..len).unwrap_or(&[]);
        self.push_frame(request.sender_hardware, EtherType::ARP, bytes);
    }

    /// Sends what was waiting for `neighbor`, now that its hardware
    /// address is known.
    ///
    /// The cache holds the payload; the fields of the header in front of
    /// it are what the facade remembered when the datagram was held, and
    /// the sender of that family writes the header again from them.
    pub(crate) fn flush_pending(&mut self, neighbor: IpAddr) {
        let at = self
            .held
            .iter()
            .position(|entry| entry.neighbor == neighbor);
        let Some(held) = at.and_then(|at| self.held.remove(at)) else {
            return;
        };
        if self.neighbors.hardware(neighbor).is_none() {
            return;
        }
        let interface = net_ip::Interface {
            hardware: self.config.hardware,
            mtu: self.config.mtu,
        };
        let mut frame = [0u8; FRAME_LEN];
        let out = &mut self.out;
        match (held.source, held.destination, neighbor) {
            (IpAddr::V4(_), IpAddr::V4(_), _) => {
                let outgoing = net_ip::Outgoing {
                    source: held.source,
                    destination: held.destination,
                    protocol: held.protocol,
                    identification: u16::try_from(held.identification & 0xFFFF).unwrap_or(0),
                    dont_fragment: false,
                };
                let mut sender = net_ip::Sender {
                    interface,
                    routes: &self.routes,
                    neighbors: &mut self.neighbors,
                };
                let _ = sender.send_pending(neighbor, outgoing, &mut frame, |bytes| {
                    out.push(bytes);
                    Ok(())
                });
            }
            (IpAddr::V6(source), IpAddr::V6(destination), IpAddr::V6(hop)) => {
                let outgoing = net_ipv6::send::Outgoing {
                    source,
                    destination,
                    protocol: held.protocol,
                    identification: held.identification,
                };
                let mut sender = net_ipv6::Sender {
                    interface,
                    routes: &self.routes,
                    neighbors: &mut self.neighbors,
                    path: &self.path,
                };
                let _ = sender.send_pending(hop, outgoing, &mut frame, |bytes| {
                    out.push(bytes);
                    Ok(())
                });
            }
            _ => {}
        }
    }

    /// An IPv4 datagram.
    fn on_ipv4(&mut self, payload: &[u8], now: Instant) {
        let Ok(datagram) = Datagram::parse(payload) else {
            return;
        };
        if !self.accepts_v4(datagram.destination()) {
            return;
        }
        let source = IpAddr::V4(datagram.source());
        let destination = IpAddr::V4(datagram.destination());
        let protocol = datagram.protocol();
        let mut buffer = [0u8; PAYLOAD_LEN];
        let answer = {
            let Ok(Some(assembled)) = self.fragments.accept(datagram, now) else {
                return;
            };
            let whole = assembled.payload();
            match protocol {
                Protocol::ICMP => icmp_answer(whole, &mut buffer),
                Protocol::UDP => {
                    // Port 68 belongs to the address configuration
                    // client, which is not a socket and must be reachable
                    // before this host has an address to bind one to.
                    if !deliver_dhcp(&mut self.dhcp, source, destination, whole, now) {
                        self.sockets.receive(source, destination, whole);
                    }
                    None
                }
                Protocol::TCP => tcp_answer(
                    &mut self.connections,
                    source,
                    destination,
                    whole,
                    now,
                    &mut buffer,
                ),
                _ => None,
            }
        };
        self.fragments.release(datagram);
        let Some(answer) = answer else {
            return;
        };
        let from = if self.is_mine(destination) {
            Some(destination)
        } else {
            self.source_for(source)
        };
        let Some(from) = from else {
            return;
        };
        let bytes = buffer.get(..answer.len).unwrap_or(&[]);
        let _ = self.transmit(from, source, answer.protocol, bytes, now);
    }

    /// Whether an IPv4 datagram to `destination` is for this host.
    fn accepts_v4(&self, destination: Ipv4Addr) -> bool {
        if destination.is_broadcast() || destination.is_multicast() {
            return true;
        }
        if self.holds_v4(destination) {
            return true;
        }
        // The directed broadcast of the network a lease put this host on.
        self.lease()
            .is_some_and(|lease| lease.network.broadcast() == destination)
    }

    /// An IPv6 packet.
    fn on_ipv6(&mut self, payload: &[u8], now: Instant) {
        let Ok(packet) = Ipv6Packet::parse(payload) else {
            return;
        };
        if !self.accepts_v6(packet.destination()) {
            return;
        }
        let Ok(upper) = packet.upper_layer() else {
            return;
        };
        let source = IpAddr::V6(packet.source());
        let destination = IpAddr::V6(packet.destination());
        // Neighbor Discovery is never fragmented and is answered before
        // the reassembler is asked anything, because its messages have to
        // reach the cache with the packet they arrived in.
        if upper.protocol == Protocol::ICMPV6
            && packet.hop_limit() == crate::stack::DISCOVERY_HOP_LIMIT
            && let Ok(discovery) = net_ipv6::ndp::receive(packet, upper.payload)
        {
            self.on_discovery(packet.source(), &discovery, now);
            return;
        }
        let mut buffer = [0u8; PAYLOAD_LEN];
        let answer = {
            let Ok(Some(whole)) =
                net_ipv6::fragment::reassemble(&mut self.fragments, packet, upper, now)
            else {
                return;
            };
            match upper.protocol {
                Protocol::ICMPV6 => {
                    icmpv6_answer(packet.source(), packet.destination(), whole, &mut buffer)
                }
                Protocol::UDP => {
                    self.sockets.receive(source, destination, whole);
                    None
                }
                Protocol::TCP => tcp_answer(
                    &mut self.connections,
                    source,
                    destination,
                    whole,
                    now,
                    &mut buffer,
                ),
                _ => None,
            }
        };
        net_ipv6::fragment::release(&mut self.fragments, packet, upper);
        let Some(answer) = answer else {
            return;
        };
        let from = if self.is_mine(destination) {
            Some(destination)
        } else {
            self.source_for(source)
        };
        let Some(from) = from else {
            return;
        };
        let bytes = buffer.get(..answer.len).unwrap_or(&[]);
        let _ = self.transmit(from, source, answer.protocol, bytes, now);
    }

    /// Whether an IPv6 packet to `destination` is for this host.
    fn accepts_v6(&self, destination: Ipv6Addr) -> bool {
        destination.is_multicast() || self.is_mine(IpAddr::V6(destination))
    }

    /// A Neighbor Discovery message: what it teaches the cache, what a
    /// solicitation for one of this host's addresses is answered with,
    /// and what a router advertisement configures.
    fn on_discovery(&mut self, source: Ipv6Addr, discovery: &Discovery<'_>, now: Instant) {
        // An address this host is still checking is one somebody else may
        // be claiming, and both a claim and a competing check are how it
        // hears so (RFC 4862, section 5.4.3 and 5.4.4).
        if let Some(dad) = self.dad.as_mut() {
            dad.on_advertisement(discovery);
            dad.on_solicitation(source, discovery);
        }
        match discovery {
            Discovery::NeighborAdvertisement { .. } => {
                net_ipv6::ndp::on_advertisement(&mut self.neighbors, discovery, now);
                if let Discovery::NeighborAdvertisement { target, .. } = *discovery {
                    self.flush_pending(IpAddr::V6(target));
                }
            }
            Discovery::NeighborSolicitation { target, .. } => {
                net_ipv6::ndp::on_solicitation(&mut self.neighbors, source, discovery, now);
                let target = *target;
                if self.is_mine(IpAddr::V6(target)) {
                    self.answer_solicitation(source, target);
                }
                self.flush_pending(IpAddr::V6(source));
            }
            Discovery::RouterAdvertisement { .. } => {
                if self
                    .slaac
                    .on_advertisement(source, discovery, self.config.hardware, now)
                    .is_ok()
                {
                    self.configure_from_advertisement(now);
                }
            }
            Discovery::RouterSolicitation { .. } => {}
        }
    }

    /// Answers a neighbor solicitation for one of this host's addresses.
    fn answer_solicitation(&mut self, source: Ipv6Addr, target: Ipv6Addr) {
        // A solicitation whose source is unspecified is a duplicate
        // address detection probe, and RFC 4862, section 5.4.3 has the
        // answer go to the all-nodes group rather than to nobody.
        let solicited = !source.is_unspecified();
        let destination = if solicited { source } else { ALL_NODES };
        let mut message = [0u8; MESSAGE_LEN];
        let mut writer = Writer::new(&mut message);
        let _ = net_ipv6::ndp::write_neighbor_advertisement(
            &mut writer,
            target,
            destination,
            target,
            self.config.hardware,
            solicited,
            true,
        );
        let len = writer.position();
        // `message` is a buffer of this function's own, so what was
        // written into it can go out of it without a copy.
        self.send_discovery(target, destination, message.get(..len).unwrap_or(&[]));
    }
}

/// Hands a datagram to the address configuration client when it is for
/// it, and answers whether it was.
fn deliver_dhcp(
    client: &mut net_dhcp::Client,
    source: IpAddr,
    destination: IpAddr,
    payload: &[u8],
    now: Instant,
) -> bool {
    let Ok(datagram) = net_udp::Datagram::parse(payload, source, destination) else {
        return false;
    };
    if datagram.destination_port != net_dhcp::CLIENT_PORT {
        return false;
    }
    client.on_datagram(source, datagram.source_port, datagram.payload, now);
    true
}

/// Hands a segment to its connection, and writes the reset that a segment
/// to nothing at all earns.
fn tcp_answer<const CONNECTIONS: usize>(
    connections: &mut net_tcp::Connections<'_, CONNECTIONS>,
    source: IpAddr,
    destination: IpAddr,
    payload: &[u8],
    now: Instant,
    buffer: &mut [u8],
) -> Option<Answer> {
    let net_tcp::Delivery::Refused(reset) = connections.receive(source, destination, payload, now)
    else {
        return None;
    };
    let mut writer = Writer::new(buffer);
    reset.write(&mut writer, destination, source).ok()?;
    Some(Answer {
        protocol: Protocol::TCP,
        len: writer.position(),
    })
}

/// The `ICMPv4` answer to `payload`, when there is one.
fn icmp_answer(payload: &[u8], buffer: &mut [u8]) -> Option<Answer> {
    let message = Message::parse(payload).ok()?;
    let reply = message.reply()?;
    let mut writer = Writer::new(buffer);
    reply.write(&mut writer).ok()?;
    Some(Answer {
        protocol: Protocol::ICMP,
        len: writer.position(),
    })
}

/// The `ICMPv6` answer to `payload`, when there is one.
fn icmpv6_answer(
    source: Ipv6Addr,
    destination: Ipv6Addr,
    payload: &[u8],
    buffer: &mut [u8],
) -> Option<Answer> {
    let message = net_ipv6::icmp::Message::parse(payload).ok()?;
    let reply = message.reply()?;
    let mut writer = Writer::new(buffer);
    reply.write(destination, source, &mut writer).ok()?;
    Some(Answer {
        protocol: Protocol::ICMPV6,
        len: writer.position(),
    })
}
