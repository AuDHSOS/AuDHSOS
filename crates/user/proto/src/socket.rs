// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The socket protocol: what the network server is asked, and the two
//! rings a socket's bytes travel through.
//!
//! Three rules shape it, and they are the rules of the input protocol and
//! the display protocol one layer on (D-31, D-116).
//!
//! A socket is a number the server chose and not a kernel object, because
//! a connection is not a kernel object. The server keeps one table of
//! sockets per client and finds that table by the badge on the endpoint
//! the message came through, exactly as the file system server finds an
//! open file.
//!
//! Bulk data moves through a ring in a memory object the server made, not
//! through one call per byte. A socket has two of them in one object: the
//! one the server writes and the client reads, and the one the client
//! writes and the server reads. A `TcpSend` says how many bytes the client
//! put into the second; a `TcpRecv` says how many the server put into the
//! first.
//!
//! A client that stops reading fills its ring, the server stops taking
//! bytes out of the connection, and the window of RFC 9293 stops
//! advancing. That is the back pressure TCP already has, and it is why
//! nothing in the server grows without bound.
//!
//! Invariants: a reply carries a handle only when its status word says the
//! request succeeded; the writer of a ring never passes its reader and
//! counts what it could not write instead of overwriting it.

use audhsos_abi::ipc_buffer::{Buffer, BufferMut};
use audhsos_abi::{Error, Handle};
use net_wire::{IpAddr, Ipv4Addr, Ipv6Addr, MacAddr, Port};
use user_rt::message::{Reader, Writer};

use crate::bytes::Bytes;
use crate::label::{Label, ProtoError, Protocol, status_of, status_word};

/// `interface`: what the link and the addresses are.
pub const INTERFACE: u16 = 1;

/// `resolve`: the addresses of a name.
pub const RESOLVE: u16 = 2;

/// `udp_bind`: a datagram socket on a port.
pub const UDP_BIND: u16 = 3;

/// `udp_send_to`: send what is in the ring to one address.
pub const UDP_SEND_TO: u16 = 4;

/// `udp_close`: give a datagram socket back.
pub const UDP_CLOSE: u16 = 5;

/// `tcp_connect`: open a connection.
pub const TCP_CONNECT: u16 = 6;

/// `tcp_listen`: take connections on a port.
pub const TCP_LISTEN: u16 = 7;

/// `tcp_accept`: the next connection of a listener.
pub const TCP_ACCEPT: u16 = 8;

/// `tcp_send`: move what is in the ring into the connection.
pub const TCP_SEND: u16 = 9;

/// `tcp_recv`: move what the connection holds into the ring.
pub const TCP_RECV: u16 = 10;

/// `tcp_shutdown`: stop one direction of a connection.
pub const TCP_SHUTDOWN: u16 = 11;

/// `tcp_close`: give a connection back.
pub const TCP_CLOSE: u16 = 12;

/// `tcp_state`: where a connection stands.
pub const TCP_STATE: u16 = 13;

/// How many bytes a name to resolve has. RFC 1035, section 2.3.4 bounds a
/// domain name at 255 bytes in its text form.
pub const MAX_NAME: usize = 255;

/// A name a client asks the resolver for.
pub type Name = Bytes<MAX_NAME>;

/// How many addresses a resolution answers with.
pub const MAX_ADDRESSES: usize = 4;

/// How many addresses an interface report carries.
pub const MAX_INTERFACE_ADDRESSES: usize = 4;

/// The socket a client gets for a request that answers none.
pub const NO_SOCKET: u32 = 0;

/// Where one end of a connection is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Endpoint {
    /// The address.
    pub address: IpAddr,
    /// The port.
    pub port: u16,
}

impl Endpoint {
    /// The endpoint at `address` and `port`.
    #[must_use]
    pub const fn new(address: IpAddr, port: u16) -> Endpoint {
        Endpoint { address, port }
    }

    /// The port as the transports name one.
    #[must_use]
    pub const fn port(self) -> Port {
        Port::new(self.port)
    }
}

/// Which half of a connection a shutdown closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Send no more.
    Write,
    /// Take no more.
    Read,
    /// Neither.
    Both,
}

impl Direction {
    /// The code the message carries.
    #[must_use]
    pub const fn code(self) -> u64 {
        match self {
            Direction::Write => 1,
            Direction::Read => 2,
            Direction::Both => 3,
        }
    }

    /// The direction that code names.
    #[must_use]
    pub const fn from_code(code: u64) -> Option<Direction> {
        match code {
            1 => Some(Direction::Write),
            2 => Some(Direction::Read),
            3 => Some(Direction::Both),
            _ => None,
        }
    }
}

/// Where a connection stands, as much of RFC 9293 as a client has to tell
/// apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum State {
    /// The handshake is running.
    Connecting,
    /// Bytes may go both ways.
    Established,
    /// The peer will send no more.
    PeerClosed,
    /// The connection is over.
    Closed,
    /// The peer refused it or reset it.
    Refused,
}

impl State {
    /// The code the message carries.
    #[must_use]
    pub const fn code(self) -> u64 {
        match self {
            State::Connecting => 1,
            State::Established => 2,
            State::PeerClosed => 3,
            State::Closed => 4,
            State::Refused => 5,
        }
    }

    /// The state that code names.
    #[must_use]
    pub const fn from_code(code: u64) -> Option<State> {
        match code {
            1 => Some(State::Connecting),
            2 => Some(State::Established),
            3 => Some(State::PeerClosed),
            4 => Some(State::Closed),
            5 => Some(State::Refused),
            _ => None,
        }
    }

    /// The name of the state, for a program that reports one.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            State::Connecting => "connecting",
            State::Established => "established",
            State::PeerClosed => "peer-closed",
            State::Closed => "closed",
            State::Refused => "refused",
        }
    }
}

/// What a client asks the network server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "the name of a resolution is what the protocol carries, and boxing it would need a heap this system does not have"
)]
pub enum Request {
    /// What the link and the addresses are.
    Interface,
    /// The addresses of a name.
    Resolve {
        /// The name to look up.
        name: Name,
    },
    /// A datagram socket on `port`; zero asks for one of the dynamic
    /// range.
    UdpBind {
        /// The port to take.
        port: u16,
        /// The client's process, reduced to `INFO` and `TRANSFER`, for
        /// its watch.
        process: Handle,
    },
    /// Sends what the client put into the outbound ring to `remote`.
    UdpSendTo {
        /// The socket.
        socket: u32,
        /// Where the datagram goes.
        remote: Endpoint,
        /// How many bytes of the ring are the datagram.
        len: u32,
    },
    /// Gives a datagram socket back.
    UdpClose {
        /// The socket.
        socket: u32,
    },
    /// Opens a connection to `remote`.
    TcpConnect {
        /// Where to connect to.
        remote: Endpoint,
        /// As [`Request::UdpBind`].
        process: Handle,
    },
    /// Takes connections on `port`.
    TcpListen {
        /// The port to listen on.
        port: u16,
        /// As [`Request::UdpBind`].
        process: Handle,
    },
    /// The next connection of a listener.
    TcpAccept {
        /// The listener.
        socket: u32,
    },
    /// Moves what the client put into the outbound ring into the
    /// connection.
    TcpSend {
        /// The connection.
        socket: u32,
        /// How many bytes of the ring to send.
        len: u32,
    },
    /// Moves what the connection holds into the inbound ring.
    TcpRecv {
        /// The connection.
        socket: u32,
    },
    /// Stops one direction of a connection.
    TcpShutdown {
        /// The connection.
        socket: u32,
        /// Which direction.
        direction: Direction,
    },
    /// Gives a connection back.
    TcpClose {
        /// The connection.
        socket: u32,
    },
    /// Where a connection stands.
    TcpState {
        /// The connection.
        socket: u32,
    },
}

/// What the link is and what this host is configured with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Interface {
    /// The hardware address of the device.
    pub mac: MacAddr,
    /// The addresses this host holds.
    pub addresses: Addresses,
    /// The router of the link, where the configuration named one.
    pub gateway: Option<IpAddr>,
    /// Whether the address configuration client has a lease.
    pub lease: bool,
}

/// A list of addresses, as short as the message that carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Addresses {
    /// The addresses.
    entries: [Option<IpAddr>; MAX_ADDRESSES],
}

impl Default for Addresses {
    fn default() -> Self {
        Self::new()
    }
}

impl Addresses {
    /// No addresses at all.
    #[must_use]
    pub const fn new() -> Addresses {
        Addresses {
            entries: [None; MAX_ADDRESSES],
        }
    }

    /// Appends `address`, or answers `false` when the list is full.
    pub fn push(&mut self, address: IpAddr) -> bool {
        let Some(slot) = self.entries.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *slot = Some(address);
        true
    }

    /// The addresses, in the order they went in.
    pub fn iter(&self) -> impl Iterator<Item = IpAddr> + '_ {
        self.entries.iter().flatten().copied()
    }

    /// How many there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// `true` for no addresses at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// What a socket the server opened is reached through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opened {
    /// The number the client names the socket by.
    pub socket: u32,
    /// The memory object of its two rings, which travels in the handle
    /// area.
    pub rings: Handle,
}

/// What the server answers.
///
/// One variant per request, as every protocol of this crate has, so that
/// the label of a reply is the label of the request it answers and a
/// client that meets a reply to something else knows at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// What the link and the addresses are.
    Interface(Result<Interface, Error>),
    /// The addresses of a name, in the order RFC 6724 puts them.
    Resolved(Result<Addresses, Error>),
    /// The datagram socket that was opened.
    Bound(Result<Opened, Error>),
    /// How many bytes of the ring went out as a datagram.
    UdpSent(Result<u32, Error>),
    /// The datagram socket is gone.
    UdpClosed(Result<(), Error>),
    /// The connection that is being opened.
    ///
    /// The connection is not established yet: the client waits on the
    /// notification of its rings and asks [`Request::TcpState`].
    Connected(Result<Opened, Error>),
    /// The listener that was opened.
    Listening(Result<u32, Error>),
    /// The connection a listener took, or [`Error::WouldBlock`] when none
    /// has arrived yet.
    Accepted(Result<Opened, Error>),
    /// How many bytes of the ring went into the connection.
    Sent(Result<u32, Error>),
    /// How many bytes of the connection went into the ring.
    Received(Result<u32, Error>),
    /// One direction of the connection is closed, or
    /// [`Error::WouldBlock`] while the outbound ring still holds bytes.
    ShutDown(Result<(), Error>),
    /// The connection is gone.
    Closed(Result<(), Error>),
    /// Where the connection stands.
    State(Result<State, Error>),
}

impl Request {
    /// The label this request is sent under.
    #[must_use]
    pub const fn label(&self) -> Label {
        let message = match self {
            Request::Interface => INTERFACE,
            Request::Resolve { .. } => RESOLVE,
            Request::UdpBind { .. } => UDP_BIND,
            Request::UdpSendTo { .. } => UDP_SEND_TO,
            Request::UdpClose { .. } => UDP_CLOSE,
            Request::TcpConnect { .. } => TCP_CONNECT,
            Request::TcpListen { .. } => TCP_LISTEN,
            Request::TcpAccept { .. } => TCP_ACCEPT,
            Request::TcpSend { .. } => TCP_SEND,
            Request::TcpRecv { .. } => TCP_RECV,
            Request::TcpShutdown { .. } => TCP_SHUTDOWN,
            Request::TcpClose { .. } => TCP_CLOSE,
            Request::TcpState { .. } => TCP_STATE,
        };
        Label::new(Protocol::Socket, message)
    }

    /// Writes the request into `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Codec`] when the message area has no room, which only
    /// a name of more than [`MAX_NAME`] bytes runs into.
    pub fn encode(&self, buffer: &mut BufferMut<'_>) -> Result<(), ProtoError> {
        let mut writer = Writer::new();
        match self {
            Request::Interface => {}
            Request::Resolve { name } => writer.bytes(buffer, name.as_bytes())?,
            Request::UdpBind { port, process } | Request::TcpListen { port, process } => {
                writer.word(buffer, u64::from(*port))?;
                writer.handle(buffer, *process)?;
            }
            Request::UdpSendTo {
                socket,
                remote,
                len,
            } => {
                writer.word(buffer, u64::from(*socket))?;
                writer.word(buffer, u64::from(*len))?;
                write_endpoint(&mut writer, buffer, *remote)?;
            }
            Request::TcpConnect { remote, process } => {
                write_endpoint(&mut writer, buffer, *remote)?;
                writer.handle(buffer, *process)?;
            }
            Request::UdpClose { socket }
            | Request::TcpAccept { socket }
            | Request::TcpRecv { socket }
            | Request::TcpClose { socket }
            | Request::TcpState { socket } => writer.word(buffer, u64::from(*socket))?,
            Request::TcpSend { socket, len } => {
                writer.word(buffer, u64::from(*socket))?;
                writer.word(buffer, u64::from(*len))?;
            }
            Request::TcpShutdown { socket, direction } => {
                writer.word(buffer, u64::from(*socket))?;
                writer.word(buffer, direction.code())?;
            }
        }
        writer.finish(buffer, self.label().raw())?;
        Ok(())
    }

    /// Reads a request out of `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError`] for a label of another protocol or another version,
    /// for a message number this protocol does not have, for a name longer
    /// than [`MAX_NAME`], for an address family the message does not name,
    /// and for a message whose fields are not there.
    pub fn decode(buffer: Buffer<'_>) -> Result<Self, ProtoError> {
        let mut reader = Reader::new(buffer)?;
        let label = expect(reader.label())?;
        match label.message {
            INTERFACE => Ok(Request::Interface),
            RESOLVE => Ok(Request::Resolve {
                name: name_of(&mut reader)?,
            }),
            UDP_BIND => Ok(Request::UdpBind {
                port: port_of(reader.word()?),
                process: reader.handle()?,
            }),
            TCP_LISTEN => Ok(Request::TcpListen {
                port: port_of(reader.word()?),
                process: reader.handle()?,
            }),
            UDP_SEND_TO => Ok(Request::UdpSendTo {
                socket: word32(reader.word()?),
                len: word32(reader.word()?),
                remote: read_endpoint(&mut reader)?,
            }),
            TCP_CONNECT => Ok(Request::TcpConnect {
                remote: read_endpoint(&mut reader)?,
                process: reader.handle()?,
            }),
            UDP_CLOSE => Ok(Request::UdpClose {
                socket: word32(reader.word()?),
            }),
            TCP_ACCEPT => Ok(Request::TcpAccept {
                socket: word32(reader.word()?),
            }),
            TCP_RECV => Ok(Request::TcpRecv {
                socket: word32(reader.word()?),
            }),
            TCP_CLOSE => Ok(Request::TcpClose {
                socket: word32(reader.word()?),
            }),
            TCP_STATE => Ok(Request::TcpState {
                socket: word32(reader.word()?),
            }),
            TCP_SEND => Ok(Request::TcpSend {
                socket: word32(reader.word()?),
                len: word32(reader.word()?),
            }),
            TCP_SHUTDOWN => Ok(Request::TcpShutdown {
                socket: word32(reader.word()?),
                direction: Direction::from_code(reader.word()?)
                    .ok_or(ProtoError::Message(Protocol::Socket, TCP_SHUTDOWN))?,
            }),
            other => Err(ProtoError::Message(Protocol::Socket, other)),
        }
    }
}

impl Reply {
    /// The label this reply is sent under, which is the label of the
    /// request it answers.
    #[must_use]
    pub const fn label(&self) -> Label {
        let message = match self {
            Reply::Interface(_) => INTERFACE,
            Reply::Resolved(_) => RESOLVE,
            Reply::Bound(_) => UDP_BIND,
            Reply::UdpSent(_) => UDP_SEND_TO,
            Reply::UdpClosed(_) => UDP_CLOSE,
            Reply::Connected(_) => TCP_CONNECT,
            Reply::Listening(_) => TCP_LISTEN,
            Reply::Accepted(_) => TCP_ACCEPT,
            Reply::Sent(_) => TCP_SEND,
            Reply::Received(_) => TCP_RECV,
            Reply::ShutDown(_) => TCP_SHUTDOWN,
            Reply::Closed(_) => TCP_CLOSE,
            Reply::State(_) => TCP_STATE,
        };
        Label::new(Protocol::Socket, message)
    }

    /// Writes the reply into `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError::Codec`] when the message area or the handle area has
    /// no room.
    pub fn encode(&self, buffer: &mut BufferMut<'_>) -> Result<(), ProtoError> {
        let mut writer = Writer::new();
        match self {
            Reply::Interface(Ok(interface)) => {
                writer.word(buffer, 0)?;
                writer.word(buffer, u64::from(interface.lease))?;
                writer.bytes(buffer, &interface.mac.octets())?;
                write_addresses(&mut writer, buffer, &interface.addresses)?;
                let mut gateway = Addresses::new();
                if let Some(router) = interface.gateway {
                    let _kept = gateway.push(router);
                }
                write_addresses(&mut writer, buffer, &gateway)?;
            }
            Reply::Resolved(Ok(addresses)) => {
                writer.word(buffer, 0)?;
                write_addresses(&mut writer, buffer, addresses)?;
            }
            Reply::Bound(Ok(opened))
            | Reply::Connected(Ok(opened))
            | Reply::Accepted(Ok(opened)) => {
                writer.word(buffer, 0)?;
                writer.word(buffer, u64::from(opened.socket))?;
                writer.handle(buffer, opened.rings)?;
            }
            Reply::Listening(Ok(socket)) => {
                writer.word(buffer, 0)?;
                writer.word(buffer, u64::from(*socket))?;
            }
            Reply::UdpSent(Ok(moved)) | Reply::Sent(Ok(moved)) | Reply::Received(Ok(moved)) => {
                writer.word(buffer, 0)?;
                writer.word(buffer, u64::from(*moved))?;
            }
            Reply::State(Ok(state)) => {
                writer.word(buffer, 0)?;
                writer.word(buffer, state.code())?;
            }
            Reply::UdpClosed(outcome) | Reply::ShutDown(outcome) | Reply::Closed(outcome) => {
                writer.word(buffer, status_word(*outcome))?;
            }
            Reply::Interface(Err(error))
            | Reply::Resolved(Err(error))
            | Reply::Bound(Err(error))
            | Reply::UdpSent(Err(error))
            | Reply::Connected(Err(error))
            | Reply::Listening(Err(error))
            | Reply::Accepted(Err(error))
            | Reply::Sent(Err(error))
            | Reply::Received(Err(error))
            | Reply::State(Err(error)) => writer.word(buffer, u64::from(error.code()))?,
        }
        writer.finish(buffer, self.label().raw())?;
        Ok(())
    }

    /// Reads a reply out of `buffer`.
    ///
    /// # Errors
    ///
    /// [`ProtoError`] as [`Request::decode`], and [`ProtoError::Status`]
    /// for a status word that names no error.
    pub fn decode(buffer: Buffer<'_>) -> Result<Self, ProtoError> {
        let mut reader = Reader::new(buffer)?;
        let label = expect(reader.label())?;
        let outcome = status_of(reader.word()?)?;
        match (label.message, outcome) {
            (UDP_CLOSE, outcome) => Ok(Reply::UdpClosed(outcome)),
            (TCP_SHUTDOWN, outcome) => Ok(Reply::ShutDown(outcome)),
            (TCP_CLOSE, outcome) => Ok(Reply::Closed(outcome)),
            (INTERFACE, Err(error)) => Ok(Reply::Interface(Err(error))),
            (RESOLVE, Err(error)) => Ok(Reply::Resolved(Err(error))),
            (UDP_BIND, Err(error)) => Ok(Reply::Bound(Err(error))),
            (UDP_SEND_TO, Err(error)) => Ok(Reply::UdpSent(Err(error))),
            (TCP_CONNECT, Err(error)) => Ok(Reply::Connected(Err(error))),
            (TCP_LISTEN, Err(error)) => Ok(Reply::Listening(Err(error))),
            (TCP_ACCEPT, Err(error)) => Ok(Reply::Accepted(Err(error))),
            (TCP_SEND, Err(error)) => Ok(Reply::Sent(Err(error))),
            (TCP_RECV, Err(error)) => Ok(Reply::Received(Err(error))),
            (TCP_STATE, Err(error)) => Ok(Reply::State(Err(error))),
            (INTERFACE, Ok(())) => {
                let lease = reader.word()? != 0;
                let mut octets = [0u8; 6];
                let len = reader.bytes(&mut octets)?;
                if len != octets.len() {
                    return Err(ProtoError::TooLong {
                        len,
                        capacity: octets.len(),
                    });
                }
                let addresses = read_addresses(&mut reader)?;
                Ok(Reply::Interface(Ok(Interface {
                    mac: MacAddr::new(octets),
                    addresses,
                    gateway: read_addresses(&mut reader)?.iter().next(),
                    lease,
                })))
            }
            (RESOLVE, Ok(())) => Ok(Reply::Resolved(Ok(read_addresses(&mut reader)?))),
            (UDP_BIND, Ok(())) => Ok(Reply::Bound(Ok(read_opened(&mut reader)?))),
            (TCP_CONNECT, Ok(())) => Ok(Reply::Connected(Ok(read_opened(&mut reader)?))),
            (TCP_ACCEPT, Ok(())) => Ok(Reply::Accepted(Ok(read_opened(&mut reader)?))),
            (TCP_LISTEN, Ok(())) => Ok(Reply::Listening(Ok(word32(reader.word()?)))),
            (UDP_SEND_TO, Ok(())) => Ok(Reply::UdpSent(Ok(word32(reader.word()?)))),
            (TCP_SEND, Ok(())) => Ok(Reply::Sent(Ok(word32(reader.word()?)))),
            (TCP_RECV, Ok(())) => Ok(Reply::Received(Ok(word32(reader.word()?)))),
            (TCP_STATE, Ok(())) => Ok(Reply::State(Ok(State::from_code(reader.word()?)
                .ok_or(ProtoError::Message(Protocol::Socket, TCP_STATE))?))),
            (other, _) => Err(ProtoError::Message(Protocol::Socket, other)),
        }
    }
}

/// The refusal that answers `request`.
///
/// A server that cannot serve at all — one on a machine with no interface —
/// answers every request with this, so that a client reads the refusal
/// under the label of what it asked rather than under another message's.
#[must_use]
pub const fn refuse(request: &Request, error: Error) -> Reply {
    match request {
        Request::Interface => Reply::Interface(Err(error)),
        Request::Resolve { .. } => Reply::Resolved(Err(error)),
        Request::UdpBind { .. } => Reply::Bound(Err(error)),
        Request::UdpSendTo { .. } => Reply::UdpSent(Err(error)),
        Request::UdpClose { .. } => Reply::UdpClosed(Err(error)),
        Request::TcpConnect { .. } => Reply::Connected(Err(error)),
        Request::TcpListen { .. } => Reply::Listening(Err(error)),
        Request::TcpAccept { .. } => Reply::Accepted(Err(error)),
        Request::TcpSend { .. } => Reply::Sent(Err(error)),
        Request::TcpRecv { .. } => Reply::Received(Err(error)),
        Request::TcpShutdown { .. } => Reply::ShutDown(Err(error)),
        Request::TcpClose { .. } => Reply::Closed(Err(error)),
        Request::TcpState { .. } => Reply::State(Err(error)),
    }
}

/// The socket and its rings that stand next in the message.
fn read_opened(reader: &mut Reader<'_>) -> Result<Opened, ProtoError> {
    let socket = word32(reader.word()?);
    Ok(Opened {
        socket,
        rings: reader.handle()?,
    })
}

/// Bytes of the record a datagram stands behind in the inbound ring.
///
/// Fixed, so that a reader that has the header has every field of it: how
/// many bytes the payload is, which port and address it came from, and
/// sixteen bytes of address whatever the family. A reader takes the header,
/// then waits until the ring holds the payload the header names.
pub const DATAGRAM_HEADER_LEN: usize = 24;

/// The record header of a datagram of `len` bytes from `from`.
#[must_use]
pub fn datagram_header(from: Endpoint, len: u16) -> [u8; DATAGRAM_HEADER_LEN] {
    let mut header = [0u8; DATAGRAM_HEADER_LEN];
    let mut put = |at: usize, bytes: &[u8]| {
        if let Some(slot) = header.get_mut(at..at.saturating_add(bytes.len())) {
            slot.copy_from_slice(bytes);
        }
    };
    put(0, &len.to_le_bytes());
    put(2, &from.port.to_le_bytes());
    match from.address {
        IpAddr::V4(address) => {
            put(4, &[4]);
            put(8, &address.octets());
        }
        IpAddr::V6(address) => {
            put(4, &[6]);
            put(8, &address.octets());
        }
    }
    header
}

/// What a record header says: where the datagram came from and how many
/// bytes follow it, or `None` for a header whose family names none.
#[must_use]
pub fn read_datagram_header(header: &[u8; DATAGRAM_HEADER_LEN]) -> Option<(Endpoint, usize)> {
    let at = |index: usize| header.get(index).copied().unwrap_or(0);
    let len = u16::from_le_bytes([at(0), at(1)]);
    let port = u16::from_le_bytes([at(2), at(3)]);
    let mut octets = [0u8; 16];
    for (step, slot) in octets.iter_mut().enumerate() {
        *slot = at(step.saturating_add(8));
    }
    let address = match at(4) {
        4 => IpAddr::V4(Ipv4Addr::from_octets([
            octets[0], octets[1], octets[2], octets[3],
        ])),
        6 => IpAddr::V6(Ipv6Addr::from_octets(octets)),
        _other => return None,
    };
    Some((Endpoint { address, port }, usize::from(len)))
}

/// The address family of an IPv4 address, as a message carries it.
const FAMILY_V4: u64 = 4;

/// The address family of an IPv6 address.
const FAMILY_V6: u64 = 6;

/// Writes one endpoint: the family, the port, and the octets.
fn write_endpoint(
    writer: &mut Writer,
    buffer: &mut BufferMut<'_>,
    endpoint: Endpoint,
) -> Result<(), ProtoError> {
    writer.word(buffer, u64::from(endpoint.port))?;
    write_address(writer, buffer, endpoint.address)
}

/// Writes one address: the family, then the octets.
fn write_address(
    writer: &mut Writer,
    buffer: &mut BufferMut<'_>,
    address: IpAddr,
) -> Result<(), ProtoError> {
    match address {
        IpAddr::V4(value) => {
            writer.word(buffer, FAMILY_V4)?;
            writer.bytes(buffer, &value.octets())?;
        }
        IpAddr::V6(value) => {
            writer.word(buffer, FAMILY_V6)?;
            writer.bytes(buffer, &value.octets())?;
        }
    }
    Ok(())
}

/// Writes a list of addresses: how many, then each of them.
fn write_addresses(
    writer: &mut Writer,
    buffer: &mut BufferMut<'_>,
    addresses: &Addresses,
) -> Result<(), ProtoError> {
    writer.word(buffer, u64::try_from(addresses.len()).unwrap_or(0))?;
    for address in addresses.iter() {
        write_address(writer, buffer, address)?;
    }
    Ok(())
}

/// The endpoint that stands next in the message.
fn read_endpoint(reader: &mut Reader<'_>) -> Result<Endpoint, ProtoError> {
    let port = port_of(reader.word()?);
    Ok(Endpoint {
        address: read_address(reader)?,
        port,
    })
}

/// The address that stands next in the message.
fn read_address(reader: &mut Reader<'_>) -> Result<IpAddr, ProtoError> {
    let family = reader.word()?;
    let mut octets = [0u8; 16];
    let len = reader.bytes(&mut octets)?;
    match (family, len) {
        (FAMILY_V4, 4) => Ok(IpAddr::V4(Ipv4Addr::from_octets([
            octets[0], octets[1], octets[2], octets[3],
        ]))),
        (FAMILY_V6, 16) => Ok(IpAddr::V6(Ipv6Addr::from_octets(octets))),
        _other => Err(ProtoError::Message(Protocol::Socket, 0)),
    }
}

/// The list of addresses that stands next in the message.
fn read_addresses(reader: &mut Reader<'_>) -> Result<Addresses, ProtoError> {
    let count = reader.word()?;
    if count > u64::try_from(MAX_ADDRESSES).unwrap_or(0) {
        return Err(ProtoError::TooLong {
            len: usize::try_from(count).unwrap_or(usize::MAX),
            capacity: MAX_ADDRESSES,
        });
    }
    let mut addresses = Addresses::new();
    for _ in 0..count {
        let _kept = addresses.push(read_address(reader)?);
    }
    Ok(addresses)
}

/// The name that stands next in the message.
fn name_of(reader: &mut Reader<'_>) -> Result<Name, ProtoError> {
    let mut into = [0u8; MAX_NAME];
    let len = reader.bytes(&mut into)?;
    Name::new(into.get(..len).unwrap_or(&[]))
}

/// The low half of a word, which is what every number of this protocol is.
fn word32(word: u64) -> u32 {
    u32::try_from(word & 0xFFFF_FFFF).unwrap_or(0)
}

/// The low sixteen bits of a word, which is what a port is.
fn port_of(word: u64) -> u16 {
    u16::try_from(word & 0xFFFF).unwrap_or(0)
}

/// Takes a label apart and insists it names this protocol.
fn expect(raw: u64) -> Result<Label, ProtoError> {
    let label = Label::parse(raw)?;
    if label.protocol != Protocol::Socket {
        return Err(ProtoError::WrongProtocol {
            expected: Protocol::Socket,
            found: label.protocol,
        });
    }
    Ok(label)
}
