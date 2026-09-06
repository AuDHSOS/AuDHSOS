// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The options of RFC 2132: a code, a length, and a body, one behind the
//! other until the end marker.
//!
//! Two codes are not that shape. Pad is one byte and means nothing, which
//! is how a block is aligned; end is one byte and stops the walk, and
//! RFC 2132, section 3.2 requires it, so a block that runs out without one
//! is a block that was cut and not a block that ended. Everything behind
//! the end marker is padding and is not read.
//!
//! An option whose length reaches past the block is an error and never a
//! short read. The length is a field the sender wrote, and believing a
//! shortened version of it would mean reading a body that stops where the
//! sender did not say it stops.

use net_wire::Writer;

use crate::error::DhcpError;

/// Which option (RFC 2132).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OptionCode(u8);

impl OptionCode {
    /// One byte of nothing, for alignment (section 3.1).
    pub const PAD: OptionCode = OptionCode(0);
    /// The subnet mask (section 3.3).
    pub const SUBNET_MASK: OptionCode = OptionCode(1);
    /// The routers of this link, best first (section 3.5).
    pub const ROUTER: OptionCode = OptionCode(3);
    /// The recursive name servers (section 3.8).
    pub const DOMAIN_NAME_SERVER: OptionCode = OptionCode(6);
    /// The address a client would like (section 9.1).
    pub const REQUESTED_ADDRESS: OptionCode = OptionCode(50);
    /// How long the lease lasts, in seconds (section 9.2).
    pub const LEASE_TIME: OptionCode = OptionCode(51);
    /// Which of the eight messages this is (section 9.6).
    pub const MESSAGE_TYPE: OptionCode = OptionCode(53);
    /// Which server the message is from or for (section 9.7).
    pub const SERVER_IDENTIFIER: OptionCode = OptionCode(54);
    /// The options the client would like to be told (section 9.8).
    pub const PARAMETER_LIST: OptionCode = OptionCode(55);
    /// The largest message this client can take (section 9.10).
    pub const MAX_MESSAGE_SIZE: OptionCode = OptionCode(57);
    /// T1, when renewal begins (section 9.11).
    pub const RENEWAL_TIME: OptionCode = OptionCode(58);
    /// T2, when rebinding begins (section 9.12).
    pub const REBINDING_TIME: OptionCode = OptionCode(59);
    /// The end of the block (section 3.2).
    pub const END: OptionCode = OptionCode(255);

    /// The option `value` names.
    #[must_use]
    pub const fn new(value: u8) -> OptionCode {
        OptionCode(value)
    }

    /// The number as it goes on the wire.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// The options of one block, in the order they stand.
#[derive(Clone, Debug)]
pub struct Options<'a> {
    /// The block, from behind the magic cookie to the end of the message.
    bytes: &'a [u8],
    /// How much of it has been walked.
    at: usize,
    /// Whether the end marker has been reached, or an error ended the
    /// walk.
    done: bool,
}

impl<'a> Options<'a> {
    /// The options in `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Options<'a> {
        Options {
            bytes,
            at: 0,
            done: false,
        }
    }
}

impl<'a> Iterator for Options<'a> {
    type Item = Result<(OptionCode, &'a [u8]), DhcpError>;

    fn next(&mut self) -> Option<Result<(OptionCode, &'a [u8]), DhcpError>> {
        loop {
            if self.done {
                return None;
            }
            let Some(&code) = self.bytes.get(self.at) else {
                // The block ran out before the end marker did.
                self.done = true;
                return Some(Err(DhcpError::MissingEnd));
            };
            let code = OptionCode(code);
            self.at = self.at.saturating_add(1);
            if code == OptionCode::END {
                self.done = true;
                return None;
            }
            if code == OptionCode::PAD {
                continue;
            }
            let Some(&len) = self.bytes.get(self.at) else {
                self.done = true;
                return Some(Err(DhcpError::TruncatedOption(code)));
            };
            let from = self.at.saturating_add(1);
            let to = from.saturating_add(usize::from(len));
            let Some(body) = self.bytes.get(from..to) else {
                self.done = true;
                return Some(Err(DhcpError::TruncatedOption(code)));
            };
            self.at = to;
            return Some(Ok((code, body)));
        }
    }
}

/// Writes one option.
///
/// # Errors
///
/// [`DhcpError::TooLong`] when the body is longer than the length field
/// can express, and [`DhcpError::Wire`] when the buffer has no room.
pub fn write_option(
    writer: &mut Writer<'_>,
    code: OptionCode,
    body: &[u8],
) -> Result<(), DhcpError> {
    let Ok(len) = u8::try_from(body.len()) else {
        return Err(DhcpError::TooLong(body.len()));
    };
    writer.write_u8(code.get())?;
    writer.write_u8(len)?;
    writer.write_bytes(body)?;
    Ok(())
}
