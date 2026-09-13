// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The server: one stack, one socket table, and the two rings of every
//! socket.
//!
//! Every request is answered at once. A connection that is not open yet, an
//! accept with nothing to take, and a resolution that is still running each
//! answer [`Error::WouldBlock`], and the client asks again (D-142).
//!
//! Invariants: a socket slot and the rings of that slot are handed out
//! together and taken back together, and the rings are emptied before they
//! go; no byte is moved out of a connection that the inbound ring has no
//! room for, which is what makes the window stop advancing; and no buffer
//! leaves the pool for a call of the stack that can refuse it, because a
//! buffer travels into that call and a refusal does not bring it back —
//! the port is asked about first, and a pool that still holds a buffer is
//! a stack that still has a table slot, the two having the same count.

use audhsos_abi::Error;
use audhsos_abi::Handle as RawHandle;
use audhsos_time::Instant;
use crypto_rng::Rng;
use net_dns::{Name as DnsName, Status};
use net_stack::{Config, Handle, Stack, StackError};
use net_tcp::State as TcpState;
use net_wire::{IpAddr, Port};
use user_proto::ring::SocketPage;
use user_proto::socket::{
    Addresses, DATAGRAM_HEADER_LEN, Direction, Endpoint, Interface, Name, Opened, Reply, Request,
    State, datagram_header,
};

use crate::memory::{CONNECTIONS, Pool, SOCKETS};
use crate::sockets::{Entry, Kind, MAX_SOCKETS, Sockets};

/// The rings of one socket: the page the server writes and the object the
/// client is given to map it.
#[derive(Clone, Copy, Debug)]
pub struct Rings<'a> {
    /// The two rings.
    pub page: &'a SocketPage,
    /// The memory object over them, which travels in the reply.
    pub object: RawHandle,
}

/// How many bytes one round moves between a ring and a connection.
const CHUNK: usize = 1024;

/// The longest datagram this server sends in one call: one IPv4 datagram
/// over a link of 1500 bytes, less the twenty bytes of the header and the
/// eight of UDP, which is what leaves no fragment.
const DATAGRAM: usize = 1472;

/// The network server.
#[derive(Debug)]
pub struct Server<'a> {
    /// The stack under it.
    stack: Stack<'a, SOCKETS, CONNECTIONS>,
    /// The buffers it hands out.
    pool: Pool<'a>,
    /// The rings of each slot.
    rings: [Rings<'a>; MAX_SOCKETS],
    /// Which slot is whose.
    sockets: Sockets,
    /// The client whose name is being resolved, the socket it is being
    /// resolved through, and the name itself.
    resolving: Option<(u64, Handle, Name)>,
}

impl<'a> Server<'a> {
    /// A server over `bytes`, with `rings` for its sockets.
    ///
    /// Answers `None` for a region shorter than
    /// [`REGION_BYTES`](crate::memory::REGION_BYTES).
    #[must_use]
    pub fn new(
        config: Config,
        bytes: &'a mut [u8],
        rings: [Rings<'a>; MAX_SOCKETS],
    ) -> Option<Server<'a>> {
        let (outgoing, pool) = Pool::split(bytes)?;
        Some(Server {
            stack: Stack::new(config, outgoing),
            pool,
            rings,
            sockets: Sockets::new(),
            resolving: None,
        })
    }

    /// The stack, for a caller that configures it or reports on it.
    pub const fn stack(&mut self) -> &mut Stack<'a, SOCKETS, CONNECTIONS> {
        &mut self.stack
    }

    /// How many sockets are open.
    #[must_use]
    pub fn open(&self) -> usize {
        self.sockets.len()
    }

    /// Takes in at most one frame and hands out at most one, moving what
    /// the connections and the rings hold in both directions first.
    ///
    /// # Errors
    ///
    /// Whatever the stack said.
    pub fn poll<'t, R: Rng + ?Sized>(
        &mut self,
        now: Instant,
        rx: Option<&[u8]>,
        tx: &'t mut [u8],
        rng: &mut R,
    ) -> Result<Option<&'t [u8]>, StackError> {
        self.pump();
        self.stack.poll(now, rx, tx, rng)
    }

    /// When the stack next has something to say.
    #[must_use]
    pub fn poll_at(&self, now: Instant) -> Option<Instant> {
        self.stack.poll_at(now)
    }

    /// Moves bytes between every open socket and its rings.
    ///
    /// What a connection holds goes into the inbound ring as far as the
    /// ring has room; what the client put into the outbound ring goes into
    /// the connection as far as its window allows. A ring with no room is
    /// what stops the window from advancing.
    pub fn pump(&mut self) {
        // The pages are taken out of the table first, so that the loop
        // holds one page per slot without asking for it by index.
        let pages = self.rings.map(|rings| rings.page);
        for (index, page) in pages.into_iter().enumerate() {
            let Some(entry) = self.sockets.at(index).copied() else {
                continue;
            };
            match entry.kind {
                Kind::Connection => {
                    let _sent = self.push(page, entry.handle);
                    let _taken = self.take(page, entry.handle);
                }
                Kind::Datagram => self.take_datagrams(page, entry.handle),
                Kind::Listener => {}
            }
        }
    }

    /// Answers one request of the client `badge` names.
    pub fn answer<R: Rng + ?Sized>(
        &mut self,
        badge: u64,
        request: &Request,
        now: Instant,
        rng: &mut R,
    ) -> Reply {
        match request {
            Request::Interface => Reply::Interface(Ok(self.interface())),
            Request::Resolve { name } => Reply::Resolved(self.resolve(badge, name, now, rng)),
            Request::UdpBind { port } => Reply::Bound(self.bind(badge, *port, rng)),
            Request::UdpSendTo {
                socket,
                remote,
                len,
            } => Reply::UdpSent(self.send_to(badge, *socket, *remote, *len, now)),
            Request::UdpClose { socket } => Reply::UdpClosed(self.close_datagram(badge, *socket)),
            Request::TcpConnect { remote } => Reply::Connected(self.connect(badge, *remote, rng)),
            Request::TcpListen { port } => Reply::Listening(self.listen(badge, *port, rng)),
            Request::TcpAccept { socket } => Reply::Accepted(self.accept(badge, *socket)),
            Request::TcpSend { socket, len } => Reply::Sent(self.send(badge, *socket, *len)),
            Request::TcpRecv { socket } => Reply::Received(self.receive(badge, *socket)),
            Request::TcpShutdown { socket, direction } => {
                Reply::ShutDown(self.shutdown(badge, *socket, *direction))
            }
            Request::TcpClose { socket } => Reply::Closed(self.close_connection(badge, *socket)),
            Request::TcpState { socket } => Reply::State(self.state(badge, *socket)),
        }
    }

    /// Gives up every socket of a client that is gone.
    pub fn forget(&mut self, badge: u64) {
        let slots: [bool; MAX_SOCKETS] = core::array::from_fn(|index| {
            self.sockets
                .at(index)
                .is_some_and(|entry| entry.badge == badge)
        });
        for (index, mine) in slots.into_iter().enumerate() {
            if mine {
                self.give_up(index);
            }
        }
        // The client is gone, so the rings it was given may go to another.
        self.sockets.release(badge);
        if let Some((whose, socket, _name)) = self.resolving
            && whose == badge
        {
            self.forget_resolution(socket);
        }
    }

    /// What the link and the addresses are.
    fn interface(&self) -> Interface {
        let mut addresses = Addresses::new();
        for address in self.stack.addresses() {
            let _kept = addresses.push(address);
        }
        let lease = self.stack.lease();
        Interface {
            mac: self.stack.config().hardware,
            addresses,
            gateway: lease.and_then(|granted| granted.router).map(IpAddr::V4),
            lease: lease.is_some(),
        }
    }

    /// Starts a resolution, or answers the one that is running.
    fn resolve<R: Rng + ?Sized>(
        &mut self,
        badge: u64,
        name: &Name,
        now: Instant,
        rng: &mut R,
    ) -> Result<Addresses, Error> {
        if let Some((whose, socket, running)) = self.resolving {
            // One resolution runs at a time, and the answer that is coming
            // is the answer to the name it began with; a client asking for
            // another name is told the server is busy, as another client
            // is.
            if whose != badge || running != *name {
                return Err(Error::Busy);
            }
            return self.resolution(socket);
        }
        let text = core::str::from_utf8(name.as_bytes()).map_err(|_| Error::InvalidArgument)?;
        let asked = DnsName::from_ascii(text).map_err(|_| Error::InvalidArgument)?;
        let buffer = self.pool.take_datagram().ok_or(Error::OutOfMemory)?;
        // The port is drawn from the dynamic range as RFC 6056 asks, which
        // is what `bind_ephemeral` is.
        let socket = match self.stack.bind_ephemeral(None, rng, buffer) {
            Ok(socket) => socket,
            Err(error) => return Err(refusal(error)),
        };
        match self.stack.resolve(&asked, socket, now) {
            Ok(()) => {
                self.resolving = Some((badge, socket, *name));
                Err(Error::WouldBlock)
            }
            Err(error) => {
                self.release_datagram(socket);
                Err(refusal(error))
            }
        }
    }

    /// Where the resolution that is running stands.
    fn resolution(&mut self, socket: Handle) -> Result<Addresses, Error> {
        match self.stack.resolution() {
            Some(Status::Done) => {
                let mut addresses = Addresses::new();
                for address in self.stack.resolved() {
                    let _kept = addresses.push(address);
                }
                self.forget_resolution(socket);
                if addresses.is_empty() {
                    return Err(Error::NotFound);
                }
                Ok(addresses)
            }
            Some(Status::Failed(_)) => {
                self.forget_resolution(socket);
                Err(Error::NotFound)
            }
            Some(Status::Idle | Status::Asking) => Err(Error::WouldBlock),
            None => Err(Error::NotFound),
        }
    }

    /// Ends the resolution that is running over `socket` and gives that
    /// socket back.
    fn forget_resolution(&mut self, socket: Handle) {
        self.resolving = None;
        self.stack.forget_resolution();
        self.release_datagram(socket);
    }

    /// Opens a datagram socket.
    fn bind<R: Rng + ?Sized>(
        &mut self,
        badge: u64,
        port: u16,
        rng: &mut R,
    ) -> Result<Opened, Error> {
        if port != 0 && self.stack.is_bound(Port::new(port)) {
            return Err(Error::AddressInUse);
        }
        let buffer = self.pool.take_datagram().ok_or(Error::OutOfMemory)?;
        let opened = if port == 0 {
            self.stack.bind_ephemeral(None, rng, buffer)
        } else {
            self.stack.bind(None, Port::new(port), buffer)
        };
        let handle = opened.map_err(refusal)?;
        self.hand_out(badge, Kind::Datagram, handle)
            .inspect_err(|_refused| {
                self.release_datagram(handle);
            })
    }

    /// Opens a connection.
    fn connect<R: Rng + ?Sized>(
        &mut self,
        badge: u64,
        remote: Endpoint,
        rng: &mut R,
    ) -> Result<Opened, Error> {
        if self.stack.source_for(remote.address).is_none() {
            return Err(Error::Unavailable);
        }
        let (send, receive) = self.pool.take_window().ok_or(Error::OutOfMemory)?;
        let handle = match self
            .stack
            .connect_to(remote.address, remote.port(), rng, send, receive)
        {
            Ok(handle) => handle,
            Err(error) => return Err(refusal(error)),
        };
        self.hand_out(badge, Kind::Connection, handle)
            .inspect_err(|_refused| {
                self.release_connection(handle);
            })
    }

    /// Takes connections on a port.
    fn listen<R: Rng + ?Sized>(
        &mut self,
        badge: u64,
        port: u16,
        rng: &mut R,
    ) -> Result<u32, Error> {
        let local = self.stack.addresses().next().ok_or(Error::Unavailable)?;
        if self.stack.listens_on(Port::new(port)) {
            return Err(Error::AddressInUse);
        }
        let (send, receive) = self.pool.take_window().ok_or(Error::OutOfMemory)?;
        let handle = match self
            .stack
            .listen(local, Port::new(port), rng, send, receive)
        {
            Ok(handle) => handle,
            Err(error) => return Err(refusal(error)),
        };
        match self.hand_out(badge, Kind::Listener, handle) {
            Ok(opened) => Ok(opened.socket),
            Err(error) => {
                self.release_connection(handle);
                Err(error)
            }
        }
    }

    /// The connection a listener took, once a peer opened one.
    ///
    /// The listener becomes that connection: one connection of the stack
    /// is what a listen opened, and what a peer's `SYN` turned into an
    /// open one is the same connection. A client that wants another
    /// listens again.
    fn accept(&mut self, badge: u64, number: u32) -> Result<Opened, Error> {
        let index = self.sockets.slot(badge, number).ok_or(Error::NotFound)?;
        let entry = self.sockets.at(index).copied().ok_or(Error::NotFound)?;
        if entry.kind != Kind::Listener {
            return Err(Error::InvalidState);
        }
        let state = self.tcp_state(entry.handle)?;
        match state {
            TcpState::Listen | TcpState::SynReceived => Err(Error::WouldBlock),
            TcpState::Closed => Err(Error::InvalidState),
            _open => {
                if let Some(slot) = self.sockets.at_mut(index) {
                    slot.kind = Kind::Connection;
                }
                Ok(Opened {
                    socket: number,
                    rings: self.object(index)?,
                })
            }
        }
    }

    /// Moves what the client put into the outbound ring into the
    /// connection.
    fn send(&mut self, badge: u64, number: u32, _len: u32) -> Result<u32, Error> {
        let entry = self.connection(badge, number)?;
        let index = self.sockets.slot(badge, number).ok_or(Error::NotFound)?;
        let page = self.page(index)?;
        Ok(self.push(page, entry.handle))
    }

    /// Moves what the connection holds into the inbound ring and answers
    /// how many bytes are waiting there.
    fn receive(&mut self, badge: u64, number: u32) -> Result<u32, Error> {
        let entry = self.connection(badge, number)?;
        let index = self.sockets.slot(badge, number).ok_or(Error::NotFound)?;
        let page = self.page(index)?;
        let _taken = self.take(page, entry.handle);
        Ok(page.inbound.held())
    }

    /// Sends what the client put into the outbound ring to one address.
    fn send_to(
        &mut self,
        badge: u64,
        number: u32,
        remote: Endpoint,
        len: u32,
        now: Instant,
    ) -> Result<u32, Error> {
        let entry = self.datagram(badge, number)?;
        let index = self.sockets.slot(badge, number).ok_or(Error::NotFound)?;
        let wanted = usize::try_from(len).unwrap_or(0);
        // A longer datagram is refused before anything leaves the ring:
        // what this took and left behind would be the head of the next
        // one.
        if wanted > DATAGRAM {
            return Err(Error::BufferTooSmall);
        }
        let mut payload = [0u8; DATAGRAM];
        let taken = self
            .page(index)?
            .outbound
            .read(payload.get_mut(..wanted).unwrap_or(&mut []));
        let bytes = payload.get(..taken).unwrap_or(&[]);
        self.stack
            .send_to(entry.handle, remote.address, remote.port(), bytes, now)
            .map_err(refusal)?;
        Ok(u32::try_from(taken).unwrap_or(0))
    }

    /// Stops one direction of a connection.
    fn shutdown(&mut self, badge: u64, number: u32, direction: Direction) -> Result<(), Error> {
        let entry = self.connection(badge, number)?;
        if direction == Direction::Read {
            return Ok(());
        }
        // What the client left in the ring goes into the connection first:
        // the `FIN` follows everything the send buffer holds, and a
        // connection that is closing takes nothing more.
        let index = self.sockets.slot(badge, number).ok_or(Error::NotFound)?;
        let page = self.page(index)?;
        let _sent = self.push(page, entry.handle);
        let connection = self.stack.connection(entry.handle).map_err(refusal)?;
        connection.close().map_err(|_| Error::InvalidState)
    }

    /// Gives a connection back.
    fn close_connection(&mut self, badge: u64, number: u32) -> Result<(), Error> {
        let index = self.sockets.slot(badge, number).ok_or(Error::NotFound)?;
        let entry = self.sockets.at(index).copied().ok_or(Error::NotFound)?;
        if entry.kind == Kind::Datagram {
            return Err(Error::WrongObjectType);
        }
        self.give_up(index);
        Ok(())
    }

    /// Gives a datagram socket back.
    fn close_datagram(&mut self, badge: u64, number: u32) -> Result<(), Error> {
        let index = self.sockets.slot(badge, number).ok_or(Error::NotFound)?;
        let entry = self.sockets.at(index).copied().ok_or(Error::NotFound)?;
        if entry.kind != Kind::Datagram {
            return Err(Error::WrongObjectType);
        }
        self.give_up(index);
        Ok(())
    }

    /// Where a connection stands.
    fn state(&mut self, badge: u64, number: u32) -> Result<State, Error> {
        let entry = self
            .sockets
            .get(badge, number)
            .copied()
            .ok_or(Error::NotFound)?;
        if entry.kind == Kind::Datagram {
            return Err(Error::WrongObjectType);
        }
        let connection = self.stack.connection(entry.handle).map_err(refusal)?;
        if connection.was_reset() {
            return Ok(State::Refused);
        }
        if connection.timed_out() {
            return Ok(State::Refused);
        }
        Ok(match connection.state() {
            TcpState::Closed => State::Closed,
            TcpState::Listen | TcpState::SynSent | TcpState::SynReceived => State::Connecting,
            TcpState::Established => State::Established,
            TcpState::CloseWait | TcpState::LastAck => State::PeerClosed,
            TcpState::FinWait1 | TcpState::FinWait2 | TcpState::Closing | TcpState::TimeWait => {
                State::Closed
            }
        })
    }

    /// The socket `number` names, which has to be a connection.
    fn connection(&self, badge: u64, number: u32) -> Result<Entry, Error> {
        let entry = self
            .sockets
            .get(badge, number)
            .copied()
            .ok_or(Error::NotFound)?;
        if entry.kind == Kind::Connection {
            Ok(entry)
        } else {
            Err(Error::WrongObjectType)
        }
    }

    /// The socket `number` names, which has to be a datagram socket.
    fn datagram(&self, badge: u64, number: u32) -> Result<Entry, Error> {
        let entry = self
            .sockets
            .get(badge, number)
            .copied()
            .ok_or(Error::NotFound)?;
        if entry.kind == Kind::Datagram {
            Ok(entry)
        } else {
            Err(Error::WrongObjectType)
        }
    }

    /// The state of one connection of the stack.
    fn tcp_state(&mut self, handle: Handle) -> Result<TcpState, Error> {
        Ok(self.stack.connection(handle).map_err(refusal)?.state())
    }

    /// Takes a slot and answers the socket and the rings that go with it.
    fn hand_out(&mut self, badge: u64, kind: Kind, handle: Handle) -> Result<Opened, Error> {
        let (socket, index) = self
            .sockets
            .open(badge, kind, handle)
            .ok_or(Error::OutOfHandles)?;
        let page = self.page(index)?;
        page.initialize();
        Ok(Opened {
            socket,
            rings: self.object(index)?,
        })
    }

    /// The rings of slot `index`.
    fn page(&self, index: usize) -> Result<&'a SocketPage, Error> {
        Ok(self.rings.get(index).ok_or(Error::NotFound)?.page)
    }

    /// The memory object over the rings of slot `index`.
    fn object(&self, index: usize) -> Result<RawHandle, Error> {
        Ok(self.rings.get(index).ok_or(Error::NotFound)?.object)
    }

    /// Closes the socket in `index`, whatever it is, and gives its
    /// buffers back.
    fn give_up(&mut self, index: usize) {
        let Some(entry) = self.sockets.close(index) else {
            return;
        };
        match entry.kind {
            Kind::Datagram => self.release_datagram(entry.handle),
            Kind::Connection | Kind::Listener => self.release_connection(entry.handle),
        }
    }

    /// Closes one connection of the stack and takes its windows back.
    ///
    /// The close is what ends the connection; nothing resets it first,
    /// because a listener that never opened has nobody to reset.
    fn release_connection(&mut self, handle: Handle) {
        if let Ok(window) = self.stack.close_connection(handle) {
            let _kept = self.pool.put_window(window);
        }
    }

    /// Closes one datagram socket of the stack and takes its buffer back.
    fn release_datagram(&mut self, handle: Handle) {
        if let Ok(buffer) = self.stack.close(handle) {
            let _kept = self.pool.put_datagram(buffer);
        }
    }

    /// Moves bytes of the outbound ring into the connection, and answers
    /// how many.
    fn push(&mut self, page: &SocketPage, handle: Handle) -> u32 {
        let Ok(connection) = self.stack.connection(handle) else {
            return 0;
        };
        // A connection that is not open yet takes nothing, and the bytes
        // stay in the ring until it is: the room below is the send buffer
        // and says nothing about the state.
        if !connection.state().can_send() {
            return 0;
        }
        let room = connection.writable().min(CHUNK);
        if room == 0 {
            return 0;
        }
        let mut bytes = [0u8; CHUNK];
        let taken = page.outbound.read(bytes.get_mut(..room).unwrap_or(&mut []));
        if taken == 0 {
            return 0;
        }
        let written = connection
            .write(bytes.get(..taken).unwrap_or(&[]))
            .unwrap_or(0);
        // What the connection would not take goes back to the front of
        // the ring, which no ring of one writer allows, so the window is
        // asked first and this cannot happen.
        u32::try_from(written).unwrap_or(0)
    }

    /// Moves bytes of the connection into the inbound ring, and answers
    /// how many.
    fn take(&mut self, page: &SocketPage, handle: Handle) -> u32 {
        let room = usize::try_from(page.inbound.free()).unwrap_or(0).min(CHUNK);
        if room == 0 {
            return 0;
        }
        let Ok(connection) = self.stack.connection(handle) else {
            return 0;
        };
        let mut bytes = [0u8; CHUNK];
        let taken = connection.read(bytes.get_mut(..room).unwrap_or(&mut []));
        if taken == 0 {
            return 0;
        }
        let written = page.inbound.write(bytes.get(..taken).unwrap_or(&[]));
        u32::try_from(written).unwrap_or(0)
    }

    /// Moves the datagrams a socket took into its inbound ring, each
    /// behind the record of where it came from.
    ///
    /// A datagram the ring has no room for stays in the socket's own ring
    /// and is moved the next time round, which is the back pressure of a
    /// datagram socket: what overflows is what `net-udp` counts as
    /// dropped, and nothing here grows.
    fn take_datagrams(&mut self, page: &SocketPage, handle: Handle) {
        loop {
            let Ok(socket) = self.stack.socket(handle) else {
                return;
            };
            let Some(received) = socket.peek() else {
                return;
            };
            let len = received.payload.len();
            let from = Endpoint::new(received.source, received.port.get());
            let header = datagram_header(from, u16::try_from(len).unwrap_or(u16::MAX));
            let needed = u32::try_from(DATAGRAM_HEADER_LEN.saturating_add(len)).unwrap_or(u32::MAX);
            if needed > page.inbound.free() {
                return;
            }
            // The whole datagram goes in, straight out of the socket:
            // a record that said one length and carried another would be
            // read as the head of the next one.
            let _header = page.inbound.write(&header);
            let _payload = page.inbound.write(received.payload);
            let _taken = socket.discard();
        }
    }
}

/// What a refusal of the stack becomes at the interface.
#[must_use]
pub const fn refusal(error: StackError) -> Error {
    match error {
        StackError::Full => Error::OutOfMemory,
        StackError::Busy => Error::Busy,
        StackError::Stale | StackError::Unknown => Error::InvalidHandle,
        StackError::NoAddress | StackError::Unresolved => Error::Unavailable,
        _other => Error::InvalidArgument,
    }
}
