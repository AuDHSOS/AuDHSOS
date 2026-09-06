// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `poll` and `poll_at`: the one entry point, and the one instant it
//! next has work at.
//!
//! Every machine under this crate answers the same two questions — what
//! do you have to say now, and when do you next have something — and all
//! `poll` does is ask them in turn and put what comes back in the
//! outgoing queue. What `poll_at` answers is the earliest of their
//! answers, so a server never polls in a loop and a test never sleeps.
//!
//! The layers are asked only when the queue is empty. That is what makes
//! a transmit buffer of one frame enough: the caller drains what has been
//! written before anything new is written, and nothing is lost or
//! reordered because nothing is produced while there is a backlog.

use audhsos_time::Instant;
use crypto_rng::Rng;
use net_eth::Event;
use net_ipv6::{DadEvent, Expired};
use net_wire::{
    EtherType, IpAddr, IpCidr, Ipv4Addr, Ipv4Cidr, Ipv6Addr, MacAddr, Protocol, Writer,
};

use crate::error::StackError;
use crate::stack::{PAYLOAD_LEN, Stack};

/// How much room one message of the link protocols is written into. An
/// ARP packet is twenty-eight bytes and the longest Neighbor Discovery
/// message this crate writes is under fifty, so nothing here can fill it.
const MESSAGE_LEN: usize = 64;

/// The all-nodes group of the link, where a duplicate address detection
/// answer goes (RFC 4291, section 2.7.1).
const ALL_NODES: Ipv6Addr =
    Ipv6Addr::from_octets([0xFF, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

impl<const SOCKETS: usize, const CONNECTIONS: usize> Stack<'_, SOCKETS, CONNECTIONS> {
    /// Takes in at most one frame and hands out at most one.
    ///
    /// A caller loops until this answers `None` with nothing to give it,
    /// and asks [`poll_at`](Stack::poll_at) when it does.
    ///
    /// # Errors
    ///
    /// Whatever a layer said that stopped the whole poll. A layer that
    /// merely could not send is not an error: the frame is counted as
    /// dropped and the machine that wrote it says it again.
    pub fn poll<'t, R: Rng + ?Sized>(
        &mut self,
        now: Instant,
        rx: Option<&[u8]>,
        tx: &'t mut [u8],
        rng: &mut R,
    ) -> Result<Option<&'t [u8]>, StackError> {
        if let Some(frame) = rx {
            self.on_frame(frame, now);
        }
        if self.out.is_empty() {
            self.drive(now, rng)?;
        }
        Ok(self.out.pop_into(tx))
    }

    /// When the stack next has something to say.
    #[must_use]
    pub fn poll_at(&self, now: Instant) -> Option<Instant> {
        if !self.out.is_empty() {
            return Some(now);
        }
        [
            self.neighbors.poll_at(),
            self.fragments.poll_at(),
            self.dhcp.poll_at(),
            self.slaac.poll_at(),
            self.dad.as_ref().and_then(net_ipv6::Dad::poll_at),
            self.resolver.as_ref().and_then(net_dns::Resolver::poll_at),
            self.connections.poll_at(now),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Asks every machine what it has to say.
    fn drive<R: Rng + ?Sized>(&mut self, now: Instant, rng: &mut R) -> Result<(), StackError> {
        self.fragments.poll(now);
        self.drive_neighbors(now);
        self.drive_dhcp(now, rng);
        self.drive_slaac(now);
        self.drive_dad(now);
        self.drive_resolver(now, rng);
        // The attempt is moved along before the connections are asked,
        // so that a connection this pass opened says its `SYN` in this
        // pass and not in the next one.
        self.drive_attempt(now, rng)?;
        self.drive_connections(now);
        Ok(())
    }

    /// The neighbor cache: what it wants asked, and of whom.
    fn drive_neighbors(&mut self, now: Instant) {
        while let Some(event) = self.neighbors.poll(now) {
            let Event::Solicit { address, hardware } = event else {
                continue;
            };
            match address {
                IpAddr::V4(target) => self.solicit_v4(target, hardware),
                IpAddr::V6(target) => self.solicit_v6(target, hardware),
            }
            if self.out.len() >= 2 {
                break;
            }
        }
    }

    /// Asks the link who holds an IPv4 address.
    fn solicit_v4(&mut self, target: Ipv4Addr, hardware: Option<MacAddr>) {
        let Some(IpAddr::V4(source)) = self.source_for(IpAddr::V4(target)) else {
            return;
        };
        let packet = net_eth::Packet::request(self.config.hardware, source, target);
        let mut payload = [0u8; MESSAGE_LEN];
        let mut writer = Writer::new(&mut payload);
        // An ARP packet is twenty-eight bytes and the buffer holds
        // sixty-four, so this cannot fail; a position of zero would send
        // a frame of nothing, which is what sending nothing looks like.
        let _ = packet.write(&mut writer);
        let len = writer.position();
        let bytes = payload.get(..len).unwrap_or(&[]);
        self.push_frame(
            hardware.unwrap_or(MacAddr::BROADCAST),
            EtherType::ARP,
            bytes,
        );
    }

    /// Asks the link who holds an IPv6 address.
    fn solicit_v6(&mut self, target: Ipv6Addr, hardware: Option<MacAddr>) {
        let Some(IpAddr::V6(source)) = self.source_for(IpAddr::V6(target)) else {
            return;
        };
        // A probe goes to the neighbor that answered last; a first ask
        // goes to the group derived from the address being asked for.
        let destination = if hardware.is_some() {
            target
        } else {
            solicited_node(target)
        };
        let mut payload = [0u8; MESSAGE_LEN];
        let mut writer = Writer::new(&mut payload);
        let _ = net_ipv6::ndp::write_neighbor_solicitation(
            &mut writer,
            source,
            destination,
            target,
            Some(self.config.hardware),
        );
        let len = writer.position();
        self.send_discovery(source, destination, payload.get(..len).unwrap_or(&[]));
    }

    /// The address configuration client.
    fn drive_dhcp<R: Rng + ?Sized>(&mut self, now: Instant, rng: &mut R) {
        let mut buffer = [0u8; PAYLOAD_LEN];
        let mut datagram = [0u8; PAYLOAD_LEN];
        let plan = match self.dhcp.poll(now, rng, &mut buffer) {
            Ok(Some(outgoing)) => {
                let len = outgoing.datagram.len();
                datagram.get_mut(..len).map(|slot| {
                    slot.copy_from_slice(outgoing.datagram);
                    (outgoing.source, outgoing.destination, len)
                })
            }
            Ok(None) | Err(_) => None,
        };
        if let Some((source, destination, len)) = plan {
            let bytes = datagram.get(..len).unwrap_or(&[]);
            let _ = self.transmit(source, destination, Protocol::UDP, bytes, now);
        }
        self.reconcile_lease();
    }

    /// Puts what a lease says into the address table and the routes, and
    /// takes it out again when the lease is gone.
    fn reconcile_lease(&mut self) {
        let leased = self.dhcp.address();
        if leased == self.leased {
            return;
        }
        if let Some(old) = self.leased.take() {
            self.remove_address(IpAddr::V4(old));
            while let Some(route) = self.leased_routes.pop() {
                self.routes.remove(route);
            }
        }
        self.leased = leased;
        let Some(lease) = self.dhcp.lease() else {
            return;
        };
        let address = lease.network.address();
        let prefix = lease.network.prefix_len();
        let router = lease.router;
        let Ok(network) = Ipv4Cidr::new(lease.network.network(), prefix) else {
            return;
        };
        let _ = self.add_address(IpAddr::V4(address));
        if self
            .routes
            .add(net_ip::Route::on_link(IpCidr::V4(network)))
            .is_ok()
        {
            let _ = self.leased_routes.push(IpCidr::V4(network));
        }
        if let Some(router) = router
            && let Ok(default) = Ipv4Cidr::new(Ipv4Addr::UNSPECIFIED, 0)
            && self
                .routes
                .add(net_ip::Route::via(IpCidr::V4(default), IpAddr::V4(router)))
                .is_ok()
        {
            let _ = self.leased_routes.push(IpCidr::V4(default));
        }
    }

    /// Puts what a router advertisement configured into the routing table,
    /// and starts checking a newly formed address for a duplicate.
    pub(crate) fn configure_from_advertisement(&mut self, now: Instant) {
        let _ = self.slaac.install(&mut self.routes);
        let Some(address) = self.slaac.address() else {
            return;
        };
        if self.is_mine(IpAddr::V6(address)) || self.dad.is_some() {
            return;
        }
        // RFC 4862, section 5.4: an address this host formed is tentative
        // until nobody has claimed it.
        self.dad = Some(net_ipv6::Dad::new(address, now));
    }

    /// What the router advertisements configured, and what ran out of it.
    fn drive_slaac(&mut self, now: Instant) {
        while let Some(expired) = self.slaac.poll(now) {
            match expired {
                Expired::Prefix(prefix) => {
                    self.routes.remove(IpCidr::V6(prefix));
                    let held: Option<IpAddr> =
                        self.addresses.iter().copied().find(
                            |address| matches!(address, IpAddr::V6(v6) if prefix.contains(*v6)),
                        );
                    if let Some(address) = held {
                        self.remove_address(address);
                    }
                }
                Expired::Router(_) | Expired::Server(_) => {}
            }
        }
        let _ = self.slaac.install(&mut self.routes);
    }

    /// Duplicate address detection, which is what stands between an
    /// address this host formed and an address it uses.
    fn drive_dad(&mut self, now: Instant) {
        let Some(dad) = self.dad.as_mut() else {
            return;
        };
        let Some(event) = dad.poll(now) else {
            return;
        };
        match event {
            DadEvent::Solicit { target, group } => self.probe(target, group),
            DadEvent::Unique(address) => {
                let _ = self.add_address(IpAddr::V6(address));
                self.dad = None;
            }
            DadEvent::Duplicate(_) => {
                self.dad = None;
            }
        }
    }

    /// One duplicate address detection probe: a solicitation from the
    /// unspecified address, with no link-layer option (RFC 4862,
    /// section 5.4.2).
    fn probe(&mut self, target: Ipv6Addr, group: Ipv6Addr) {
        let mut payload = [0u8; MESSAGE_LEN];
        let mut writer = Writer::new(&mut payload);
        // A solicitation is thirty-two bytes at most and the buffer holds
        // twice that, so a failure here is a case that cannot arise; what
        // it would leave is a position of zero and a frame of nothing,
        // which is the same as sending nothing.
        let _ = net_ipv6::ndp::write_neighbor_solicitation(
            &mut writer,
            Ipv6Addr::UNSPECIFIED,
            group,
            target,
            None,
        );
        let len = writer.position();
        self.send_discovery(
            Ipv6Addr::UNSPECIFIED,
            group,
            payload.get(..len).unwrap_or(&[]),
        );
    }

    /// The open connections: whatever each of them has to send.
    fn drive_connections(&mut self, now: Instant) {
        // The scratch is one buffer for the whole walk and the plan holds
        // only a length, so what a connection wrote survives the end of
        // the borrow that wrote it and nothing is copied a second time.
        let mut segment = [0u8; PAYLOAD_LEN];
        let mut plan: Option<(IpAddr, IpAddr, usize)> = None;
        for (_, connection) in self.connections.iter_mut() {
            let Some(bytes) = connection.poll(now, &mut segment) else {
                continue;
            };
            let len = bytes.len();
            plan = Some((connection.local().address, connection.remote().address, len));
            break;
        }
        if let Some((source, destination, len)) = plan {
            let bytes = segment.get(..len).unwrap_or(&[]);
            let _ = self.transmit(source, destination, Protocol::TCP, bytes, now);
        }
    }
}

/// The solicited-node multicast group of `address` (RFC 4291,
/// section 2.7.1): `ff02::1:ff` and the low three bytes.
#[must_use]
pub const fn solicited_node(address: Ipv6Addr) -> Ipv6Addr {
    let octets = address.octets();
    let [.., thirteenth, fourteenth, fifteenth] = octets;
    Ipv6Addr::from_octets([
        0xFF, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0xFF, thirteenth, fourteenth, fifteenth,
    ])
}

/// The all-nodes group, for a caller that needs to name it.
#[must_use]
pub const fn all_nodes() -> Ipv6Addr {
    ALL_NODES
}
