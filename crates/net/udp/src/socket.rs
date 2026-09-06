// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The socket table: a fixed number of sockets, the ports they hold, and
//! the way an arriving datagram finds one.
//!
//! A socket is bound to a port and either to one of this host's addresses
//! or to none of them. Bound to none, it takes every datagram that reaches
//! that port, including those addressed to a broadcast or a multicast
//! group, which is what a DHCP client needs before it has an address of
//! its own. Bound to one, it takes only what was addressed to exactly that
//! address, and a datagram that names another one is offered to the
//! wildcard socket instead.
//!
//! A port is held once, whichever address holds it. Two sockets on one
//! port with different local addresses is a distinction this system has no
//! use for, and one that makes the question of which of them a broadcast
//! belongs to a matter of precedence rules rather than of a lookup.
//!
//! Nothing is sent from here. A datagram for a port nobody holds is
//! reported as such and the layer below decides whether an ICMP error may
//! go back: the restrictions of RFC 1122, section 3.2.2 are about the
//! datagram's addresses and its fragment offset, which are fields of a
//! header this crate does not read.

use crypto_rng::Rng;
use net_wire::{IpAddr, Port};

use crate::datagram::Datagram;
use crate::error::UdpError;
use crate::ring::{DropReason, Received, Ring};

/// The dynamic range of RFC 6335 is `0xC000` to `0xFFFF`, which is exactly
/// `2^14` ports. The low fourteen bits of two random bytes therefore name
/// one of them with no bias, where a remainder over a range that is not a
/// power of two would favour its first ports.
const EPHEMERAL_MASK: u16 = 0x3FFF;

/// How often a port is drawn before the search for a free one gives up.
///
/// RFC 6056, section 3.3.1 draws again on a collision rather than walking
/// to the next port, because a port beside a taken one is a port an
/// observer who saw the first can guess. Drawing again has to be bounded,
/// and this is the bound. With a table of a few sockets among sixteen
/// thousand ports, eight draws that all collide is not a case that occurs;
/// it is a case that must have an answer.
const EPHEMERAL_ATTEMPTS: usize = 8;

/// Which socket of a table. It is the index and nothing more; the facade
/// above this crate is where a handle grows a generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SocketId(usize);

impl SocketId {
    /// The socket at `index`. A facade that keeps handles of its own
    /// rebuilds one with this; an index the table does not have is
    /// reported by the call that uses it and not here.
    #[must_use]
    pub const fn new(index: usize) -> SocketId {
        SocketId(index)
    }

    /// The index in the table.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// What became of an arriving datagram.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Delivery {
    /// It is in the ring of this socket.
    Delivered(SocketId),
    /// A socket holds the port and the ring would not take the datagram.
    Dropped {
        /// Whose ring it was.
        socket: SocketId,
        /// Why it did not go in.
        reason: DropReason,
    },
    /// No socket holds the port. The layer below decides whether an ICMP
    /// destination-unreachable message with the port code may go back.
    PortUnreachable,
    /// The datagram is not one, and nothing was delivered.
    Malformed(UdpError),
}

/// One open socket.
#[derive(Debug)]
pub struct Socket<'a> {
    /// The port it holds.
    port: Port,
    /// The address it holds, or `None` for every address of this host.
    local: Option<IpAddr>,
    /// What has arrived and not been read.
    ring: Ring<'a>,
}

impl Socket<'_> {
    /// The port it holds.
    #[must_use]
    pub const fn port(&self) -> Port {
        self.port
    }

    /// The address it holds, or `None` when it holds every one of them.
    #[must_use]
    pub const fn local(&self) -> Option<IpAddr> {
        self.local
    }

    /// How many datagrams are waiting.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.ring.len()
    }

    /// Whether none are.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// How many datagrams the ring refused since the socket was bound.
    #[must_use]
    pub const fn dropped(&self) -> u32 {
        self.ring.dropped()
    }

    /// The oldest datagram, without taking it out.
    #[must_use]
    pub fn peek(&self) -> Option<Received<'_>> {
        self.ring.peek()
    }

    /// Drops the oldest datagram and answers whether there was one.
    pub fn discard(&mut self) -> bool {
        self.ring.discard()
    }
}

/// A fixed number of sockets and the ports they hold.
#[derive(Debug)]
pub struct Sockets<'a, const N: usize> {
    /// The table. An entry is `None` while nothing holds it.
    entries: [Option<Socket<'a>>; N],
}

impl<'a, const N: usize> Default for Sockets<'a, N> {
    fn default() -> Sockets<'a, N> {
        Sockets::new()
    }
}

impl<'a, const N: usize> Sockets<'a, N> {
    /// An empty table.
    #[must_use]
    pub const fn new() -> Sockets<'a, N> {
        Sockets {
            entries: [const { None }; N],
        }
    }

    /// How many sockets are open.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    /// Whether none are.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether a socket holds `port`.
    #[must_use]
    pub fn is_bound(&self, port: Port) -> bool {
        self.entries
            .iter()
            .flatten()
            .any(|socket| socket.port == port)
    }

    /// The socket `id` names.
    #[must_use]
    pub fn get(&self, id: SocketId) -> Option<&Socket<'a>> {
        self.entries.get(id.0)?.as_ref()
    }

    /// The socket `id` names, to read datagrams out of.
    pub fn get_mut(&mut self, id: SocketId) -> Option<&mut Socket<'a>> {
        self.entries.get_mut(id.0)?.as_mut()
    }

    /// Opens a socket on `port`, receiving into `buffer`.
    ///
    /// `local` is one of this host's addresses, or `None` for every one of
    /// them.
    ///
    /// # Errors
    ///
    /// [`UdpError::UnspecifiedPort`] for port zero,
    /// [`UdpError::PortInUse`] when a socket already holds the port, and
    /// [`UdpError::NoSocket`] when the table is full.
    pub fn bind(
        &mut self,
        local: Option<IpAddr>,
        port: Port,
        buffer: &'a mut [u8],
    ) -> Result<SocketId, UdpError> {
        if port.is_unspecified() {
            return Err(UdpError::UnspecifiedPort);
        }
        if self.is_bound(port) {
            return Err(UdpError::PortInUse(port));
        }
        let Some((index, slot)) = self
            .entries
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        else {
            return Err(UdpError::NoSocket);
        };
        *slot = Some(Socket {
            port,
            local,
            ring: Ring::new(buffer),
        });
        Ok(SocketId(index))
    }

    /// Opens a socket on a port drawn from the dynamic range of RFC 6335.
    ///
    /// # Errors
    ///
    /// [`UdpError::Rng`] when the generator fails, [`UdpError::NoPort`]
    /// when every port the search visited is taken, and whatever
    /// [`bind`](Sockets::bind) returns.
    pub fn bind_ephemeral<R: Rng + ?Sized>(
        &mut self,
        local: Option<IpAddr>,
        rng: &mut R,
        buffer: &'a mut [u8],
    ) -> Result<SocketId, UdpError> {
        let port = self.ephemeral_port(rng)?;
        self.bind(local, port, buffer)
    }

    /// Closes the socket `id` names and gives its memory back.
    ///
    /// # Errors
    ///
    /// [`UdpError::UnknownSocket`] when `id` names no open socket.
    pub fn close(&mut self, id: SocketId) -> Result<&'a mut [u8], UdpError> {
        let Some(slot) = self.entries.get_mut(id.0) else {
            return Err(UdpError::UnknownSocket);
        };
        let Some(socket) = slot.take() else {
            return Err(UdpError::UnknownSocket);
        };
        Ok(socket.ring.into_bytes())
    }

    /// Reads the datagram in `bytes`, which arrived from `source` and was
    /// addressed to `destination`, and puts it in the ring of the socket
    /// that holds its port.
    pub fn receive(&mut self, source: IpAddr, destination: IpAddr, bytes: &[u8]) -> Delivery {
        let datagram = match Datagram::parse(bytes, source, destination) {
            Ok(datagram) => datagram,
            Err(error) => return Delivery::Malformed(error),
        };
        let Some((index, socket)) = self.lookup(datagram.destination_port, destination) else {
            return Delivery::PortUnreachable;
        };
        match socket
            .ring
            .push(source, destination, datagram.source_port, datagram.payload)
        {
            Ok(()) => Delivery::Delivered(SocketId(index)),
            Err(reason) => Delivery::Dropped {
                socket: SocketId(index),
                reason,
            },
        }
    }

    /// Which socket takes a datagram to `port` at `destination`: the one
    /// that holds the port, when the address it is bound to is that one or
    /// none at all. A port is held once, so there is never a second
    /// candidate to prefer this one over.
    fn lookup(&mut self, port: Port, destination: IpAddr) -> Option<(usize, &mut Socket<'a>)> {
        self.entries
            .iter_mut()
            .enumerate()
            .find_map(|(index, slot)| {
                let socket = slot.as_mut()?;
                (socket.port == port && socket.local.is_none_or(|local| local == destination))
                    .then_some((index, socket))
            })
    }

    /// A free port of the dynamic range, drawn at random.
    fn ephemeral_port<R: Rng + ?Sized>(&self, rng: &mut R) -> Result<Port, UdpError> {
        for _ in 0..EPHEMERAL_ATTEMPTS {
            let mut seed = [0u8; 2];
            rng.fill(&mut seed)?;
            let offset = u16::from_be_bytes(seed) & EPHEMERAL_MASK;
            let port = Port::new(Port::EPHEMERAL_FIRST.get() | offset);
            if !self.is_bound(port) {
                return Ok(port);
            }
        }
        Err(UdpError::NoPort)
    }
}
