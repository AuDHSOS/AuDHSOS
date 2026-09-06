// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Names, and the connection that is made to one.
//!
//! Two things live here and they are deliberately apart. Trying a list of
//! addresses one after another is a loop over `connect`, and it needs no
//! resolver: a caller that got its addresses somewhere else uses it as it
//! stands. Turning a name into that list is the resolver and the ordering
//! of RFC 6724, and it ends by handing the ordered list to the same loop.
//!
//! They are not raced. RFC 8305 would open a connection to each family a
//! moment apart and keep whichever answers first, and that is a policy
//! about how much of somebody's network to use for a guess; it belongs
//! above a stack and not in one. What its absence costs is one timeout on
//! a path that is broken in a way the routing table does not know about.
//!
//! The socket the resolver sends from is one the caller bound. That is
//! the same rule as everywhere else here — buffers belong to the caller —
//! and it is what makes the source port the ephemeral port `net-udp`
//! drew, which is where the randomness of RFC 5452 comes from (D-86).

use audhsos_time::Instant;
use crypto_rng::Rng;
use net_dns::{Name, Resolver, Status};
use net_wire::{IpAddr, Port};

use crate::error::StackError;
use crate::handle::Handle;
use crate::stack::{Attempt, PAYLOAD_LEN, Stack};

/// Where a connection to a name or to a list of addresses stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connecting {
    /// Nothing is being connected.
    Idle,
    /// A name is being resolved, or a candidate is being tried.
    Trying,
    /// The connection is open.
    Open(Handle),
    /// Every candidate was tried and none answered.
    Failed,
}

impl<'a, const SOCKETS: usize, const CONNECTIONS: usize> Stack<'a, SOCKETS, CONNECTIONS> {
    /// Starts resolving `name`, sending from the socket `socket` names.
    ///
    /// # Errors
    ///
    /// [`StackError::Busy`] when a resolution is running,
    /// [`StackError::Unresolved`] when this host knows no name server,
    /// [`StackError::NoAddress`] when none of them can be reached, and
    /// whatever the handle or the resolver says.
    pub fn resolve(&mut self, name: &Name, socket: Handle, now: Instant) -> Result<(), StackError> {
        if self.resolver.is_some() {
            return Err(StackError::Busy);
        }
        let port = self.socket(socket)?.port();
        let server = self.name_servers().next().ok_or(StackError::Unresolved)?;
        let local = self.source_for(server).ok_or(StackError::NoAddress)?;
        let mut resolver = Resolver::new(local, port, self.config.resolver);
        for server in self.name_servers() {
            if server.version() == local.version() {
                let _ = resolver.add_server(server);
            }
        }
        resolver.start(name, now)?;
        self.resolver = Some(resolver);
        self.resolver_socket = Some(socket);
        Ok(())
    }

    /// Where the resolution stands.
    #[must_use]
    pub fn resolution(&self) -> Option<Status> {
        self.resolver.as_ref().map(Resolver::status)
    }

    /// The addresses the resolution found, in the order they arrived.
    pub fn resolved(&self) -> impl Iterator<Item = IpAddr> + '_ {
        self.resolver.iter().flat_map(Resolver::addresses)
    }

    /// Forgets the resolution, so that the next one starts clean.
    pub const fn forget_resolution(&mut self) {
        self.resolver = None;
        self.resolver_socket = None;
    }

    /// Reads what the resolver's socket took in, and writes what the
    /// resolver wants sent.
    pub(crate) fn drive_resolver<R: Rng + ?Sized>(&mut self, now: Instant, rng: &mut R) {
        // The socket and the resolver are set together and forgotten
        // together, so one of them standing for both is not a shortcut.
        let Some(socket) = self.resolver_socket else {
            return;
        };
        let mut payload = [0u8; PAYLOAD_LEN];
        while let Some((source, port, len)) = self.take_datagram(socket, &mut payload) {
            let bytes = payload.get(..len).unwrap_or(&[]);
            if let Some(resolver) = self.resolver.as_mut() {
                resolver.on_datagram(source, port, bytes, now);
            }
        }
        let mut buffer = [0u8; PAYLOAD_LEN];
        let query = self
            .resolver
            .as_mut()
            .and_then(|resolver| resolver.poll(now, rng, &mut buffer).ok().flatten());
        if let Some(query) = query {
            let (source, destination) = (query.source, query.destination);
            let _ = self.transmit(
                source,
                destination,
                net_wire::Protocol::UDP,
                query.datagram,
                now,
            );
        }
    }

    /// Takes the oldest datagram out of the socket `handle` names and
    /// copies its payload into `out`.
    fn take_datagram(&mut self, handle: Handle, out: &mut [u8]) -> Option<(IpAddr, Port, usize)> {
        let socket = self.socket(handle).ok()?;
        let received = socket.peek()?;
        let len = received.payload.len();
        let (source, port) = (received.source, received.port);
        let copied = match out.get_mut(..len) {
            Some(slot) => {
                slot.copy_from_slice(received.payload);
                true
            }
            None => false,
        };
        socket.discard();
        copied.then_some((source, port, len))
    }

    /// Opens a connection to the first of `candidates` that answers,
    /// trying them in the order they are given.
    ///
    /// The order is the caller's. [`Stack::order`] is what puts a list in
    /// the order of RFC 6724, and [`connect_to_name`](Stack::connect_to_name)
    /// is what does both at once.
    ///
    /// # Errors
    ///
    /// [`StackError::Busy`] when a connection is already being made, and
    /// [`StackError::Unresolved`] for an empty list.
    pub fn connect_to_any<R: Rng + ?Sized>(
        &mut self,
        candidates: &[IpAddr],
        port: Port,
        rng: &mut R,
        send: &'a mut [u8],
        receive: &'a mut [u8],
        now: Instant,
    ) -> Result<(), StackError> {
        if self.attempt.is_some() {
            return Err(StackError::Busy);
        }
        if candidates.is_empty() {
            return Err(StackError::Unresolved);
        }
        let mut held = audhsos_collections::ArrayVec::new();
        for address in candidates {
            let _ = held.push(*address);
        }
        self.attempt = Some(Attempt {
            candidates: held,
            at: 0,
            port,
            buffers: Some((send, receive)),
            handle: None,
            resolving: false,
        });
        self.drive_attempt(now, rng)
    }

    /// Resolves `name` and opens a connection to the first of its
    /// addresses that answers, in the order of RFC 6724.
    ///
    /// # Errors
    ///
    /// Whatever [`resolve`](Stack::resolve) says, and
    /// [`StackError::Busy`] when a connection is already being made.
    pub fn connect_to_name(
        &mut self,
        name: &Name,
        socket: Handle,
        port: Port,
        send: &'a mut [u8],
        receive: &'a mut [u8],
        now: Instant,
    ) -> Result<(), StackError> {
        if self.attempt.is_some() {
            return Err(StackError::Busy);
        }
        self.resolve(name, socket, now)?;
        self.attempt = Some(Attempt {
            candidates: audhsos_collections::ArrayVec::new(),
            at: 0,
            port,
            buffers: Some((send, receive)),
            handle: None,
            resolving: true,
        });
        Ok(())
    }

    /// Where the connection being made stands.
    #[must_use]
    pub const fn connecting(&self) -> Connecting {
        let Some(attempt) = self.attempt.as_ref() else {
            return Connecting::Idle;
        };
        match attempt.handle {
            Some(handle) => Connecting::Open(handle),
            None if attempt.resolving || attempt.at < attempt.candidates.len() => {
                Connecting::Trying
            }
            None => Connecting::Failed,
        }
    }

    /// Takes the open connection out, so that the next one can be made.
    pub fn take_connection(&mut self) -> Option<Handle> {
        let handle = self.attempt.as_ref()?.handle?;
        self.attempt = None;
        Some(handle)
    }

    /// Gives up on the connection being made and hands the buffers back.
    pub fn abandon(&mut self) -> Option<(&'a mut [u8], &'a mut [u8])> {
        let mut attempt = self.attempt.take()?;
        let buffers = attempt.buffers.take();
        if let Some(handle) = attempt.handle {
            return self.close_connection(handle).ok();
        }
        buffers
    }

    /// Moves the connection being made along: the next candidate when the
    /// last one did not answer, and the resolver's answers when it has
    /// them.
    pub(crate) fn drive_attempt<R: Rng + ?Sized>(
        &mut self,
        now: Instant,
        rng: &mut R,
    ) -> Result<(), StackError> {
        if self
            .attempt
            .as_ref()
            .is_some_and(|attempt| attempt.resolving)
        {
            match self.resolution() {
                Some(Status::Asking | Status::Idle) => return Ok(()),
                Some(Status::Done) => self.take_resolved(),
                Some(Status::Failed(_)) | None => {
                    if let Some(attempt) = self.attempt.as_mut() {
                        attempt.resolving = false;
                        attempt.candidates.clear();
                    }
                    self.forget_resolution();
                }
            }
        }
        self.retire_dead_candidate()?;
        self.try_next_candidate(now, rng)
    }

    /// Puts the resolver's answers in the candidate list, in the order of
    /// RFC 6724.
    fn take_resolved(&mut self) {
        // The resolver holds at most `MAX_ADDRESSES`, so zipping the
        // answers onto an array of that size takes all of them and the
        // count says how many.
        let mut found = [IpAddr::V4(net_wire::Ipv4Addr::UNSPECIFIED); net_dns::MAX_ADDRESSES];
        let mut count = 0usize;
        for (slot, address) in found.iter_mut().zip(self.resolved()) {
            *slot = address;
            count = count.saturating_add(1);
        }
        let addresses = found.get_mut(..count).unwrap_or(&mut []);
        self.order(addresses);
        let ordered = found.get(..count).unwrap_or(&[]);
        if let Some(attempt) = self.attempt.as_mut() {
            attempt.candidates.clear();
            for address in ordered {
                let _ = attempt.candidates.push(*address);
            }
            attempt.resolving = false;
        }
        self.forget_resolution();
    }

    /// Closes a candidate that will not answer, so that the next one can
    /// be tried over the same buffers.
    fn retire_dead_candidate(&mut self) -> Result<(), StackError> {
        let Some(handle) = self.attempt.as_ref().and_then(|attempt| attempt.handle) else {
            return Ok(());
        };
        let dead = {
            let connection = self.connection(handle)?;
            connection.timed_out() || connection.was_reset()
        };
        if !dead {
            return Ok(());
        }
        let buffers = self.close_connection(handle)?;
        if let Some(attempt) = self.attempt.as_mut() {
            attempt.handle = None;
            attempt.buffers = Some(buffers);
        }
        Ok(())
    }

    /// Opens a connection to the next candidate, when there is one and
    /// nothing is open.
    fn try_next_candidate<R: Rng + ?Sized>(
        &mut self,
        now: Instant,
        rng: &mut R,
    ) -> Result<(), StackError> {
        let Some(attempt) = self.attempt.as_mut() else {
            return Ok(());
        };
        if attempt.handle.is_some() || attempt.resolving {
            return Ok(());
        }
        let Some(address) = attempt.candidates.get(attempt.at).copied() else {
            // Every candidate has been tried. The buffers stay where they
            // are until the caller takes them back.
            return Ok(());
        };
        attempt.at = attempt.at.saturating_add(1);
        let Some((send, receive)) = attempt.buffers.take() else {
            return Ok(());
        };
        let port = attempt.port;
        let _ = now;
        match self.connect_to(address, port, rng, send, receive) {
            Ok(handle) => {
                if let Some(attempt) = self.attempt.as_mut() {
                    attempt.handle = Some(handle);
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}
