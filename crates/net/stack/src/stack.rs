// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The facade: one interface, one `poll`, one `poll_at`.
//!
//! Everything under it is a component that takes time as an argument and
//! writes into a buffer; what this crate adds is the wiring — which layer
//! a frame goes to, which of the two senders carries a datagram out, and
//! which of the many deadlines is the next one. A server process is a
//! loop around `poll` and nothing else.
//!
//! Two limits are the caller's and the rest are this host's. How many
//! sockets and how many connections is what an application sizes, so both
//! are parameters. How many routes, how many neighbors, how many
//! reassembly buffers and how many prefixes are properties of a host with
//! one interface, whatever runs on it, so they are constants of this
//! crate — and a type with seven numbers in it is a type nobody writes
//! down twice.

use audhsos_collections::ArrayVec;
use audhsos_time::Instant;
use crypto_rng::Rng;
use net_dhcp::{Client, Lease};
use net_dns::Resolver;
use net_eth::{HEADER_LEN as ETH_HEADER_LEN, MAX_FRAME_LEN, MTU, NeighborCache};
use net_ip::{Header, Interface, Outgoing, Reassembler, Route, RoutingTable, Sender, Sent};
use net_ipv6::{Configuration, Dad, PathMtu};
use net_tcp::{Connection, Connections};
use net_udp::{Socket, Sockets};
use net_wire::{EtherType, IpAddr, IpCidr, Ipv4Addr, MacAddr, Port, Protocol, Writer};

use crate::error::StackError;
use crate::handle::{Handle, Slots};
use crate::queue::Queue;
use crate::select;

/// How many routes this host holds.
pub const ROUTES: usize = 8;

/// How many neighbors the cache holds.
pub const NEIGHBORS: usize = 16;

/// How many bytes of one datagram wait behind an unresolved neighbor.
pub const HELD: usize = 512;

/// How many datagrams are reassembled at once.
pub const SLOTS: usize = 4;

/// How many bytes one reassembled datagram may come to.
pub const REASSEMBLY: usize = 2048;

/// How many prefixes a router advertisement may configure.
pub const PREFIXES: usize = 4;

/// How many name servers a router advertisement may name.
pub const ADVERTISED: usize = 4;

/// How many paths the MTU estimates cover.
pub const PATHS: usize = 8;

/// How many addresses this host holds, over both families.
pub const ADDRESSES: usize = 4;

/// How many addresses a name may resolve to before the rest are dropped.
pub const CANDIDATES: usize = net_dns::MAX_ADDRESSES;

/// The longest frame, which is what a transmit buffer must hold to be
/// sure of never blocking on one.
pub const FRAME_LEN: usize = MAX_FRAME_LEN;

/// The longest payload a frame carries, which is what the scratch buffers
/// of one poll are sized by.
pub const PAYLOAD_LEN: usize = MTU;

/// The hop limit every Neighbor Discovery message carries and is believed
/// at (RFC 4861, section 7.1).
pub const DISCOVERY_HOP_LIMIT: u8 = 255;

/// How much room one message of the link protocols is written into.
pub const MESSAGE_LEN: usize = 64;

/// What the interface is and what the machines above it are configured
/// with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The hardware address frames leave with.
    pub hardware: MacAddr,
    /// The largest payload a frame carries.
    pub mtu: usize,
    /// What the address configuration client is configured with.
    pub dhcp: net_dhcp::Config,
    /// What the resolver is configured with.
    pub resolver: net_dns::Config,
    /// What a connection is configured with.
    pub tcp: net_tcp::Config,
}

impl Config {
    /// The defaults of every layer, for the interface at `hardware`.
    #[must_use]
    pub const fn new(hardware: MacAddr) -> Config {
        Config {
            hardware,
            mtu: MTU,
            dhcp: net_dhcp::Config::DEFAULT,
            resolver: net_dns::Config::DEFAULT,
            tcp: net_tcp::Config::DEFAULT,
        }
    }
}

/// A datagram that is waiting for a neighbor to answer.
///
/// The cache holds the payload and nothing else, because the two internet
/// layers write their own headers and the fields those headers carry are
/// not the cache's business. So the facade remembers the fields: they are
/// what `send_pending` needs when the answer comes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Held {
    /// Whose answer it is waiting for.
    pub(crate) neighbor: IpAddr,
    /// Which of this host's addresses it leaves from.
    pub(crate) source: IpAddr,
    /// Where it goes.
    pub(crate) destination: IpAddr,
    /// What the payload is.
    pub(crate) protocol: Protocol,
    /// Which datagram it is.
    pub(crate) identification: u32,
}

/// How many datagrams wait for an answer at once. The cache holds one per
/// neighbor, and this is how many of those the facade remembers the
/// fields of.
pub const WAITING: usize = 4;

/// Where a connection that is being made to a list of addresses stands.
#[derive(Debug)]
pub(crate) struct Attempt<'a> {
    /// The addresses still to try, in the order they are tried.
    pub(crate) candidates: ArrayVec<IpAddr, CANDIDATES>,
    /// How many of them have been tried.
    pub(crate) at: usize,
    /// Which port they are tried at.
    pub(crate) port: Port,
    /// The buffers the connection is made over, held while the resolver
    /// is still working and while one candidate is given up for the next.
    pub(crate) buffers: Option<(&'a mut [u8], &'a mut [u8])>,
    /// The connection, once one is open.
    pub(crate) handle: Option<Handle>,
    /// Whether the addresses are still being resolved.
    pub(crate) resolving: bool,
}

/// The stack.
#[derive(Debug)]
pub struct Stack<'a, const SOCKETS: usize, const CONNECTIONS: usize> {
    /// What it was configured with.
    pub(crate) config: Config,
    /// This host's addresses.
    pub(crate) addresses: ArrayVec<IpAddr, ADDRESSES>,
    /// Where things go.
    pub(crate) routes: RoutingTable<ROUTES>,
    /// Which hardware address holds which internet address.
    pub(crate) neighbors: NeighborCache<NEIGHBORS, HELD>,
    /// The datagrams being put back together.
    pub(crate) fragments: Reassembler<SLOTS, REASSEMBLY>,
    /// What each path is known to carry.
    pub(crate) path: PathMtu<PATHS>,
    /// The frames that have been written and not yet handed over.
    pub(crate) out: Queue<'a>,
    /// The UDP sockets.
    pub(crate) sockets: Sockets<'a, SOCKETS>,
    /// Their generations.
    pub(crate) socket_slots: Slots<SOCKETS>,
    /// The TCP connections.
    pub(crate) connections: Connections<'a, CONNECTIONS>,
    /// Their generations.
    pub(crate) connection_slots: Slots<CONNECTIONS>,
    /// What goes in the identification field of the next IPv4 datagram.
    pub(crate) identification: u16,
    /// What goes in the identification field of the next IPv6 fragment.
    pub(crate) identification6: u32,
    /// The address configuration client.
    pub(crate) dhcp: Client,
    /// What the router advertisements configured.
    pub(crate) slaac: Configuration<PREFIXES, ADVERTISED>,
    /// The address the last lease granted, so that a lease that changed
    /// can be told from one that did not.
    pub(crate) leased: Option<Ipv4Addr>,
    /// The routes that lease installed, to be taken out with it.
    pub(crate) leased_routes: ArrayVec<IpCidr, 2>,
    /// Duplicate address detection, while an address is being checked.
    pub(crate) dad: Option<Dad>,
    /// What the datagrams waiting for a neighbor were being sent as.
    pub(crate) held: ArrayVec<Held, WAITING>,
    /// The resolver, while a resolution is running.
    pub(crate) resolver: Option<Resolver>,
    /// The socket the resolver sends from, while it has one.
    pub(crate) resolver_socket: Option<Handle>,
    /// The connection being made to a list of addresses.
    pub(crate) attempt: Option<Attempt<'a>>,
}

impl<'a, const SOCKETS: usize, const CONNECTIONS: usize> Stack<'a, SOCKETS, CONNECTIONS> {
    /// A stack with no address, no route and nothing open.
    ///
    /// `outgoing` is where frames wait between being written and being
    /// handed to the driver. Everything else a layer needs memory for is
    /// given to the operation that needs it: a socket's ring at `bind`, a
    /// connection's two windows at `connect`.
    #[must_use]
    pub const fn new(config: Config, outgoing: &'a mut [u8]) -> Self {
        Stack {
            config,
            addresses: ArrayVec::new(),
            routes: RoutingTable::new(),
            neighbors: NeighborCache::new(),
            fragments: Reassembler::new(),
            path: PathMtu::new(),
            out: Queue::new(outgoing),
            sockets: Sockets::new(),
            socket_slots: Slots::new(),
            connections: Connections::new(),
            connection_slots: Slots::new(),
            identification: 0,
            identification6: 0,
            dhcp: Client::new(config.hardware, config.dhcp),
            slaac: Configuration::new(),
            leased: None,
            leased_routes: ArrayVec::new(),
            dad: None,
            held: ArrayVec::new(),
            resolver: None,
            resolver_socket: None,
            attempt: None,
        }
    }

    /// What it was configured with.
    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// This host's addresses.
    pub fn addresses(&self) -> impl Iterator<Item = IpAddr> + '_ {
        self.addresses.iter().copied()
    }

    /// Whether `address` is one of this host's.
    #[must_use]
    pub fn is_mine(&self, address: IpAddr) -> bool {
        self.addresses.iter().any(|held| *held == address)
    }

    /// Gives this host an address.
    ///
    /// # Errors
    ///
    /// [`StackError::TooManyAddresses`] when the table is full.
    pub fn add_address(&mut self, address: IpAddr) -> Result<(), StackError> {
        if self.is_mine(address) {
            return Ok(());
        }
        self.addresses
            .push(address)
            .map_err(|_| StackError::TooManyAddresses)
    }

    /// Takes an address away and forgets what was learned about it.
    pub fn remove_address(&mut self, address: IpAddr) -> bool {
        let Some(at) = self.addresses.iter().position(|held| *held == address) else {
            return false;
        };
        self.addresses.remove(at);
        true
    }

    /// The routing table, to read.
    #[must_use]
    pub const fn routes(&self) -> &RoutingTable<ROUTES> {
        &self.routes
    }

    /// Adds a route.
    ///
    /// # Errors
    ///
    /// Whatever [`RoutingTable::add`] returns.
    pub fn add_route(&mut self, route: Route) -> Result<(), StackError> {
        self.routes.add(route)?;
        Ok(())
    }

    /// Takes a route away.
    pub fn remove_route(&mut self, destination: IpCidr) -> bool {
        self.routes.remove(destination)
    }

    /// The neighbor cache, to read.
    #[must_use]
    pub const fn neighbors(&self) -> &NeighborCache<NEIGHBORS, HELD> {
        &self.neighbors
    }

    /// How many frames were dropped for want of room in the queue.
    #[must_use]
    pub const fn dropped(&self) -> u32 {
        self.out.dropped()
    }

    /// Which of this host's addresses a packet to `destination` leaves
    /// from, under RFC 6724 (see [`crate::select`]).
    #[must_use]
    pub fn source_for(&self, destination: IpAddr) -> Option<IpAddr> {
        // The table holds at most `ADDRESSES`, so zipping it onto an
        // array of that size takes all of it.
        let mut candidates = [IpAddr::V4(Ipv4Addr::UNSPECIFIED); ADDRESSES];
        let mut count = 0usize;
        for (slot, address) in candidates.iter_mut().zip(&self.addresses) {
            *slot = *address;
            count = count.saturating_add(1);
        }
        select::source_for(destination, candidates.get(..count).unwrap_or(&[]))
    }

    /// Puts `addresses` in the order this host should try them, under
    /// RFC 6724.
    pub fn order(&self, addresses: &mut [IpAddr]) {
        select::order_destinations(addresses, &|destination| self.source_for(destination));
    }
}

impl<'a, const SOCKETS: usize, const CONNECTIONS: usize> Stack<'a, SOCKETS, CONNECTIONS> {
    /// Opens a UDP socket on `port`, receiving into `buffer`.
    ///
    /// # Errors
    ///
    /// Whatever [`Sockets::bind`] returns, and [`StackError::Full`] when
    /// the slot's generation has run out.
    pub fn bind(
        &mut self,
        local: Option<IpAddr>,
        port: Port,
        buffer: &'a mut [u8],
    ) -> Result<Handle, StackError> {
        let id = self.sockets.bind(local, port, buffer)?;
        self.socket_slots.handle(id.index())
    }

    /// Opens a UDP socket on a port drawn from the dynamic range.
    ///
    /// # Errors
    ///
    /// Whatever [`Sockets::bind_ephemeral`] returns.
    pub fn bind_ephemeral<R: Rng + ?Sized>(
        &mut self,
        local: Option<IpAddr>,
        rng: &mut R,
        buffer: &'a mut [u8],
    ) -> Result<Handle, StackError> {
        let id = self.sockets.bind_ephemeral(local, rng, buffer)?;
        self.socket_slots.handle(id.index())
    }

    /// The socket `handle` names, to read datagrams out of.
    ///
    /// # Errors
    ///
    /// [`StackError::Stale`] for a handle to a socket that was closed,
    /// and [`StackError::Unknown`] for one that names no slot.
    pub fn socket(&mut self, handle: Handle) -> Result<&mut Socket<'a>, StackError> {
        let index = self.socket_slots.resolve(handle)?;
        self.sockets
            .get_mut(net_udp::SocketId::new(index))
            .ok_or(StackError::Stale)
    }

    /// Closes a socket and gives its memory back.
    ///
    /// # Errors
    ///
    /// Whatever [`socket`](Stack::socket) returns.
    pub fn close(&mut self, handle: Handle) -> Result<&'a mut [u8], StackError> {
        let index = self.socket_slots.resolve(handle)?;
        let buffer = self.sockets.close(net_udp::SocketId::new(index))?;
        self.socket_slots.retire(index);
        Ok(buffer)
    }

    /// Sends `payload` from the socket `handle` names to `destination` at
    /// `port`.
    ///
    /// # Errors
    ///
    /// [`StackError::NoAddress`] when nothing of the destination's family
    /// can carry it, and whatever the layers below say.
    pub fn send_to(
        &mut self,
        handle: Handle,
        destination: IpAddr,
        port: Port,
        payload: &[u8],
        now: Instant,
    ) -> Result<(), StackError> {
        let index = self.socket_slots.resolve(handle)?;
        let socket = self
            .sockets
            .get(net_udp::SocketId::new(index))
            .ok_or(StackError::Stale)?;
        let source = socket
            .local()
            .or_else(|| self.source_for(destination))
            .ok_or(StackError::NoAddress)?;
        let source_port = socket.port();
        let mut scratch = [0u8; MTU];
        let mut writer = Writer::new(&mut scratch);
        net_udp::Datagram {
            source_port,
            destination_port: port,
            payload,
        }
        .write(
            &mut writer,
            source,
            destination,
            net_udp::ChecksumPolicy::Computed,
        )?;
        let len = writer.position();
        let datagram = scratch.get(..len).unwrap_or(&[]);
        self.transmit(source, destination, Protocol::UDP, datagram, now)?;
        Ok(())
    }

    /// Puts a connection in `LISTEN` on `port`.
    ///
    /// # Errors
    ///
    /// [`StackError::NoAddress`] when this host has none, and whatever
    /// [`Connections::listen`] returns.
    pub fn listen<R: Rng + ?Sized>(
        &mut self,
        local: IpAddr,
        port: Port,
        rng: &mut R,
        send: &'a mut [u8],
        receive: &'a mut [u8],
    ) -> Result<Handle, StackError> {
        let endpoint = net_tcp::Endpoint::new(local, port);
        let id = self
            .connections
            .listen(endpoint, self.config.tcp, rng, send, receive)?;
        self.connection_slots.handle(id.index())
    }

    /// Opens a connection to `address` at `port`.
    ///
    /// # Errors
    ///
    /// [`StackError::NoAddress`] when nothing of that family can reach
    /// it, and whatever [`Connections::connect`] returns.
    pub fn connect_to<R: Rng + ?Sized>(
        &mut self,
        address: IpAddr,
        port: Port,
        rng: &mut R,
        send: &'a mut [u8],
        receive: &'a mut [u8],
    ) -> Result<Handle, StackError> {
        let source = self.source_for(address).ok_or(StackError::NoAddress)?;
        let local = net_tcp::Endpoint::new(source, Port::new(0));
        let remote = net_tcp::Endpoint::new(address, port);
        let id = self
            .connections
            .connect(local, remote, self.config.tcp, rng, send, receive)?;
        self.connection_slots.handle(id.index())
    }

    /// The connection `handle` names.
    ///
    /// # Errors
    ///
    /// [`StackError::Stale`] for a handle to a connection that was
    /// closed, and [`StackError::Unknown`] for one that names no slot.
    pub fn connection(&mut self, handle: Handle) -> Result<&mut Connection<'a>, StackError> {
        let index = self.connection_slots.resolve(handle)?;
        self.connections
            .get_mut(net_tcp::ConnectionId::new(index))
            .ok_or(StackError::Stale)
    }

    /// Closes a connection and gives its two buffers back.
    ///
    /// # Errors
    ///
    /// Whatever [`connection`](Stack::connection) returns.
    pub fn close_connection(
        &mut self,
        handle: Handle,
    ) -> Result<(&'a mut [u8], &'a mut [u8]), StackError> {
        let index = self.connection_slots.resolve(handle)?;
        let buffers = self.connections.close(net_tcp::ConnectionId::new(index))?;
        self.connection_slots.retire(index);
        Ok(buffers)
    }
}

impl<const SOCKETS: usize, const CONNECTIONS: usize> Stack<'_, SOCKETS, CONNECTIONS> {
    /// The next identification field for an IPv4 datagram.
    pub(crate) const fn next_identification(&mut self) -> u16 {
        let value = self.identification;
        self.identification = self.identification.wrapping_add(1);
        value
    }

    /// Puts one frame in the outgoing queue and answers whether there was
    /// room.
    pub(crate) fn push_frame(
        &mut self,
        destination: MacAddr,
        ether_type: EtherType,
        payload: &[u8],
    ) -> bool {
        let mut frame = [0u8; FRAME_LEN];
        let mut writer = Writer::new(&mut frame);
        if net_eth::Frame::write(
            &mut writer,
            destination,
            self.config.hardware,
            ether_type,
            payload,
        )
        .is_err()
        {
            return false;
        }
        let len = writer.position();
        let bytes = frame.get(..len).unwrap_or(&[]);
        self.out.push(bytes)
    }

    /// Sends one datagram, through whichever of the two senders has a
    /// header for its family.
    ///
    /// A datagram to the limited broadcast address goes out without
    /// asking the routing table, because a client that has no address yet
    /// has no route either and that is exactly when DHCP has to send
    /// (RFC 2131, section 4.1).
    ///
    /// # Errors
    ///
    /// Whatever the sender of that family says.
    pub(crate) fn transmit(
        &mut self,
        source: IpAddr,
        destination: IpAddr,
        protocol: Protocol,
        payload: &[u8],
        now: Instant,
    ) -> Result<(), StackError> {
        match (source, destination) {
            (IpAddr::V4(_), IpAddr::V4(to)) if to.is_broadcast() => {
                self.broadcast(source, destination, protocol, payload)
            }
            (IpAddr::V4(_), IpAddr::V4(_)) => {
                self.transmit_v4(source, destination, protocol, payload, now)
            }
            (IpAddr::V6(from), IpAddr::V6(to)) => {
                self.transmit_v6(from, to, protocol, payload, now)
            }
            _ => Err(StackError::Wire(net_wire::WireError::MixedFamilies)),
        }
    }

    /// One IPv4 datagram, through the routing table and the cache.
    fn transmit_v4(
        &mut self,
        source: IpAddr,
        destination: IpAddr,
        protocol: Protocol,
        payload: &[u8],
        now: Instant,
    ) -> Result<(), StackError> {
        let outgoing = Outgoing {
            source,
            destination,
            protocol,
            identification: self.next_identification(),
            dont_fragment: false,
        };
        let interface = Interface {
            hardware: self.config.hardware,
            mtu: self.config.mtu,
        };
        let mut frame = [0u8; FRAME_LEN];
        let sent = {
            let out = &mut self.out;
            let mut sender = Sender {
                interface,
                routes: &self.routes,
                neighbors: &mut self.neighbors,
            };
            sender.send(outgoing, payload, now, &mut frame, |bytes| {
                out.push(bytes);
                Ok(())
            })?
        };
        if sent == Sent::Resolving {
            self.remember_held(
                destination,
                source,
                protocol,
                u32::from(outgoing.identification),
            );
        }
        Ok(())
    }

    /// Notes what a datagram that is waiting for a neighbor was being
    /// sent as, so that `send_pending` can send it when the answer comes.
    fn remember_held(
        &mut self,
        destination: IpAddr,
        source: IpAddr,
        protocol: Protocol,
        identification: u32,
    ) {
        let Ok(hop) = self.routes.lookup(destination) else {
            return;
        };
        let neighbor = hop.address();
        let held = Held {
            neighbor,
            source,
            destination,
            protocol,
            identification,
        };
        if let Some(at) = self
            .held
            .iter()
            .position(|entry| entry.neighbor == neighbor)
        {
            if let Some(slot) = self.held.get_mut(at) {
                *slot = held;
            }
            return;
        }
        if self.held.is_full() {
            self.held.remove(0);
        }
        let _ = self.held.push(held);
    }

    /// One IPv6 packet, through the routing table, the cache and the path
    /// estimates.
    fn transmit_v6(
        &mut self,
        source: net_wire::Ipv6Addr,
        destination: net_wire::Ipv6Addr,
        protocol: Protocol,
        payload: &[u8],
        now: Instant,
    ) -> Result<(), StackError> {
        let identification = self.identification6;
        self.identification6 = self.identification6.wrapping_add(1);
        let outgoing = net_ipv6::send::Outgoing {
            source,
            destination,
            protocol,
            identification,
        };
        let interface = Interface {
            hardware: self.config.hardware,
            mtu: self.config.mtu,
        };
        let mut frame = [0u8; FRAME_LEN];
        let sent = {
            let out = &mut self.out;
            let mut sender = net_ipv6::Sender {
                interface,
                routes: &self.routes,
                neighbors: &mut self.neighbors,
                path: &self.path,
            };
            sender.send(outgoing, payload, now, &mut frame, |bytes| {
                out.push(bytes);
                Ok(())
            })?
        };
        if sent == Sent::Resolving {
            self.remember_held(
                IpAddr::V6(destination),
                IpAddr::V6(source),
                protocol,
                identification,
            );
        }
        Ok(())
    }

    /// One Neighbor Discovery message, straight onto the link.
    ///
    /// These do not go through either sender, and the reason is the hop
    /// limit. RFC 4861, section 7.1 requires 255 and that is the whole of
    /// what makes the protocol link-local; a sender that writes the
    /// default would produce a message every receiver is right to throw
    /// away. Nor do they need one: a discovery message goes to a
    /// link-local address or to a multicast group whose hardware address
    /// RFC 2464 derives, so there is no route to look up and no neighbor
    /// to resolve — which is fortunate, because resolving one is what
    /// they are for.
    pub(crate) fn send_discovery(
        &mut self,
        source: net_wire::Ipv6Addr,
        destination: net_wire::Ipv6Addr,
        message: &[u8],
    ) {
        let hardware = if destination.is_multicast() {
            net_eth::multicast_hardware(destination)
        } else {
            let Some(hardware) = self.neighbors.hardware(IpAddr::V6(destination)) else {
                return;
            };
            hardware
        };
        let mut header =
            net_ipv6::Header::new(source, destination, Protocol::ICMPV6, message.len());
        header.hop_limit = DISCOVERY_HOP_LIMIT;
        let mut packet = [0u8; PAYLOAD_LEN];
        let mut writer = Writer::new(&mut packet);
        // Forty bytes of header and a message under fifty, into a buffer
        // of an MTU: neither of these can fail, and a short write leaves
        // a frame the receiver drops.
        let _ = header.write(&mut writer);
        let _ = writer.write_bytes(message);
        let len = writer.position();
        let bytes = packet.get(..len).unwrap_or(&[]);
        self.push_frame(hardware, EtherType::IPV6, bytes);
    }

    /// One IPv4 datagram to the whole link, with no route consulted.
    fn broadcast(
        &mut self,
        source: IpAddr,
        destination: IpAddr,
        protocol: Protocol,
        payload: &[u8],
    ) -> Result<(), StackError> {
        let (IpAddr::V4(from), IpAddr::V4(to)) = (source, destination) else {
            return Err(StackError::Wire(net_wire::WireError::MixedFamilies));
        };
        let mut header = Header::new(from, to, protocol, payload.len());
        header.identification = self.next_identification();
        let mut datagram = [0u8; MTU];
        let mut writer = Writer::new(&mut datagram);
        header.write(&mut writer)?;
        writer.write_bytes(payload)?;
        let len = writer.position();
        let bytes = datagram.get(..len).unwrap_or(&[]);
        self.push_frame(MacAddr::BROADCAST, EtherType::IPV4, bytes);
        Ok(())
    }

    /// The datagram at the front of the outgoing queue, without taking it
    /// out.
    #[must_use]
    pub fn peek(&self) -> Option<&[u8]> {
        self.out.peek()
    }

    /// How many frames are waiting to go out.
    #[must_use]
    pub const fn pending(&self) -> usize {
        self.out.len()
    }
}

/// How long an Ethernet header is, for a caller sizing a buffer.
pub const HEADER_LEN: usize = ETH_HEADER_LEN;

impl<const SOCKETS: usize, const CONNECTIONS: usize> Stack<'_, SOCKETS, CONNECTIONS> {
    /// Starts asking for an address.
    ///
    /// Nothing happens on its own: a host that is given its address by
    /// hand never sends a discover, and one that is not says so here.
    pub const fn configure(&mut self, now: Instant) {
        self.dhcp.start(now);
    }

    /// Where the address configuration client stands.
    #[must_use]
    pub const fn configuration(&self) -> net_dhcp::State {
        self.dhcp.state()
    }

    /// The lease, once there is one.
    #[must_use]
    pub const fn lease(&self) -> Option<&Lease> {
        self.dhcp.lease()
    }

    /// The name servers this host knows: those a lease named, and those a
    /// router advertisement named.
    pub fn name_servers(&self) -> impl Iterator<Item = IpAddr> + '_ {
        let leased = self
            .dhcp
            .lease()
            .into_iter()
            .flat_map(|lease| lease.servers.iter().copied().map(IpAddr::V4));
        let advertised = self
            .slaac
            .servers()
            .map(|server| IpAddr::V6(server.address));
        leased.chain(advertised)
    }

    /// The name being resolved, or `None` when nothing is.
    #[must_use]
    pub const fn resolving(&self) -> bool {
        self.resolver.is_some()
    }
}
