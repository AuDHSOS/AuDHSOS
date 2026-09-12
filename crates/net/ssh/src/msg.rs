// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The message numbers of RFC 4250, section 4.1.2.
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

/// The reason codes of a `SSH_MSG_DISCONNECT` (RFC 4250, section 4.2.2),
/// again only the ones the layers that are built name.
pub mod disconnect {
    /// `SSH_DISCONNECT_KEY_EXCHANGE_FAILED`, which RFC 8731, section 3,
    /// requires for a shared secret of all zeros and for a public value
    /// of the wrong length.
    pub const KEY_EXCHANGE_FAILED: u32 = 3;

    /// `SSH_DISCONNECT_MAC_ERROR`, which a tag that does not check is.
    pub const MAC_ERROR: u32 = 5;
}
