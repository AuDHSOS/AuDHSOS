// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The connection table: which of a fixed number of connections a segment
//! belongs to, and what happens when it belongs to none.
//!
//! A connection is named by four things — the two addresses and the two
//! ports — and a segment finds it by all four. A segment that matches none
//! of them is offered to a connection listening on its destination port,
//! and that is what a passive open is: the `SYN` arrives, the listening
//! connection takes the sender's address and port, and it is that
//! connection from then on.
//!
//! A segment that matches nothing at all is answered with a reset. That
//! belongs here and not in the layer above: RFC 9293, section 3.10.7.1
//! calls it the processing of state `CLOSED`, and which numbers the reset
//! carries is decided by the segment's own header fields, which nothing
//! above this crate reads.

use audhsos_time::Instant;
use crypto_rng::Rng;
use net_wire::{IpAddr, Port};

use crate::connection::{Config, Connection, Endpoint};
use crate::error::TcpError;
use crate::segment::{Segment, reset_for};
use crate::state::State;

/// Which connection of a table. It is the index and nothing more; the
/// facade above this crate is where a handle grows a generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnectionId(usize);

impl ConnectionId {
    /// The connection at `index`.
    #[must_use]
    pub const fn new(index: usize) -> ConnectionId {
        ConnectionId(index)
    }

    /// The index in the table.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// What became of an arriving segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery<'a> {
    /// This connection took it. Whatever it owes in answer comes out of
    /// that connection's `poll`.
    Delivered(ConnectionId),
    /// Nothing holds the four-tuple and nothing listens on the port. The
    /// segment carried here is the reset to send back.
    Refused(Segment<'a>),
    /// Nothing holds it, and the segment was itself a reset, which is
    /// never answered with another.
    Dropped,
    /// The bytes are not a segment, and nothing was delivered.
    Malformed(TcpError),
}

/// A fixed number of connections.
#[derive(Debug)]
pub struct Connections<'a, const N: usize> {
    /// The table. An entry is `None` while nothing holds it.
    entries: [Option<Connection<'a>>; N],
}

impl<'a, const N: usize> Default for Connections<'a, N> {
    fn default() -> Connections<'a, N> {
        Connections::new()
    }
}

impl<'a, const N: usize> Connections<'a, N> {
    /// An empty table.
    #[must_use]
    pub const fn new() -> Connections<'a, N> {
        Connections {
            entries: [const { None }; N],
        }
    }

    /// How many connections are open.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    /// Whether none are.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The connection `id` names.
    #[must_use]
    pub fn get(&self, id: ConnectionId) -> Option<&Connection<'a>> {
        self.entries.get(id.index())?.as_ref()
    }

    /// The connection `id` names, to read from, write to, or poll.
    pub fn get_mut(&mut self, id: ConnectionId) -> Option<&mut Connection<'a>> {
        self.entries.get_mut(id.index())?.as_mut()
    }

    /// Every open connection, to poll in turn.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (ConnectionId, &mut Connection<'a>)> {
        self.entries
            .iter_mut()
            .enumerate()
            .filter_map(|(index, slot)| Some((ConnectionId(index), slot.as_mut()?)))
    }

    /// The earliest instant at which any connection has work.
    #[must_use]
    pub fn poll_at(&self, now: Instant) -> Option<Instant> {
        self.entries
            .iter()
            .flatten()
            .filter_map(|connection| connection.poll_at(now))
            .min()
    }

    /// Opens a connection that waits for a peer on `local`.
    ///
    /// # Errors
    ///
    /// [`TcpError::PortInUse`] when something already listens there,
    /// [`TcpError::NoConnection`] when the table is full, and
    /// [`TcpError::Rng`] when the generator fails.
    pub fn listen<R: Rng + ?Sized>(
        &mut self,
        local: Endpoint,
        config: Config,
        rng: &mut R,
        send: &'a mut [u8],
        recv: &'a mut [u8],
    ) -> Result<ConnectionId, TcpError> {
        if self.listens_on(local.port) {
            return Err(TcpError::PortInUse(local.port));
        }
        let (id, connection) = self.place(local, config, send, recv)?;
        connection.listen(rng)?;
        Ok(id)
    }

    /// Opens a connection from `local` to `remote`.
    ///
    /// # Errors
    ///
    /// [`TcpError::PortInUse`] when the four-tuple is already held,
    /// [`TcpError::NoConnection`] when the table is full,
    /// [`TcpError::MixedFamilies`] when the two ends are of different
    /// families, and [`TcpError::Rng`] when the generator fails.
    pub fn connect<R: Rng + ?Sized>(
        &mut self,
        local: Endpoint,
        remote: Endpoint,
        config: Config,
        rng: &mut R,
        send: &'a mut [u8],
        recv: &'a mut [u8],
    ) -> Result<ConnectionId, TcpError> {
        if self.holds(local, remote) {
            return Err(TcpError::PortInUse(local.port));
        }
        let (id, connection) = self.place(local, config, send, recv)?;
        connection.connect(remote, rng)?;
        Ok(id)
    }

    /// Closes the connection `id` names at once and gives its two buffers
    /// back. A connection that was still open sends a reset first, which
    /// the caller fetches with one last `poll` before it takes the
    /// buffers.
    ///
    /// # Errors
    ///
    /// [`TcpError::UnknownConnection`] when `id` names no open connection.
    pub fn close(&mut self, id: ConnectionId) -> Result<(&'a mut [u8], &'a mut [u8]), TcpError> {
        let Some(slot) = self.entries.get_mut(id.index()) else {
            return Err(TcpError::UnknownConnection);
        };
        let Some(connection) = slot.take() else {
            return Err(TcpError::UnknownConnection);
        };
        Ok(connection.into_buffers())
    }

    /// Reads the segment in `bytes`, which arrived from `source` and was
    /// addressed to `destination`, and hands it to the connection it
    /// belongs to.
    pub fn receive<'b>(
        &mut self,
        source: IpAddr,
        destination: IpAddr,
        bytes: &'b [u8],
        now: Instant,
    ) -> Delivery<'b> {
        let segment = match Segment::parse(bytes, source, destination) {
            Ok(segment) => segment,
            Err(error) => return Delivery::Malformed(error),
        };
        let local = Endpoint::new(destination, segment.destination_port);
        let remote = Endpoint::new(source, segment.source_port);
        let Some((id, connection)) = self.lookup(local, remote) else {
            return match reset_for(&segment) {
                Some(reset) => Delivery::Refused(reset),
                None => Delivery::Dropped,
            };
        };
        connection.on_segment(source, &segment, now);
        Delivery::Delivered(id)
    }

    /// Whether a connection already joins these two ends.
    ///
    /// One that has been closed and not yet taken out of the table holds
    /// nothing: its numbers are spent and its buffers are waiting to be
    /// given back, so the pair of ends is free again.
    fn holds(&self, local: Endpoint, remote: Endpoint) -> bool {
        self.entries
            .iter()
            .flatten()
            .any(|connection| joins(connection, local, remote))
    }

    /// Whether a connection is listening on `port`.
    fn listens_on(&self, port: Port) -> bool {
        self.entries.iter().flatten().any(|connection| {
            connection.state() == State::Listen && connection.local().port == port
        })
    }

    /// Puts a fresh connection in the first free slot.
    fn place(
        &mut self,
        local: Endpoint,
        config: Config,
        send: &'a mut [u8],
        recv: &'a mut [u8],
    ) -> Result<(ConnectionId, &mut Connection<'a>), TcpError> {
        let Some((index, slot)) = self
            .entries
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none())
        else {
            return Err(TcpError::NoConnection);
        };
        let connection = slot.insert(Connection::new(local, config, send, recv));
        Ok((ConnectionId(index), connection))
    }

    /// The connection a segment between these two ends belongs to: the one
    /// that joins them, or failing that the one listening on the local
    /// port.
    fn lookup(
        &mut self,
        local: Endpoint,
        remote: Endpoint,
    ) -> Option<(ConnectionId, &mut Connection<'a>)> {
        let exact = self.entries.iter().position(|slot| {
            slot.as_ref()
                .is_some_and(|connection| joins(connection, local, remote))
        });
        let index = match exact {
            Some(index) => index,
            None => self.entries.iter().position(|slot| {
                slot.as_ref().is_some_and(|connection| {
                    connection.state() == State::Listen && connection.local() == local
                })
            })?,
        };
        let connection = self.entries.get_mut(index)?.as_mut()?;
        Some((ConnectionId(index), connection))
    }
}

/// Whether `connection` is the open one joining these two ends.
///
/// A connection that has been closed is not: a segment that reaches it is
/// a segment for a connection that no longer exists, and RFC 9293,
/// section 3.10.7.1 answers that with a reset rather than with silence.
fn joins(connection: &Connection<'_>, local: Endpoint, remote: Endpoint) -> bool {
    connection.state().is_open()
        && connection.state() != State::Listen
        && connection.local() == local
        && connection.remote() == remote
}
