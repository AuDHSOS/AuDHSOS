// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The message numbers of RFC 4250, section 4.1.2, and the one message
//! that is a number and text.
//!
//! Section 4.1.1 puts them in ranges: 1 to 19 transport generic, 20 to 29
//! algorithm negotiation, 30 to 49 specific to a key exchange method, 50
//! to 79 authentication, 80 to 127 connection. The two method-specific
//! ranges are reused by every method, so a byte in them means nothing
//! until one knows which method is running; those numbers are named where
//! the method is, not here.
//!
//! What is here is what the steps that are built use. The rest arrive
//! with the layer that sends them.

use crate::error::SshError;
use crate::wire::{Reader, Writer};

/// `SSH_MSG_DISCONNECT`.
pub const DISCONNECT: u8 = 1;

/// `SSH_MSG_IGNORE`.
pub const IGNORE: u8 = 2;

/// `SSH_MSG_UNIMPLEMENTED`.
pub const UNIMPLEMENTED: u8 = 3;

/// `SSH_MSG_DEBUG`.
pub const DEBUG: u8 = 4;

/// `SSH_MSG_SERVICE_REQUEST`.
pub const SERVICE_REQUEST: u8 = 5;

/// `SSH_MSG_SERVICE_ACCEPT`.
pub const SERVICE_ACCEPT: u8 = 6;

/// `SSH_MSG_EXT_INFO`, which is RFC 8308, section 2.3, and not RFC 4250.
pub const EXT_INFO: u8 = 7;

/// `SSH_MSG_KEXINIT`.
pub const KEXINIT: u8 = 20;

/// `SSH_MSG_NEWKEYS`.
pub const NEWKEYS: u8 = 21;

/// `SSH_MSG_USERAUTH_REQUEST`.
pub const USERAUTH_REQUEST: u8 = 50;

/// `SSH_MSG_USERAUTH_FAILURE`.
pub const USERAUTH_FAILURE: u8 = 51;

/// `SSH_MSG_USERAUTH_SUCCESS`.
pub const USERAUTH_SUCCESS: u8 = 52;

/// `SSH_MSG_USERAUTH_BANNER`.
pub const USERAUTH_BANNER: u8 = 53;

/// `SSH_MSG_GLOBAL_REQUEST`.
pub const GLOBAL_REQUEST: u8 = 80;

/// `SSH_MSG_REQUEST_SUCCESS`.
pub const REQUEST_SUCCESS: u8 = 81;

/// `SSH_MSG_REQUEST_FAILURE`.
pub const REQUEST_FAILURE: u8 = 82;

/// `SSH_MSG_CHANNEL_OPEN`.
pub const CHANNEL_OPEN: u8 = 90;

/// `SSH_MSG_CHANNEL_OPEN_CONFIRMATION`.
pub const CHANNEL_OPEN_CONFIRMATION: u8 = 91;

/// `SSH_MSG_CHANNEL_OPEN_FAILURE`.
pub const CHANNEL_OPEN_FAILURE: u8 = 92;

/// `SSH_MSG_CHANNEL_WINDOW_ADJUST`.
pub const CHANNEL_WINDOW_ADJUST: u8 = 93;

/// `SSH_MSG_CHANNEL_DATA`.
pub const CHANNEL_DATA: u8 = 94;

/// `SSH_MSG_CHANNEL_EXTENDED_DATA`.
pub const CHANNEL_EXTENDED_DATA: u8 = 95;

/// `SSH_MSG_CHANNEL_EOF`.
pub const CHANNEL_EOF: u8 = 96;

/// `SSH_MSG_CHANNEL_CLOSE`.
pub const CHANNEL_CLOSE: u8 = 97;

/// `SSH_MSG_CHANNEL_REQUEST`.
pub const CHANNEL_REQUEST: u8 = 98;

/// `SSH_MSG_CHANNEL_SUCCESS`.
pub const CHANNEL_SUCCESS: u8 = 99;

/// `SSH_MSG_CHANNEL_FAILURE`.
pub const CHANNEL_FAILURE: u8 = 100;

/// The reason codes of an `SSH_MSG_CHANNEL_OPEN_FAILURE` (RFC 4250,
/// section 4.3).
pub mod open {
    /// `SSH_OPEN_ADMINISTRATIVELY_PROHIBITED`.
    pub const ADMINISTRATIVELY_PROHIBITED: u32 = 1;

    /// `SSH_OPEN_CONNECT_FAILED`.
    pub const CONNECT_FAILED: u32 = 2;

    /// `SSH_OPEN_UNKNOWN_CHANNEL_TYPE`.
    pub const UNKNOWN_CHANNEL_TYPE: u32 = 3;

    /// `SSH_OPEN_RESOURCE_SHORTAGE`.
    pub const RESOURCE_SHORTAGE: u32 = 4;
}

/// The reason codes of a `SSH_MSG_DISCONNECT` (RFC 4250, section 4.2.2),
/// again only the ones the layers that are built name.
pub mod disconnect {
    /// `SSH_DISCONNECT_KEY_EXCHANGE_FAILED`, which RFC 8731, section 3,
    /// requires for a shared secret of all zeros and for a public value
    /// of the wrong length.
    pub const KEY_EXCHANGE_FAILED: u32 = 3;

    /// `SSH_DISCONNECT_MAC_ERROR`, which a tag that does not check is.
    pub const MAC_ERROR: u32 = 5;

    /// `SSH_DISCONNECT_HOST_KEY_NOT_VERIFIABLE`, for a host key no rule
    /// admits and for a signature that is not the peer's over the
    /// exchange hash.
    pub const HOST_KEY_NOT_VERIFIABLE: u32 = 9;

    /// `SSH_DISCONNECT_BY_APPLICATION`, which a client that is done sends.
    pub const BY_APPLICATION: u32 = 11;
}

/// A `SSH_MSG_DISCONNECT` (RFC 4253, section 11.1), which ends the
/// connection for both sides: after it neither may send and neither may
/// accept anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Disconnect<'a> {
    /// Why, as one of the codes in [`disconnect`].
    pub reason: u32,
    /// The text, which a peer wrote and which is bytes until something
    /// decides to show it.
    pub description: &'a [u8],
    /// The language tag, which may be empty.
    pub language: &'a [u8],
}

impl<'a> Disconnect<'a> {
    /// Reads one from a packet payload.
    ///
    /// # Errors
    ///
    /// [`SshError::Message`] when the payload is another message and
    /// [`SshError::OutOfBounds`] when it ends early.
    pub fn read(payload: &'a [u8]) -> Result<Disconnect<'a>, SshError> {
        let mut reader = Reader::new(payload);
        let number = reader.read_byte()?;
        if number != DISCONNECT {
            return Err(SshError::Message(number));
        }
        Ok(Disconnect {
            reason: reader.read_u32()?,
            description: reader.read_string()?,
            language: reader.read_string()?,
        })
    }

    /// Writes one, with no language tag, which section 11.1 permits and
    /// which is what a client with no text of its own to localize sends.
    ///
    /// # Errors
    ///
    /// [`SshError::OutOfBounds`] when `out` is too small.
    pub fn write(&self, out: &mut [u8]) -> Result<usize, SshError> {
        let mut writer = Writer::new(out);
        writer.write_byte(DISCONNECT)?;
        writer.write_u32(self.reason)?;
        writer.write_string(self.description)?;
        writer.write_string(self.language)?;
        Ok(writer.position())
    }
}
