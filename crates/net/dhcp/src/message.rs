// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The message of RFC 2131, section 2: the fixed BOOTP part of 236 bytes,
//! the magic cookie of RFC 2132, section 2, and the option block behind
//! it.
//!
//! Two of the fixed fields are read and refused rather than believed. The
//! hardware type and length must be the six bytes of Ethernet, because a
//! reply carries the client's hardware address in a sixteen-byte field and
//! a client that reads six of a longer one is reading an address for a
//! link this system has no frames for. And the magic cookie must be the
//! one of RFC 2132: without it the bytes behind it are the vendor field of
//! plain BOOTP and not options at all.
//!
//! `sname` and `file` are skipped. RFC 2132, section 9.3 lets a server
//! overload them with options when the option field runs out, and this
//! client does not read them: the options it needs are the six of a lease,
//! which fit the option field of every message with room to spare.

use net_wire::{Ipv4Addr, MacAddr, Reader, Writer};

use crate::error::DhcpError;
use crate::option::Options;

/// The fixed part, from `op` to the end of `file`.
pub const FIXED_LEN: usize = 236;

/// What tells an option block from the vendor field of BOOTP
/// (RFC 2132, section 2).
pub const MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];

/// The shortest message this client writes. RFC 951 has a BOOTP message
/// of 300 bytes, and relay agents in the field still expect one, so the
/// option block is padded out to it.
pub const MIN_MESSAGE_LEN: usize = 300;

/// The longest message this client reads. RFC 2131, section 2 has every
/// client be ready for the 576 bytes of the smallest datagram RFC 791
/// requires a host to accept; this is that, less the twenty bytes of an
/// IPv4 header and the eight of a UDP one.
pub const MAX_MESSAGE_LEN: usize = 548;

/// Ethernet, as the "Assigned Numbers" table has it.
pub const HARDWARE_ETHERNET: u8 = 1;

/// The one bit of the flags field that has a meaning (RFC 2131, figure 2).
const FLAG_BROADCAST: u16 = 0x8000;

/// The length of an Ethernet address, as the `hlen` field carries it.
const HARDWARE_LEN: u8 = 6;

/// How much of the sixteen-byte `chaddr` field is padding behind an
/// Ethernet address.
const CHADDR_PAD: usize = 10;

/// How many bytes `sname` and `file` take together.
const NAMES_LEN: usize = 192;

/// Which direction a message goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Op(u8);

impl Op {
    /// From a client to a server.
    pub const REQUEST: Op = Op(1);
    /// From a server to a client.
    pub const REPLY: Op = Op(2);

    /// The number as it goes on the wire.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Which of the messages of RFC 2131 this is (RFC 2132, section 9.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MessageType(u8);

impl MessageType {
    /// A client looking for a server.
    pub const DISCOVER: MessageType = MessageType(1);
    /// A server offering an address.
    pub const OFFER: MessageType = MessageType(2);
    /// A client asking for one.
    pub const REQUEST: MessageType = MessageType(3);
    /// A client saying the address is already in use.
    pub const DECLINE: MessageType = MessageType(4);
    /// A server granting one.
    pub const ACK: MessageType = MessageType(5);
    /// A server refusing.
    pub const NAK: MessageType = MessageType(6);
    /// A client giving one back.
    pub const RELEASE: MessageType = MessageType(7);
    /// A client asking for parameters and no address.
    pub const INFORM: MessageType = MessageType(8);

    /// The type `value` names.
    #[must_use]
    pub const fn new(value: u8) -> MessageType {
        MessageType(value)
    }

    /// The number as it goes on the wire.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// One message, with its option block borrowed from the bytes it arrived
/// in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Message<'a> {
    /// Which direction it goes.
    pub op: Op,
    /// What ties a reply to the message it answers.
    pub xid: u32,
    /// How long the client has been at this, in seconds.
    pub secs: u16,
    /// Whether the client asks for the reply to be broadcast.
    pub broadcast: bool,
    /// The address the client already holds, if it holds one.
    pub client: Ipv4Addr,
    /// The address the server is handing out.
    pub yours: Ipv4Addr,
    /// The next server in a boot sequence, which this client ignores.
    pub next_server: Ipv4Addr,
    /// The relay agent, if the message came through one.
    pub relay: Ipv4Addr,
    /// The client's hardware address.
    pub hardware: MacAddr,
    /// The option block, from behind the magic cookie to the end.
    pub options: &'a [u8],
}

impl<'a> Message<'a> {
    /// An empty message from this client, with `hardware` as its address.
    #[must_use]
    pub const fn request(xid: u32, hardware: MacAddr) -> Message<'a> {
        Message {
            op: Op::REQUEST,
            xid,
            secs: 0,
            broadcast: false,
            client: Ipv4Addr::UNSPECIFIED,
            yours: Ipv4Addr::UNSPECIFIED,
            next_server: Ipv4Addr::UNSPECIFIED,
            relay: Ipv4Addr::UNSPECIFIED,
            hardware,
            options: &[],
        }
    }

    /// The message in `bytes`.
    ///
    /// # Errors
    ///
    /// [`DhcpError::Wire`] when there is not even a fixed part and a
    /// cookie, [`DhcpError::Op`] for an op code that is neither direction,
    /// [`DhcpError::Hardware`] for a link this client does not speak over,
    /// and [`DhcpError::Cookie`] when the four bytes before the options
    /// are other bytes.
    pub fn parse(bytes: &'a [u8]) -> Result<Message<'a>, DhcpError> {
        let mut reader = Reader::new(bytes);
        let op = Op(reader.read_u8()?);
        if op != Op::REQUEST && op != Op::REPLY {
            return Err(DhcpError::Op(op.0));
        }
        let htype = reader.read_u8()?;
        let hlen = reader.read_u8()?;
        if htype != HARDWARE_ETHERNET || usize::from(hlen) != MacAddr::LEN {
            return Err(DhcpError::Hardware { htype, hlen });
        }
        let _hops = reader.read_u8()?;
        let xid = reader.read_u32()?;
        let secs = reader.read_u16()?;
        let flags = reader.read_u16()?;
        let client = reader.read_ipv4()?;
        let yours = reader.read_ipv4()?;
        let next_server = reader.read_ipv4()?;
        let relay = reader.read_ipv4()?;
        let hardware = reader.read_mac()?;
        // The rest of `chaddr` is padding for a link with longer
        // addresses, and `sname` and `file` are not read (RFC 2132,
        // section 9.3).
        let _ = reader.read_bytes(CHADDR_PAD)?;
        let _ = reader.read_bytes(NAMES_LEN)?;
        let cookie = reader.read_array::<4>()?;
        if cookie != MAGIC_COOKIE {
            return Err(DhcpError::Cookie(cookie));
        }
        Ok(Message {
            op,
            xid,
            secs,
            broadcast: flags & FLAG_BROADCAST != 0,
            client,
            yours,
            next_server,
            relay,
            hardware,
            options: reader.rest(),
        })
    }

    /// Writes the message and pads it out to [`MIN_MESSAGE_LEN`].
    ///
    /// The option block is written as it stands, end marker and all, so a
    /// caller that assembles one is the caller that closes it.
    ///
    /// # Errors
    ///
    /// [`DhcpError::Wire`] when the buffer has no room.
    pub fn write(&self, writer: &mut Writer<'_>) -> Result<(), DhcpError> {
        let at = writer.position();
        writer.write_u8(self.op.0)?;
        writer.write_u8(HARDWARE_ETHERNET)?;
        writer.write_u8(HARDWARE_LEN)?;
        writer.write_u8(0)?;
        writer.write_u32(self.xid)?;
        writer.write_u16(self.secs)?;
        writer.write_u16(if self.broadcast { FLAG_BROADCAST } else { 0 })?;
        writer.write_ipv4(self.client)?;
        writer.write_ipv4(self.yours)?;
        writer.write_ipv4(self.next_server)?;
        writer.write_ipv4(self.relay)?;
        writer.write_mac(self.hardware)?;
        writer.write_zeros(CHADDR_PAD)?;
        writer.write_zeros(NAMES_LEN)?;
        writer.write_bytes(&MAGIC_COOKIE)?;
        writer.write_bytes(self.options)?;
        let written = writer.position().saturating_sub(at);
        writer.write_zeros(MIN_MESSAGE_LEN.saturating_sub(written))?;
        Ok(())
    }

    /// The options.
    #[must_use]
    pub const fn options(&self) -> Options<'a> {
        Options::new(self.options)
    }

    /// Which of the eight messages this is.
    ///
    /// # Errors
    ///
    /// [`DhcpError::MissingMessageType`] when the option is not there,
    /// [`DhcpError::OptionLength`] when it is not one byte, and whatever
    /// the option walk found wrong on the way.
    pub fn message_type(&self) -> Result<MessageType, DhcpError> {
        let mut found = None;
        for option in self.options() {
            let (code, body) = option?;
            if code == crate::option::OptionCode::MESSAGE_TYPE {
                let &[value] = body else {
                    return Err(DhcpError::OptionLength {
                        code,
                        len: body.len(),
                    });
                };
                found = Some(MessageType(value));
            }
        }
        found.ok_or(DhcpError::MissingMessageType)
    }
}
