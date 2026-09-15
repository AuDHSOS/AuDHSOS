// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One end of a TCP connection, over the socket protocol of `server-net`
//! (D-116).
//!
//! What a program has without this is a socket number, a memory object
//! holding two rings, and eight message kinds: it maps the object, writes
//! into the outbound ring, tells the server how many bytes went in, asks
//! for what arrived, reads the inbound ring, and asks where the
//! connection stands. [`Stream`] is that sequence with a name, so a
//! program that talks over a connection writes what it is saying and not
//! how a ring works.
//!
//! Every call of the protocol is answered at once and what is not ready
//! yet comes back as `WouldBlock` (D-142), so a program asks again after a
//! wait on the clock. [`Idle`] is that wait: one notification for the
//! whole program, because one made for each wait uses up the object quota
//! of the process.
//!
//! Invariants: a [`Stream`] holds the mapping its rings live in for as
//! long as it stands; a [`Listener`] becomes the connection it took and
//! keeps its number (D-143), which is why [`Listener::accept`] consumes
//! it.

use audhsos_abi::Error;
use audhsos_abi::layout::PAGE_SIZE;
use user_proto::ring::{SOCKET_PAGE_LEN, SocketPage};
use user_proto::socket::{Direction, Endpoint, Reply, Request, State};
use user_rt::{EndpointHandle, MemoryHandle, NotificationHandle, ProcessHandle, Typed as _};
use user_sys_x86_64::Gate;

use crate::mapping::Mapping;

/// How long a wait sleeps before it asks again, in microseconds.
pub const STEP: u64 = 2_000;

/// What one ask of a connection found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Received {
    /// That many bytes came out of the inbound ring.
    Bytes(usize),
    /// Nothing yet, and the connection is open: ask again.
    Waiting,
    /// The peer will send no more and the ring is empty.
    Ended,
}

/// What every wait of a program sleeps on.
///
/// One is made for the whole program and handed to each call that may
/// have to wait.
#[derive(Clone, Copy, Debug)]
pub struct Idle {
    /// The notification the wait sleeps on. Nothing signals it; the
    /// deadline is what wakes the thread.
    notification: NotificationHandle,
    /// How long one sleep is, in microseconds.
    step: u64,
}

impl Idle {
    /// A wait on a notification of this program's own, sleeping [`STEP`]
    /// at a time.
    ///
    /// # Errors
    ///
    /// Whatever the kernel answered to the notification this makes.
    pub fn new(gate: &mut Gate) -> Result<Idle, Error> {
        Ok(Idle {
            notification: gate.notification_create()?,
            step: STEP,
        })
    }

    /// The same, sleeping `step` microseconds at a time.
    ///
    /// A step of zero would be a deadline already behind the clock, which
    /// is a wait that returns at once and a loop around it that spins, so
    /// zero is read as [`STEP`].
    #[must_use]
    pub const fn with_step(self, step: u64) -> Idle {
        Idle {
            step: if step == 0 { STEP } else { step },
            ..self
        }
    }

    /// Sleeps one step, and refuses once `until` has passed.
    ///
    /// # Errors
    ///
    /// [`Error::Cancelled`] when `until` is behind the clock, and whatever
    /// the kernel answered to the clock or the wait.
    pub fn wait(self, gate: &mut Gate, until: u64) -> Result<(), Error> {
        let now = gate.clock_now()?;
        if now >= until {
            return Err(Error::Cancelled);
        }
        let _bits =
            gate.notification_wait_until(self.notification, now.saturating_add(self.step))?;
        Ok(())
    }
}

/// A port this program takes connections on.
#[derive(Debug)]
pub struct Listener {
    /// The network server.
    server: EndpointHandle,
    /// The socket the listen answered.
    socket: u32,
    /// The port it took.
    port: u16,
}

impl Listener {
    /// Takes connections on `port`.
    ///
    /// # Errors
    ///
    /// Whatever the server answered, and the errors of the call itself.
    pub fn bind(gate: &mut Gate, server: EndpointHandle, port: u16) -> Result<Listener, Error> {
        let Reply::Listening(outcome) = call(gate, server, &Request::TcpListen { port })? else {
            return Err(Error::InvalidArgument);
        };
        Ok(Listener {
            server,
            socket: outcome?,
            port,
        })
    }

    /// The port it took.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// The connection a peer opened, once one has, with its rings mapped
    /// at `at`.
    ///
    /// The listener becomes that connection and keeps its number (D-143),
    /// so this consumes it: a program that wants a second connection binds
    /// again.
    ///
    /// # Errors
    ///
    /// [`Error::Cancelled`] when `until` passes before a peer opens one,
    /// and whatever the server or the kernel answered.
    pub fn accept(
        self,
        gate: &mut Gate,
        process: ProcessHandle,
        at: u64,
        idle: Idle,
        until: u64,
    ) -> Result<Stream, Error> {
        loop {
            let reply = call(
                gate,
                self.server,
                &Request::TcpAccept {
                    socket: self.socket,
                },
            )?;
            let Reply::Accepted(outcome) = reply else {
                return Err(Error::InvalidArgument);
            };
            match outcome {
                Ok(opened) => {
                    return Stream::over(
                        gate,
                        self.server,
                        process,
                        opened.socket,
                        MemoryHandle::from_handle(opened.rings),
                        at,
                    );
                }
                Err(Error::WouldBlock) => idle.wait(gate, until)?,
                Err(error) => return Err(error),
            }
        }
    }
}

/// One end of a connection: the socket it is, and the two rings it moves
/// bytes through.
#[derive(Debug)]
pub struct Stream {
    /// The network server.
    server: EndpointHandle,
    /// The connection there.
    socket: u32,
    /// Where its rings are mapped. It stands as long as this does.
    mapping: Mapping,
    /// The process the mapping is in, which [`Stream::close`] takes it out
    /// of again.
    process: ProcessHandle,
}

impl Stream {
    /// Opens a connection to `remote`, with its rings mapped at `at`, and
    /// answers once the handshake of RFC 9293 is through.
    ///
    /// `TcpConnect` answers the socket while that handshake still runs
    /// (D-142), so this waits for `Established` before it hands the
    /// connection over: a caller that writes into a connection that is
    /// not open yet has nothing to write into.
    ///
    /// # Errors
    ///
    /// [`Error::Unavailable`] for a connection the peer refused or closed,
    /// [`Error::Cancelled`] when `until` passes first, and whatever the
    /// server or the kernel answered.
    pub fn connect(
        gate: &mut Gate,
        server: EndpointHandle,
        process: ProcessHandle,
        remote: Endpoint,
        at: u64,
        idle: Idle,
        until: u64,
    ) -> Result<Stream, Error> {
        let Reply::Connected(outcome) = call(gate, server, &Request::TcpConnect { remote })? else {
            return Err(Error::InvalidArgument);
        };
        let opened = outcome?;
        let stream = Stream::over(
            gate,
            server,
            process,
            opened.socket,
            MemoryHandle::from_handle(opened.rings),
            at,
        )?;
        loop {
            match stream.state(gate)? {
                State::Established => return Ok(stream),
                State::Connecting => idle.wait(gate, until)?,
                State::PeerClosed | State::Closed | State::Refused => {
                    return Err(Error::Unavailable);
                }
            }
        }
    }

    /// The stream over a socket the server has already answered, with
    /// `object` mapped at `at`.
    fn over(
        gate: &mut Gate,
        server: EndpointHandle,
        process: ProcessHandle,
        socket: u32,
        object: MemoryHandle,
        at: u64,
    ) -> Result<Stream, Error> {
        let len = u64::try_from(SOCKET_PAGE_LEN)
            .unwrap_or(0)
            .next_multiple_of(PAGE_SIZE);
        let mapping = Mapping::new(gate, process, object, at, len)?;
        Ok(Stream {
            server,
            socket,
            mapping,
            process,
        })
    }

    /// The number the server knows this connection by.
    #[must_use]
    pub const fn socket(&self) -> u32 {
        self.socket
    }

    /// The two rings.
    ///
    /// # Errors
    ///
    /// [`Error::Unaligned`] for a mapping that is not a socket page, which
    /// is a server that answered a window of the wrong size.
    pub fn page(&self) -> Result<&SocketPage, Error> {
        // SAFETY: the mapping stands as long as this stream does, it is
        // memory of this process alone, and every access to it is atomic
        // on both sides.
        unsafe { self.mapping.socket_page() }.ok_or(Error::Unaligned)
    }

    /// Where the connection stands.
    ///
    /// # Errors
    ///
    /// Whatever the server answered, and the errors of the call itself.
    pub fn state(&self, gate: &mut Gate) -> Result<State, Error> {
        let Reply::State(state) = call(
            gate,
            self.server,
            &Request::TcpState {
                socket: self.socket,
            },
        )?
        else {
            return Err(Error::InvalidArgument);
        };
        state
    }

    /// Puts every byte of `bytes` into the connection, in as many rounds
    /// as the ring takes.
    ///
    /// A ring with no room is a peer that has not read yet, so this waits
    /// and asks again rather than refusing.
    ///
    /// # Errors
    ///
    /// [`Error::Cancelled`] when `until` passes before the last byte goes
    /// in, and whatever the server or the kernel answered.
    pub fn write_all(
        &self,
        gate: &mut Gate,
        bytes: &[u8],
        idle: Idle,
        until: u64,
    ) -> Result<(), Error> {
        let page = self.page()?;
        let mut rest = bytes;
        while !rest.is_empty() {
            let taken = page.outbound.write(rest);
            rest = rest.get(taken..).unwrap_or(&[]);
            let len = u32::try_from(taken).unwrap_or(0);
            let Reply::Sent(outcome) = call(
                gate,
                self.server,
                &Request::TcpSend {
                    socket: self.socket,
                    len,
                },
            )?
            else {
                return Err(Error::InvalidArgument);
            };
            let _moved = outcome?;
            if taken == 0 {
                idle.wait(gate, until)?;
            }
        }
        Ok(())
    }

    /// Asks once for what the connection holds and puts it into `into`.
    ///
    /// This is the call for a program that has something else to do while
    /// it waits; [`Stream::read`] is the one that waits here.
    ///
    /// # Errors
    ///
    /// [`Error::Unavailable`] for a connection the peer refused or that
    /// timed out, and whatever the server answered.
    pub fn receive(&self, gate: &mut Gate, into: &mut [u8]) -> Result<Received, Error> {
        let page = self.page()?;
        let taken = self.take(gate, page, into)?;
        if taken > 0 {
            return Ok(Received::Bytes(taken));
        }
        match self.state(gate)? {
            State::Established | State::Connecting => Ok(Received::Waiting),
            // The server answers each call out of what the stack held when
            // the call came, so bytes the peer sent before its `FIN` can
            // arrive between the two calls above. One more ask takes them,
            // and after a `FIN` nothing follows, so an empty ring here is
            // the end and not a wait.
            //
            // `Closed` is that same end and not a failure: this side has
            // sent a `FIN` of its own, which the server reports as
            // `Closed` from `FIN-WAIT-1` on, and a peer that is still
            // sending still fills the ring. A program that shuts its write
            // side and then reads to the end of the answer — which is what
            // a half-close is for — would otherwise lose that answer.
            // `Refused` is the failure: that connection never carried
            // anything.
            State::PeerClosed | State::Closed => match self.take(gate, page, into)? {
                0 => Ok(Received::Ended),
                last => Ok(Received::Bytes(last)),
            },
            State::Refused => Err(Error::Unavailable),
        }
    }

    /// Waits until the connection holds something or is over, and answers
    /// how many bytes it put into `into`. Zero is the end of it.
    ///
    /// # Errors
    ///
    /// [`Error::Cancelled`] when `until` passes with nothing arriving, and
    /// whatever the server or the kernel answered.
    pub fn read(
        &self,
        gate: &mut Gate,
        into: &mut [u8],
        idle: Idle,
        until: u64,
    ) -> Result<usize, Error> {
        loop {
            match self.receive(gate, into)? {
                Received::Bytes(taken) => return Ok(taken),
                Received::Ended => return Ok(0),
                Received::Waiting => idle.wait(gate, until)?,
            }
        }
    }

    /// One ask of the server, and what the inbound ring held after it.
    fn take(&self, gate: &mut Gate, page: &SocketPage, into: &mut [u8]) -> Result<usize, Error> {
        let Reply::Received(outcome) = call(
            gate,
            self.server,
            &Request::TcpRecv {
                socket: self.socket,
            },
        )?
        else {
            return Err(Error::InvalidArgument);
        };
        let _held = outcome?;
        Ok(page.inbound.read(into))
    }

    /// Says this side will send no more.
    ///
    /// # Errors
    ///
    /// Whatever the server answered, and the errors of the call itself.
    pub fn shutdown_write(&self, gate: &mut Gate) -> Result<(), Error> {
        let Reply::ShutDown(outcome) = call(
            gate,
            self.server,
            &Request::TcpShutdown {
                socket: self.socket,
                direction: Direction::Write,
            },
        )?
        else {
            return Err(Error::InvalidArgument);
        };
        outcome
    }

    /// Gives the connection back, and the window its rings were mapped
    /// in with it.
    ///
    /// The socket goes first, so that the server has stopped writing into
    /// the rings before they leave the address space. The window is taken
    /// back because a program that opened a second connection at the same
    /// address would otherwise be refused by the kernel for a range it
    /// believes it gave up.
    ///
    /// # Errors
    ///
    /// Whatever the server answered to the close, and whatever the kernel
    /// answered to the unmapping; the socket is given back either way.
    pub fn close(self, gate: &mut Gate) -> Result<(), Error> {
        let closed = match call(
            gate,
            self.server,
            &Request::TcpClose {
                socket: self.socket,
            },
        )? {
            Reply::Closed(outcome) => outcome,
            _ => Err(Error::InvalidArgument),
        };
        let unmapped = self.mapping.unmap(gate, self.process);
        closed.and(unmapped)
    }
}

/// Sends one request of the socket protocol and reads the reply.
fn call(gate: &mut Gate, server: EndpointHandle, request: &Request) -> Result<Reply, Error> {
    request.encode(&mut gate.writer())?;
    gate.ipc_call(server)?;
    Ok(Reply::decode(gate.reader())?)
}
