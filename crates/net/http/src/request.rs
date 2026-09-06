// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The request: a request line, a `Host` field, and whatever else the
//! caller asks for, written into the caller's buffer.
//!
//! Every field is checked before a byte of it goes down. A value carrying
//! a carriage return is a value that ends its field and begins another,
//! so a client that writes one has let whoever supplied it write a header
//! of its own choosing — which is response splitting from the sending end.
//! The same grammar that refuses such a value on the way in refuses it
//! here.
//!
//! `Host` is written from the field of this type and never from the
//! caller's header list, because RFC 9112, section 3.2 has exactly one of
//! them in a message and two would be a message two recipients could read
//! differently. `Content-Length` is written the same way, for the same
//! reason.

use net_wire::Writer;

use crate::error::HttpError;
use crate::field::{is_token, is_value, same_name};

/// Which method. Only the ones a client of this system sends are here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Method {
    /// Fetch a representation.
    Get,
    /// Fetch the head of one, which has no body whatever the fields say.
    Head,
    /// Send a representation.
    Post,
}

impl Method {
    /// The word that goes on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Head => "HEAD",
            Method::Post => "POST",
        }
    }

    /// Whether a response to this method carries a body, whatever its
    /// fields say (RFC 9112, section 6.3, point 1).
    #[must_use]
    pub const fn expects_body(self) -> bool {
        !matches!(self, Method::Head)
    }
}

/// The longest decimal a `usize` takes.
const DIGITS: usize = 20;

/// What goes out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request<'a> {
    /// Which method.
    pub method: Method,
    /// The origin-form target: an absolute path with an optional query
    /// (RFC 9112, section 3.2.1).
    pub target: &'a str,
    /// What goes in the `Host` field.
    pub host: &'a str,
    /// The other fields, in the order they are written.
    pub headers: &'a [(&'a str, &'a str)],
    /// The body, which is written behind the fields with a
    /// `Content-Length` in front of it.
    pub body: &'a [u8],
}

impl<'a> Request<'a> {
    /// A `GET` of `target` at `host`, with no fields of its own.
    #[must_use]
    pub const fn get(target: &'a str, host: &'a str) -> Request<'a> {
        Request {
            method: Method::Get,
            target,
            host,
            headers: &[],
            body: &[],
        }
    }

    /// Writes the request.
    ///
    /// # Errors
    ///
    /// [`HttpError::Target`] for a target that is not one,
    /// [`HttpError::HeaderName`] for a name that is not a token or is one
    /// this method writes itself, [`HttpError::HeaderValue`] for a value
    /// carrying a control character or an empty host, and
    /// [`HttpError::Wire`] when the buffer has no room.
    pub fn write(&self, writer: &mut Writer<'_>) -> Result<(), HttpError> {
        check_target(self.target)?;
        if self.host.is_empty() || !is_value(self.host) {
            return Err(HttpError::HeaderValue);
        }
        for (name, value) in self.headers {
            if !is_token(name)
                || same_name(name, "host")
                || same_name(name, "content-length")
                || same_name(name, "transfer-encoding")
            {
                return Err(HttpError::HeaderName);
            }
            if !is_value(value) {
                return Err(HttpError::HeaderValue);
            }
        }
        writer.write_bytes(self.method.as_str().as_bytes())?;
        writer.write_u8(b' ')?;
        writer.write_bytes(self.target.as_bytes())?;
        writer.write_bytes(b" HTTP/1.1\r\n")?;
        field(writer, "Host", self.host)?;
        for (name, value) in self.headers {
            field(writer, name, value)?;
        }
        if !self.body.is_empty() {
            let mut digits = [0u8; DIGITS];
            let text = decimal(self.body.len(), &mut digits);
            field(writer, "Content-Length", text)?;
        }
        writer.write_bytes(b"\r\n")?;
        writer.write_bytes(self.body)?;
        Ok(())
    }
}

/// Writes one field and the two bytes that end it.
fn field(writer: &mut Writer<'_>, name: &str, value: &str) -> Result<(), HttpError> {
    writer.write_bytes(name.as_bytes())?;
    writer.write_bytes(b": ")?;
    writer.write_bytes(value.as_bytes())?;
    writer.write_bytes(b"\r\n")?;
    Ok(())
}

/// Whether `target` is an origin-form target: a path that begins with a
/// slash and carries no byte that would end the request line.
fn check_target(target: &str) -> Result<(), HttpError> {
    let usable = target.bytes().all(|byte| byte.is_ascii_graphic());
    if target.starts_with('/') && usable {
        Ok(())
    } else {
        Err(HttpError::Target)
    }
}

/// `value` as decimal digits, written into the back of `digits`.
fn decimal(value: usize, digits: &mut [u8; DIGITS]) -> &str {
    let mut at = DIGITS;
    let mut left = value;
    loop {
        at = at.saturating_sub(1);
        let digit = u8::try_from(left.checked_rem(10).unwrap_or(0)).unwrap_or(0);
        if let Some(slot) = digits.get_mut(at) {
            *slot = b'0'.saturating_add(digit);
        }
        left = left.checked_div(10).unwrap_or(0);
        if left == 0 {
            break;
        }
    }
    core::str::from_utf8(digits.get(at..).unwrap_or(&[])).unwrap_or("0")
}
