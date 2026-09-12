// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! PEM in the strict form of RFC 7468, and the same frame at another
//! width.
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
//! Not every text in this frame is RFC 7468. `openssh-key-v1`, the file a
//! Secure Shell private key is written into, fixes no width at all: what
//! OpenSSH writes is seventy characters, and that is not a multiple of
//! four, so a line of such a text holds part of a Base64 quantum and
//! cannot be encoded or decoded on its own. [`encode_wrapped`] therefore
//! writes the body in one piece and pushes it apart, and the reader takes
//! the characters a quantum at a time and never a line at a time.
//! [`decode_wrapped`] is also the laxer of the two readers, and
//! deliberately: where no document fixes the width, the width is one
//! writer's choice, so a line is held to a maximum rather than to a
//! length.
//!
//! Decoding borrows twice: the label points into the input, the bytes into
//! the buffer the caller supplied.

use core::num::NonZeroUsize;

use crate::base64;
use crate::error::EncodingError;

/// The characters a body line holds, except the last line of a block.
pub const LINE: usize = 64;

/// [`LINE`] for the functions that take a width. The second arm is there
/// because a constant cannot unwrap; `LINE` is 64.
const STRICT: NonZeroUsize = match NonZeroUsize::new(LINE) {
    Some(width) => width,
    None => NonZeroUsize::MIN,
};

/// The pad character of Base64, which may stand only in the last line.
const PAD: u8 = b'=';

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
    encoded_len_wrapped(label, bytes, STRICT)
}

/// The number of bytes [`encode_wrapped`] writes for a block whose body
/// lines hold `wrap` characters, or `None` when that count does not fit a
/// `usize`.
#[must_use]
pub fn encoded_len_wrapped(label: &str, bytes: usize, wrap: NonZeroUsize) -> Option<usize> {
    let width = wrap.get();
    let body = base64::encoded_len(bytes)?;
    let lines = body
        .checked_add(width.wrapping_sub(1))?
        .checked_div(width)?;
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
    encode_wrapped(label, bytes, STRICT, out)
}

/// Writes `bytes` as a block with `label` whose body lines hold `wrap`
/// characters, and answers how many bytes it wrote. `wrap` of [`LINE`] is
/// what [`encode`] writes; seventy is what OpenSSH writes an
/// `openssh-key-v1` key with.
///
/// # Errors
///
/// [`EncodingError::Label`] for a label RFC 7468 does not allow,
/// [`EncodingError::EmptyPayload`] for no bytes, and
/// [`EncodingError::BufferTooSmall`] when `out` is shorter than
/// [`encoded_len_wrapped`], in which case nothing is written.
pub fn encode_wrapped(
    label: &str,
    bytes: &[u8],
    wrap: NonZeroUsize,
    out: &mut [u8],
) -> Result<usize, EncodingError> {
    check_label(label.as_bytes())?;
    if bytes.is_empty() {
        return Err(EncodingError::EmptyPayload);
    }
    let needed =
        encoded_len_wrapped(label, bytes.len(), wrap).ok_or(EncodingError::BufferTooSmall)?;
    if out.len() < needed {
        return Err(EncodingError::BufferTooSmall);
    }
    let mut at = 0usize;
    at = write(out, at, BEGIN)?;
    at = write(out, at, label.as_bytes())?;
    at = write(out, at, DASHES)?;
    at = write(out, at, b"\n")?;
    // The body is written as one Base64 text and then pushed apart into
    // lines, because a width that is not a multiple of four leaves a
    // quantum straddling two of them.
    let room = out.get_mut(at..).ok_or(EncodingError::BufferTooSmall)?;
    let body = base64::encode(bytes, room)?;
    at = split(out, at, body, wrap)?;
    at = write(out, at, END)?;
    at = write(out, at, label.as_bytes())?;
    at = write(out, at, DASHES)?;
    at = write(out, at, b"\n")?;
    Ok(at)
}

/// Pushes the `body` characters standing at `at` apart into lines of
/// `wrap` characters, each followed by a newline, and answers the position
/// after the last one. The lines move from the back, so that no character
/// is written over before it has moved.
fn split(
    out: &mut [u8],
    at: usize,
    body: usize,
    wrap: NonZeroUsize,
) -> Result<usize, EncodingError> {
    let width = wrap.get();
    let lines = body
        .checked_add(width.wrapping_sub(1))
        .and_then(|rounded| rounded.checked_div(width))
        .ok_or(EncodingError::BufferTooSmall)?;
    let mut line = lines;
    while let Some(index) = line.checked_sub(1) {
        line = index;
        let start = index
            .checked_mul(width)
            .ok_or(EncodingError::BufferTooSmall)?;
        let length = body
            .checked_sub(start)
            .ok_or(EncodingError::BufferTooSmall)?
            .min(width);
        let from = at.checked_add(start).ok_or(EncodingError::BufferTooSmall)?;
        let to = from
            .checked_add(index)
            .ok_or(EncodingError::BufferTooSmall)?;
        let end = from
            .checked_add(length)
            .ok_or(EncodingError::BufferTooSmall)?;
        let newline = to
            .checked_add(length)
            .ok_or(EncodingError::BufferTooSmall)?;
        let slot = out.get_mut(newline).ok_or(EncodingError::BufferTooSmall)?;
        *slot = b'\n';
        out.copy_within(from..end, to);
    }
    at.checked_add(body)
        .and_then(|written| written.checked_add(lines))
        .ok_or(EncodingError::BufferTooSmall)
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
    read(input, out, STRICT, true)
}

/// Reads the first block of `input` whose body lines are no longer than
/// `longest`, writing its bytes into `out`. A text written by
/// [`encode_wrapped`] at that width reads back here; so does one some
/// other writer wrapped more narrowly, because no standard fixes the width
/// of a text that is not RFC 7468.
///
/// # Errors
///
/// The errors of [`decode`], with [`EncodingError::LineLength`] for a body
/// line that is empty or longer than `longest`.
pub fn decode_wrapped<'label, 'bytes>(
    input: &'label [u8],
    out: &'bytes mut [u8],
    longest: NonZeroUsize,
) -> Result<Block<'label, 'bytes>, EncodingError> {
    read(input, out, longest, false)
}

/// Reads the first block of `input` under one rule for its body lines:
/// `exact` holds every line but the last to `wrap` characters, and without
/// it a line is held to that many at most.
fn read<'label, 'bytes>(
    input: &'label [u8],
    out: &'bytes mut [u8],
    wrap: NonZeroUsize,
    exact: bool,
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

    // A line is handed over only once the next one is known, because the
    // rule for the last body line is not the rule for the others.
    let mut quanta = Quanta::new(out);
    let mut body_lines = Lines::new(body);
    let mut previous = body_lines.next().ok_or(EncodingError::EmptyPayload)?;
    let written = loop {
        let next = body_lines.next();
        quanta.take(previous, wrap, exact, next.is_none())?;
        match next {
            Some(line) => previous = line,
            None => break quanta.finish()?,
        }
    };
    let bytes = out.get(..written).ok_or(EncodingError::BufferTooSmall)?;
    Ok(Block { label, bytes })
}

/// A body being decoded four characters at a time: the quantum being
/// filled, how much of it is filled, and where its bytes go.
struct Quanta<'out> {
    out: &'out mut [u8],
    quantum: [u8; 4],
    filled: usize,
    characters: usize,
    written: usize,
    padded: bool,
}

impl<'out> Quanta<'out> {
    /// A body with nothing read yet, writing into `out`.
    const fn new(out: &'out mut [u8]) -> Quanta<'out> {
        Quanta {
            out,
            quantum: [0u8; 4],
            filled: 0,
            characters: 0,
            written: 0,
            padded: false,
        }
    }

    /// Takes one body line, whose length is judged first.
    fn take(
        &mut self,
        line: &[u8],
        wrap: NonZeroUsize,
        exact: bool,
        last: bool,
    ) -> Result<(), EncodingError> {
        let width = wrap.get();
        let fits = if exact && !last {
            line.len() == width
        } else {
            !line.is_empty() && line.len() <= width
        };
        if !fits {
            return Err(EncodingError::LineLength {
                characters: line.len(),
                wrap: width,
            });
        }
        // Only the last line may carry a pad. Saying so here rather than
        // letting the quantum say it keeps the complaint about the pad and
        // not about the bits under it.
        if !last && line.contains(&PAD) {
            return Err(EncodingError::Padding);
        }
        for byte in line {
            self.push(*byte)?;
        }
        Ok(())
    }

    /// Takes one character, decoding the quantum it completes.
    fn push(&mut self, byte: u8) -> Result<(), EncodingError> {
        if self.padded {
            return Err(EncodingError::Padding);
        }
        let [_, second, third, fourth] = self.quantum;
        self.quantum = [second, third, fourth, byte];
        self.characters = self
            .characters
            .checked_add(1)
            .ok_or(EncodingError::BufferTooSmall)?;
        self.filled = self
            .filled
            .checked_add(1)
            .ok_or(EncodingError::BufferTooSmall)?;
        if self.filled == 4 {
            self.padded = self.quantum.contains(&PAD);
            let room = self
                .out
                .get_mut(self.written..)
                .ok_or(EncodingError::BufferTooSmall)?;
            let characters = self.characters;
            let bytes =
                base64::decode(&self.quantum, room).map_err(|error| in_body(error, characters))?;
            self.written = self
                .written
                .checked_add(bytes)
                .ok_or(EncodingError::BufferTooSmall)?;
            self.filled = 0;
        }
        Ok(())
    }

    /// How many bytes the body decoded to, once it has all been read.
    const fn finish(&self) -> Result<usize, EncodingError> {
        if self.filled == 0 {
            Ok(self.written)
        } else {
            Err(EncodingError::Length(self.characters))
        }
    }
}

/// A character error inside a quantum, moved to the offset it has in the
/// body. `characters` counts every body character read so far, the four of
/// this quantum included.
const fn in_body(error: EncodingError, characters: usize) -> EncodingError {
    match error {
        EncodingError::Character(at) => {
            EncodingError::Character(characters.wrapping_sub(4).wrapping_add(at))
        }
        other => other,
    }
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
