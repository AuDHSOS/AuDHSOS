// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The identification string of RFC 4253, section 4.2: the first bytes
//! either side sends, before the binary packet protocol starts.
//!
//! The part of the string before the CR LF is what goes into the exchange
//! hash of section 8, so both sides' strings are kept after they are
//! parsed rather than read and dropped.

use crate::error::SshError;

/// The longest identification string, CR and LF included (section 4.2).
pub const MAX_LEN: usize = 255;

/// What every SSH-2 identification string begins with.
pub const PREFIX: &str = "SSH-2.0-";

/// What this client sends. The software version is printable US-ASCII
/// with no space and no minus in it, which is what section 4.2 requires
/// of that field.
pub const CLIENT: &str = concat!("SSH-2.0-AuDHSOS_", env!("CARGO_PKG_VERSION"));

/// What was at the front of the bytes that have arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Greeting<'a> {
    /// No line has ended yet, so there is nothing to judge.
    Incomplete,
    /// The peer's identification string, without its CR LF, and the bytes
    /// it took — the lines the server sent before it included.
    Identification {
        /// The string as it goes into the exchange hash.
        line: &'a str,
        /// Where the binary packet protocol starts.
        length: usize,
    },
}

/// Writes this client's identification string, CR LF and all.
///
/// # Errors
///
/// [`SshError::OutOfBounds`] when `out` is shorter than the string.
pub fn write(out: &mut [u8]) -> Result<usize, SshError> {
    let len = CLIENT.len().saturating_add(2);
    let available = out.len();
    let slot = out.get_mut(..len).ok_or(SshError::OutOfBounds {
        needed: len,
        available,
    })?;
    let (line, end) = slot.split_at_mut(CLIENT.len());
    line.copy_from_slice(CLIENT.as_bytes());
    end.copy_from_slice(b"\r\n");
    Ok(len)
}

/// Reads the peer's identification string out of what has arrived.
///
/// A server may send other lines first and they may not begin with
/// `SSH-`, so they are skipped and counted; the caller that wants to show
/// them has them in its own buffer. A peer that sends nothing but such
/// lines never completes, which is the caller's buffer to bound.
///
/// Every line is held to [`MAX_LEN`], which section 4.2 asks only of the
/// identification: the lines before it have no length in the document and
/// this reader has a buffer that does. The same limit is applied to a
/// line that has not ended yet, so what is refused does not depend on how
/// the bytes were split on the way here.
///
/// # Errors
///
/// [`SshError::Identification`] for a line that is too long, holds a byte
/// that is not printable US-ASCII, or names a protocol version this
/// client does not speak. RFC 4253, section 5.1, allows `SSH-1.99-` for a
/// server that speaks both versions; this client speaks one and says so.
pub fn read(bytes: &[u8]) -> Result<Greeting<'_>, SshError> {
    let mut start = 0usize;
    loop {
        let rest = bytes.get(start..).unwrap_or(&[]);
        let Some(end) = rest.windows(2).position(|pair| pair == b"\r\n") else {
            // Nothing else can be said until the line ends, but a line
            // that cannot be one is said now rather than buffered on.
            return if rest.len() >= MAX_LEN {
                Err(SshError::Identification)
            } else {
                Ok(Greeting::Incomplete)
            };
        };
        let line = rest.get(..end).unwrap_or(&[]);
        if end.saturating_add(2) > MAX_LEN {
            return Err(SshError::Identification);
        }
        let next = start.saturating_add(end).saturating_add(2);
        if line.starts_with(b"SSH-") {
            return identification(line, next);
        }
        start = next;
    }
}

/// The line that begins with `SSH-`, judged.
fn identification(line: &[u8], length: usize) -> Result<Greeting<'_>, SshError> {
    // The length is the loop's rule and is not judged again here, where
    // no line that broke it can arrive. UTF-8 first and printability
    // second, so that each answer is one a peer can produce.
    let line = core::str::from_utf8(line).map_err(|_| SshError::Identification)?;
    if !line.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        return Err(SshError::Identification);
    }
    let software = line.strip_prefix(PREFIX).ok_or(SshError::Identification)?;
    if software.is_empty() {
        return Err(SshError::Identification);
    }
    Ok(Greeting::Identification { line, length })
}
