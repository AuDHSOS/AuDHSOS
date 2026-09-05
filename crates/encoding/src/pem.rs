// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! PEM in the strict form of RFC 7468.
//!
//! What the strict form buys is that one block of bytes has one text. The
//! end line must name the label the begin line named, every body line but
//! the last is exactly 64 characters, a pad may appear only in the last of
//! them, and nothing but line terminators may follow the end line.
//!
//! One thing is deliberately lax, and the RFC says so: text *before* the
//! begin line is explanatory and is skipped. That is how a certificate
//! file with a human-readable preamble is read. Text *after* the end line
//! is not the same thing — appending to a file is how a reader is made to
//! see something the writer did not sign — so it is refused.
//!
//! Decoding borrows twice: the label points into the input, the bytes into
//! the buffer the caller supplied.

use crate::base64;
use crate::error::EncodingError;

/// The characters a body line holds, except the last line of a block.
pub const LINE: usize = 64;

/// The bytes one full body line decodes to.
const LINE_BYTES: usize = 48;

/// What stands before a label on the begin line.
const BEGIN: &[u8] = b"-----BEGIN ";

/// What stands before a label on the end line.
const END: &[u8] = b"-----END ";

/// What stands after a label on either line.
const DASHES: &[u8] = b"-----";

/// A decoded block: its label, and its bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Block<'label, 'bytes> {
    /// The label both lines carried, borrowed from the input.
    pub label: &'label str,
    /// The bytes the body decoded to, borrowed from the caller's buffer.
    pub bytes: &'bytes [u8],
}

/// The number of bytes [`encode`] writes for a block, or `None` when that
/// count does not fit a `usize`.
#[must_use]
pub fn encoded_len(label: &str, bytes: usize) -> Option<usize> {
    let body = base64::encoded_len(bytes)?;
    let lines = body.checked_add(LINE.wrapping_sub(1))?.checked_div(LINE)?;
    // `-----BEGIN ` + label + `-----\n` and `-----END ` + label + `-----\n`.
    let frame = BEGIN
        .len()
        .checked_add(END.len())?
        .checked_add(DASHES.len().checked_mul(2)?)?
        .checked_add(2)?
        .checked_add(label.len().checked_mul(2)?)?;
    frame.checked_add(body)?.checked_add(lines)
}

/// Writes `bytes` as a PEM block with `label` into `out` and answers how
/// many bytes it wrote. Lines end with a single newline and carry
/// [`LINE`] characters except the last.
///
/// # Errors
///
/// [`EncodingError::Label`] for a label RFC 7468 does not allow,
/// [`EncodingError::EmptyPayload`] for no bytes, and
/// [`EncodingError::BufferTooSmall`] when `out` is shorter than
/// [`encoded_len`], in which case nothing is written.
pub fn encode(label: &str, bytes: &[u8], out: &mut [u8]) -> Result<usize, EncodingError> {
    check_label(label.as_bytes())?;
    if bytes.is_empty() {
        return Err(EncodingError::EmptyPayload);
    }
    let needed = encoded_len(label, bytes.len()).ok_or(EncodingError::BufferTooSmall)?;
    if out.len() < needed {
        return Err(EncodingError::BufferTooSmall);
    }
    let mut at = 0usize;
    at = write(out, at, BEGIN)?;
    at = write(out, at, label.as_bytes())?;
    at = write(out, at, DASHES)?;
    at = write(out, at, b"\n")?;
    // The body goes straight into `out` at the position it belongs, one
    // full line at a time, so that no intermediate buffer is needed.
    for chunk in bytes.chunks(LINE_BYTES) {
        let room = out.get_mut(at..).ok_or(EncodingError::BufferTooSmall)?;
        let written = base64::encode(chunk, room)?;
        at = at
            .checked_add(written)
            .ok_or(EncodingError::BufferTooSmall)?;
        at = write(out, at, b"\n")?;
    }
    at = write(out, at, END)?;
    at = write(out, at, label.as_bytes())?;
    at = write(out, at, DASHES)?;
    at = write(out, at, b"\n")?;
    Ok(at)
}

/// Reads the first PEM block of `input`, writing its bytes into `out`.
///
/// # Errors
///
/// [`EncodingError::MissingBegin`] or [`EncodingError::MissingEnd`] when a
/// frame line is absent, [`EncodingError::LabelMismatch`] when the two
/// disagree, [`EncodingError::Label`] for a label RFC 7468 does not allow,
/// [`EncodingError::LineLength`] for a body line before the last that is
/// not [`LINE`] characters, [`EncodingError::EmptyPayload`] for a block
/// with no body, [`EncodingError::TrailingData`] for anything but line
/// terminators after the end line, whatever [`base64::decode`] reports for
/// the body, and [`EncodingError::BufferTooSmall`] when `out` is too
/// short.
pub fn decode<'label, 'bytes>(
    input: &'label [u8],
    out: &'bytes mut [u8],
) -> Result<Block<'label, 'bytes>, EncodingError> {
    let mut lines = Lines::new(input);
    let label = loop {
        let line = lines.next().ok_or(EncodingError::MissingBegin)?;
        if let Some(label) = framed(line, BEGIN) {
            break label;
        }
    };
    check_label(label)?;
    let label = core::str::from_utf8(label).map_err(|_| EncodingError::Label)?;

    // The body is found before it is read. A text that never closes is
    // missing its end line, which is what a caller needs to hear; a
    // complaint about the length of a line inside it would name a rule
    // that only applies to a block that has one.
    let body = lines.rest;
    let closing;
    let closed = loop {
        let start_of_line = lines.rest;
        let line = lines.next().ok_or(EncodingError::MissingEnd)?;
        if let Some(end) = framed(line, END) {
            closing = start_of_line;
            break end;
        }
    };
    if closed != label.as_bytes() {
        return Err(EncodingError::LabelMismatch);
    }
    for line in lines {
        if !line.is_empty() {
            return Err(EncodingError::TrailingData);
        }
    }

    let span = body
        .len()
        .checked_sub(closing.len())
        .ok_or(EncodingError::MissingEnd)?;
    let body = body.get(..span).ok_or(EncodingError::MissingEnd)?;
    let body = body.strip_suffix(b"\n").unwrap_or(body);
    let body = body.strip_suffix(b"\r").unwrap_or(body);
    if body.is_empty() {
        return Err(EncodingError::EmptyPayload);
    }

    // A line is decoded only once the next one is known, because the rule
    // for the last body line is not the rule for the others.
    let mut written = 0usize;
    let mut body_lines = Lines::new(body);
    let mut previous = body_lines.next().ok_or(EncodingError::EmptyPayload)?;
    for line in body_lines {
        written = take(previous, out, written, false)?;
        previous = line;
    }
    written = take(previous, out, written, true)?;
    let bytes = out.get(..written).ok_or(EncodingError::BufferTooSmall)?;
    Ok(Block { label, bytes })
}

/// Decodes one body line into `out` at `written` and answers the next
/// position. Only the last line of a block may be short, and only it may
/// carry a pad.
fn take(line: &[u8], out: &mut [u8], written: usize, last: bool) -> Result<usize, EncodingError> {
    if last {
        if line.is_empty() || line.len() > LINE {
            return Err(EncodingError::LineLength(line.len()));
        }
    } else {
        if line.len() != LINE {
            return Err(EncodingError::LineLength(line.len()));
        }
        if line.contains(&b'=') {
            return Err(EncodingError::Padding);
        }
    }
    let room = out
        .get_mut(written..)
        .ok_or(EncodingError::BufferTooSmall)?;
    let bytes = base64::decode(line, room)?;
    written
        .checked_add(bytes)
        .ok_or(EncodingError::BufferTooSmall)
}

/// The label of a frame line, or `None` when the line is not one.
fn framed<'a>(line: &'a [u8], opening: &[u8]) -> Option<&'a [u8]> {
    let rest = line.strip_prefix(opening)?;
    rest.strip_suffix(DASHES)
}

/// Checks a label against the characters RFC 7468 allows: printable ASCII
/// without the hyphen, with single spaces between words and none at either
/// edge.
fn check_label(label: &[u8]) -> Result<(), EncodingError> {
    if label.first() == Some(&b' ') || label.last() == Some(&b' ') {
        return Err(EncodingError::Label);
    }
    let mut previous_space = false;
    for byte in label {
        let space = *byte == b' ';
        if space && previous_space {
            return Err(EncodingError::Label);
        }
        if !space && !matches!(byte, 0x21..=0x2C | 0x2E..=0x7E) {
            return Err(EncodingError::Label);
        }
        previous_space = space;
    }
    Ok(())
}

/// Writes `what` at `at` and answers the next position.
fn write(out: &mut [u8], at: usize, what: &[u8]) -> Result<usize, EncodingError> {
    let end = at
        .checked_add(what.len())
        .ok_or(EncodingError::BufferTooSmall)?;
    let room = out.get_mut(at..end).ok_or(EncodingError::BufferTooSmall)?;
    room.copy_from_slice(what);
    Ok(end)
}

/// The lines of a text, split at a newline, with a carriage return before
/// it removed. A text that does not end in a newline still yields its last
/// line.
struct Lines<'a> {
    rest: &'a [u8],
    done: bool,
}

impl<'a> Lines<'a> {
    const fn new(input: &'a [u8]) -> Lines<'a> {
        Lines {
            rest: input,
            done: false,
        }
    }
}

impl<'a> Iterator for Lines<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.done {
            return None;
        }
        let line = if let Some(at) = self.rest.iter().position(|byte| *byte == b'\n') {
            let (line, rest) = self.rest.split_at_checked(at)?;
            self.rest = rest.get(1..).unwrap_or(&[]);
            line
        } else {
            self.done = true;
            let line = self.rest;
            self.rest = &[];
            line
        };
        Some(line.strip_suffix(b"\r").unwrap_or(line))
    }
}
