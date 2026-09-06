// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Building the server's half, which this crate never writes.

use net_wire::{Ipv4Addr, MacAddr, Writer};

use crate::message::{MIN_MESSAGE_LEN, Message, MessageType, Op};
use crate::option::{OptionCode, write_option};

/// This host's hardware address.
pub(crate) const MAC: MacAddr = MacAddr::new([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);

/// The server.
pub(crate) const SERVER: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 1);

/// The address it hands out.
pub(crate) const OFFERED: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 50);

/// The mask that goes with it.
pub(crate) const MASK: Ipv4Addr = Ipv4Addr::new(255, 255, 255, 0);

/// One option to put in a reply.
pub(crate) struct Opt {
    /// Which option.
    pub(crate) code: OptionCode,
    /// Its body.
    pub(crate) body: Vec<u8>,
}

/// The option `code` with `body`.
pub(crate) fn opt(code: OptionCode, body: &[u8]) -> Opt {
    Opt {
        code,
        body: body.to_vec(),
    }
}

/// The lease-time, renewal-time and rebinding-time options for `seconds`.
pub(crate) fn lease_time(seconds: u32) -> Opt {
    opt(OptionCode::LEASE_TIME, &seconds.to_be_bytes())
}

/// A reply from the server, built out of the parts a test wants.
pub(crate) struct Reply {
    /// The transaction id it answers.
    pub(crate) xid: u32,
    /// Which of the eight it is.
    pub(crate) kind: MessageType,
    /// The address it hands out.
    pub(crate) yours: Ipv4Addr,
    /// Whose hardware address it names.
    pub(crate) hardware: MacAddr,
    /// The options behind the message type.
    pub(crate) options: Vec<Opt>,
}

impl Reply {
    /// An offer of [`OFFERED`] with a mask and a lease of an hour.
    pub(crate) fn offer(xid: u32) -> Reply {
        Reply {
            xid,
            kind: MessageType::OFFER,
            yours: OFFERED,
            hardware: MAC,
            options: vec![
                opt(OptionCode::SERVER_IDENTIFIER, &SERVER.octets()),
                opt(OptionCode::SUBNET_MASK, &MASK.octets()),
                lease_time(3600),
            ],
        }
    }

    /// An acknowledgment of the same, with a router and two name servers.
    pub(crate) fn ack(xid: u32) -> Reply {
        Reply {
            xid,
            kind: MessageType::ACK,
            yours: OFFERED,
            hardware: MAC,
            options: vec![
                opt(OptionCode::SERVER_IDENTIFIER, &SERVER.octets()),
                opt(OptionCode::SUBNET_MASK, &MASK.octets()),
                opt(OptionCode::ROUTER, &SERVER.octets()),
                opt(
                    OptionCode::DOMAIN_NAME_SERVER,
                    &[SERVER.octets(), Ipv4Addr::new(9, 9, 9, 9).octets()].concat(),
                ),
                lease_time(3600),
            ],
        }
    }

    /// A refusal.
    pub(crate) fn nak(xid: u32) -> Reply {
        Reply {
            xid,
            kind: MessageType::NAK,
            yours: Ipv4Addr::UNSPECIFIED,
            hardware: MAC,
            options: vec![opt(OptionCode::SERVER_IDENTIFIER, &SERVER.octets())],
        }
    }

    /// The same reply without the option `code`.
    pub(crate) fn without(mut self, code: OptionCode) -> Reply {
        self.options.retain(|option| option.code != code);
        self
    }

    /// The same reply with `option` added.
    pub(crate) fn with(mut self, option: Opt) -> Reply {
        self.options.push(option);
        self
    }

    /// The bytes of it.
    pub(crate) fn bytes(&self) -> Vec<u8> {
        let mut block = [0u8; 128];
        let mut options = Writer::new(&mut block);
        write_option(&mut options, OptionCode::MESSAGE_TYPE, &[self.kind.get()]).expect("room");
        for option in &self.options {
            write_option(&mut options, option.code, &option.body).expect("room");
        }
        options.write_u8(OptionCode::END.get()).expect("room");
        let len = options.position();
        let message = Message {
            op: Op::REPLY,
            xid: self.xid,
            secs: 0,
            broadcast: false,
            client: Ipv4Addr::UNSPECIFIED,
            yours: self.yours,
            next_server: Ipv4Addr::UNSPECIFIED,
            relay: Ipv4Addr::UNSPECIFIED,
            hardware: self.hardware,
            options: block.get(..len).expect("what was written"),
        };
        let mut buffer = [0u8; MIN_MESSAGE_LEN];
        let mut writer = Writer::new(&mut buffer);
        message.write(&mut writer).expect("room");
        writer.finish().to_vec()
    }
}
