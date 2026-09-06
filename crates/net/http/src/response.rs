// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The response, decoded as it arrives.
//!
//! The decoder takes at most one line of the head per call. That is what
//! keeps a byte of the body from ever being copied into the buffer the
//! head is assembled in, and it is what makes a response split at any
//! boundary decode to what the whole of it decodes to: nothing here
//! depends on where a read ended.
//!
//! Framing is decided once, when the head is complete, in the order
//! RFC 9112, section 6.3 gives — with one departure. Point 3 of that list
//! lets a recipient prefer `Transfer-Encoding` over `Content-Length` and
//! says in the same paragraph that such a message ought to be handled as
//! an error; this decoder does the latter. Two readings of one message is
//! the whole of request smuggling, and a preference rule is a second
//! reading with a tie-breaker rather than one reading.
//!
//! What is left is four framings and no ambiguity. No body at all, for a
//! response to `HEAD` and for the statuses that cannot carry one. A
//! chunked body. A body of a declared length. And a body that runs until
//! the connection closes, which is point 8 of the same list and the
//! reason [`Decoder::finish`] exists: without a caller saying the peer
//! closed, a body that ended and a body that was cut off are the same
//! bytes.

use crate::error::HttpError;
use crate::field::{is_token, is_value, same_name, trim};
use crate::request::Method;

/// The longest status line this decoder reads.
///
/// A status line is a version, a code, and a reason phrase that RFC 9112,
/// section 4 lets a server leave out. Two hundred and fifty-six bytes is
/// past every reason phrase anyone writes and short enough that a server
/// cannot spend a caller's head buffer on one.
pub const MAX_STATUS_LINE: usize = 256;

/// The longest header line this decoder reads.
pub const MAX_HEADER_LINE: usize = 1024;

/// The longest chunk-size line and trailer line this decoder reads. A
/// chunk size is at most sixteen hex digits; the rest is room for the
/// extensions RFC 9112, section 7.1.1 allows and this decoder ignores.
const MAX_CHUNK_LINE: usize = 256;

/// Where a piece of text sits inside the head buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Span {
    /// Where it begins.
    start: usize,
    /// How long it is.
    len: usize,
}

impl Span {
    /// A span of nothing.
    const EMPTY: Span = Span { start: 0, len: 0 };

    /// What it points at, as text. Every byte that reaches the buffer has
    /// been checked to be ASCII, so this is text and not a guess.
    fn of(self, bytes: &[u8]) -> &str {
        let end = self.start.saturating_add(self.len);
        let slice = bytes.get(self.start..end).unwrap_or(&[]);
        core::str::from_utf8(slice).unwrap_or_default()
    }
}

/// A response status code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Status(u16);

impl Status {
    /// The code `value` names.
    #[must_use]
    pub const fn new(value: u16) -> Status {
        Status(value)
    }

    /// The number as it goes on the wire.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Whether it is a 1xx, which is a message the exchange goes on past.
    #[must_use]
    pub const fn is_informational(self) -> bool {
        self.0 >= 100 && self.0 < 200
    }

    /// Whether it is a 2xx.
    #[must_use]
    pub const fn is_successful(self) -> bool {
        self.0 >= 200 && self.0 < 300
    }

    /// Whether it is a 3xx.
    #[must_use]
    pub const fn is_redirection(self) -> bool {
        self.0 >= 300 && self.0 < 400
    }

    /// Whether it is a 4xx.
    #[must_use]
    pub const fn is_client_error(self) -> bool {
        self.0 >= 400 && self.0 < 500
    }

    /// Whether it is a 5xx.
    #[must_use]
    pub const fn is_server_error(self) -> bool {
        self.0 >= 500 && self.0 < 600
    }

    /// Whether it is one of the five codes that send a client somewhere
    /// else: 301, 302, 303, 307 and 308 (RFC 9110, section 15.4). 300 and
    /// 304 are 3xx and are not among them.
    #[must_use]
    pub const fn is_redirect(self) -> bool {
        matches!(self.0, 301 | 302 | 303 | 307 | 308)
    }

    /// Whether a response with this code carries a body, whatever its
    /// fields say (RFC 9112, section 6.3, point 1).
    #[must_use]
    pub const fn carries_body(self) -> bool {
        !self.is_informational() && self.0 != 204 && self.0 != 304
    }
}

/// What one call to [`Decoder::feed`] made of its input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event<'a> {
    /// Nothing can be said until more bytes arrive.
    NeedMore,
    /// The head is complete; [`Decoder::head`] has it.
    Head,
    /// A piece of the body, borrowed from the input it arrived in.
    Body(&'a [u8]),
    /// The message is complete.
    Done,
}

/// The head of a response: the status, the reason, and the fields.
#[derive(Clone, Copy, Debug)]
pub struct Head<'a> {
    /// The status code.
    pub status: Status,
    /// The bytes the spans point into.
    bytes: &'a [u8],
    /// Where the reason phrase is.
    reason: Span,
    /// The fields, in the order they arrived.
    fields: &'a [(Span, Span)],
}

impl<'a> Head<'a> {
    /// The reason phrase, which a server may leave empty.
    #[must_use]
    pub fn reason(&self) -> &'a str {
        self.reason.of(self.bytes)
    }

    /// The fields, in the order they arrived.
    #[must_use]
    pub const fn headers(&self) -> Headers<'a> {
        Headers {
            bytes: self.bytes,
            fields: self.fields,
            at: 0,
        }
    }

    /// The value of the first field called `name`, compared without
    /// regard to case.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&'a str> {
        self.headers()
            .find(|(seen, _)| same_name(seen, name))
            .map(|(_, value)| value)
    }

    /// Where the response sends the caller, when it is one of the five
    /// redirects and carries a `Location`.
    ///
    /// Nothing is followed. Which locations this host is willing to fetch
    /// from is a decision the caller makes, and a client that follows a
    /// redirect on its own has made it for them.
    #[must_use]
    pub fn redirect(&self) -> Option<&'a str> {
        self.status.is_redirect().then(|| self.header("location"))?
    }
}

/// The fields of a head.
#[derive(Clone, Debug)]
pub struct Headers<'a> {
    /// The bytes the spans point into.
    bytes: &'a [u8],
    /// The spans.
    fields: &'a [(Span, Span)],
    /// How many have been handed out.
    at: usize,
}

impl<'a> Iterator for Headers<'a> {
    type Item = (&'a str, &'a str);

    fn next(&mut self) -> Option<(&'a str, &'a str)> {
        let (name, value) = *self.fields.get(self.at)?;
        self.at = self.at.saturating_add(1);
        Some((name.of(self.bytes), value.of(self.bytes)))
    }
}

/// Where a chunked body stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Chunk {
    /// Reading the size line.
    Size,
    /// This many bytes of chunk data are still to come.
    Data(u64),
    /// The two bytes that end a chunk.
    After,
    /// Trailer lines, until an empty one.
    Trailer,
}

/// Where the decoder stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Reading the status line.
    Status,
    /// Reading field lines.
    Fields,
    /// A body of a declared length; this many bytes are left.
    Length(u64),
    /// A chunked body.
    Chunked(Chunk),
    /// A body that ends when the connection does.
    Close,
    /// The message is complete.
    Done,
    /// Something was wrong, and the decoder says so again.
    Failed(HttpError),
}

/// The response decoder.
#[derive(Debug)]
pub struct Decoder<'b, const HEADERS: usize> {
    /// Where the head is assembled.
    buffer: &'b mut [u8],
    /// How much of it is used.
    filled: usize,
    /// Where the line being read began.
    line: usize,
    /// Which method this is the answer to.
    method: Method,
    /// The status, once the status line is read.
    status: Status,
    /// Where the reason phrase is.
    reason: Span,
    /// The fields.
    fields: [(Span, Span); HEADERS],
    /// How many of them there are.
    count: usize,
    /// Whether the head is complete.
    head: bool,
    /// The chunk-size or trailer line being assembled.
    chunk: [u8; MAX_CHUNK_LINE],
    /// How much of it is used.
    chunk_len: usize,
    /// Where it stands.
    state: State,
}

impl<'b, const HEADERS: usize> Decoder<'b, HEADERS> {
    /// A decoder for the answer to a request of `method`, assembling the
    /// head in `buffer`.
    #[must_use]
    pub const fn new(buffer: &'b mut [u8], method: Method) -> Decoder<'b, HEADERS> {
        Decoder {
            buffer,
            filled: 0,
            line: 0,
            method,
            status: Status(0),
            reason: Span::EMPTY,
            fields: [(Span::EMPTY, Span::EMPTY); HEADERS],
            count: 0,
            head: false,
            chunk: [0u8; MAX_CHUNK_LINE],
            chunk_len: 0,
            state: State::Status,
        }
    }

    /// The head, once it is complete.
    #[must_use]
    pub fn head(&self) -> Option<Head<'_>> {
        self.head.then(|| Head {
            status: self.status,
            bytes: self.buffer,
            reason: self.reason,
            fields: self.fields.get(..self.count).unwrap_or(&[]),
        })
    }

    /// Whether the message is complete.
    #[must_use]
    pub const fn is_done(&self) -> bool {
        matches!(self.state, State::Done)
    }

    /// Whether the body ends when the connection does, so that
    /// [`finish`](Decoder::finish) is what completes it.
    #[must_use]
    pub const fn ends_at_close(&self) -> bool {
        matches!(self.state, State::Close)
    }

    /// Takes in the bytes that arrived and answers what it made of them.
    ///
    /// The answer names how many bytes were taken; a caller loops over
    /// what is left until the answer is [`Event::NeedMore`] or the message
    /// is done. At most one line of the head is read per call, so no byte
    /// of the body is ever copied into the head buffer.
    ///
    /// # Errors
    ///
    /// Whatever the head or the framing was wrong about. A decoder that
    /// has failed answers the same error to every further call.
    pub fn feed<'i>(&mut self, input: &'i [u8]) -> Result<(usize, Event<'i>), HttpError> {
        match self.state {
            State::Failed(error) => Err(error),
            State::Done => Ok((0, Event::Done)),
            State::Status | State::Fields => self.feed_head(input),
            State::Length(left) => Ok(self.feed_length(input, left)),
            State::Chunked(chunk) => self.feed_chunked(input, chunk),
            State::Close => Ok(if input.is_empty() {
                (0, Event::NeedMore)
            } else {
                (input.len(), Event::Body(input))
            }),
        }
    }

    /// Tells the decoder that the peer closed the connection.
    ///
    /// # Errors
    ///
    /// [`HttpError::Truncated`] when the message had said how long it was
    /// and was not that long.
    pub const fn finish(&mut self) -> Result<(), HttpError> {
        match self.state {
            State::Done => Ok(()),
            State::Close => {
                self.state = State::Done;
                Ok(())
            }
            State::Failed(error) => Err(error),
            _ => {
                self.state = State::Failed(HttpError::Truncated);
                Err(HttpError::Truncated)
            }
        }
    }

    /// Remembers `error` and answers it.
    const fn fail<T>(&mut self, error: HttpError) -> Result<T, HttpError> {
        self.state = State::Failed(error);
        Err(error)
    }

    /// One line of the head.
    fn feed_head<'i>(&mut self, input: &'i [u8]) -> Result<(usize, Event<'i>), HttpError> {
        if input.is_empty() {
            return Ok((0, Event::NeedMore));
        }
        let end = input
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(input.len(), |at| at.saturating_add(1));
        let piece = input.get(..end).unwrap_or(&[]);
        let room = self.buffer.len();
        let filled = self.filled.saturating_add(piece.len());
        let Some(slot) = self.buffer.get_mut(self.filled..filled) else {
            return self.fail(HttpError::HeadTooLong(room));
        };
        slot.copy_from_slice(piece);
        self.filled = filled;
        let len = self.filled.saturating_sub(self.line);
        let limit = if self.state == State::Status {
            MAX_STATUS_LINE
        } else {
            MAX_HEADER_LINE
        };
        if len > limit {
            return if self.state == State::Status {
                self.fail(HttpError::StatusLine(len))
            } else {
                self.fail(HttpError::HeaderLine(len))
            };
        }
        if piece.last() != Some(&b'\n') {
            return Ok((end, Event::NeedMore));
        }
        match self.take_line() {
            Ok(true) => Ok((end, Event::Head)),
            Ok(false) => Ok((end, Event::NeedMore)),
            Err(error) => self.fail(error),
        }
    }

    /// Reads the line that has just been completed, and answers whether
    /// the head is now done.
    fn take_line(&mut self) -> Result<bool, HttpError> {
        let start = self.line;
        let end = self.filled;
        self.line = end;
        let line = eol(self.buffer.get(start..end).unwrap_or(&[]));
        if self.state == State::Status {
            let (status, reason) = parse_status(start, line)?;
            self.status = status;
            self.reason = reason;
            self.state = State::Fields;
            return Ok(false);
        }
        if line.is_empty() {
            self.head = true;
            self.state = self.framing()?;
            return Ok(true);
        }
        let field = parse_field(start, line)?;
        let Some(slot) = self.fields.get_mut(self.count) else {
            return Err(HttpError::TooManyHeaders(HEADERS));
        };
        *slot = field;
        self.count = self.count.saturating_add(1);
        Ok(false)
    }

    /// Which of the four framings this message has, in the order of
    /// RFC 9112, section 6.3.
    fn framing(&self) -> Result<State, HttpError> {
        if !self.method.expects_body() || !self.status.carries_body() {
            return Ok(State::Done);
        }
        let head = Head {
            status: self.status,
            bytes: self.buffer,
            reason: self.reason,
            fields: self.fields.get(..self.count).unwrap_or(&[]),
        };
        let mut encodings = head
            .headers()
            .filter(|(name, _)| same_name(name, "transfer-encoding"));
        let encoding = encodings.next();
        let lengths = head
            .headers()
            .filter(|(name, _)| same_name(name, "content-length"));
        let declared = lengths.clone().count();
        if encoding.is_some() && declared > 0 {
            return Err(HttpError::ConflictingFraming);
        }
        if let Some((_, value)) = encoding {
            if encodings.next().is_some() || !same_name(trim(value), "chunked") {
                return Err(HttpError::TransferEncoding);
            }
            return Ok(State::Chunked(Chunk::Size));
        }
        if declared == 0 {
            return Ok(State::Close);
        }
        let length = single_length(lengths.map(|(_, value)| value))?;
        Ok(if length == 0 {
            State::Done
        } else {
            State::Length(length)
        })
    }

    /// A body of a declared length.
    fn feed_length<'i>(&mut self, input: &'i [u8], left: u64) -> (usize, Event<'i>) {
        if input.is_empty() {
            return (0, Event::NeedMore);
        }
        let take = usize::try_from(left).unwrap_or(usize::MAX).min(input.len());
        let body = input.get(..take).unwrap_or(&[]);
        let rest = left.saturating_sub(take.try_into().unwrap_or(u64::MAX));
        self.state = if rest == 0 {
            State::Done
        } else {
            State::Length(rest)
        };
        (take, Event::Body(body))
    }

    /// A chunked body.
    fn feed_chunked<'i>(
        &mut self,
        input: &'i [u8],
        chunk: Chunk,
    ) -> Result<(usize, Event<'i>), HttpError> {
        if input.is_empty() {
            return Ok((0, Event::NeedMore));
        }
        match chunk {
            Chunk::Data(left) => {
                let take = usize::try_from(left).unwrap_or(usize::MAX).min(input.len());
                let body = input.get(..take).unwrap_or(&[]);
                let rest = left.saturating_sub(take.try_into().unwrap_or(u64::MAX));
                self.state = State::Chunked(if rest == 0 {
                    Chunk::After
                } else {
                    Chunk::Data(rest)
                });
                Ok((take, Event::Body(body)))
            }
            Chunk::Size | Chunk::After | Chunk::Trailer => {
                let (consumed, complete) = match self.chunk_line(input) {
                    Ok(taken) => taken,
                    Err(error) => return self.fail(error),
                };
                if !complete {
                    return Ok((consumed, Event::NeedMore));
                }
                let len = self.chunk_len;
                let line = eol(self.chunk.get(..len).unwrap_or(&[]));
                let next = if chunk == Chunk::Size {
                    match chunk_size(line) {
                        Ok(0) => Ok(State::Chunked(Chunk::Trailer)),
                        Ok(size) => Ok(State::Chunked(Chunk::Data(size))),
                        Err(error) => Err(error),
                    }
                } else if chunk == Chunk::After {
                    // The two bytes behind the data and nothing else.
                    if line.is_empty() {
                        Ok(State::Chunked(Chunk::Size))
                    } else {
                        Err(HttpError::Chunk)
                    }
                } else {
                    // A trailer field, which is read past and dropped;
                    // the empty line behind them ends the body.
                    Ok(if line.is_empty() {
                        State::Done
                    } else {
                        State::Chunked(Chunk::Trailer)
                    })
                };
                self.chunk_len = 0;
                match next {
                    Ok(state) => {
                        self.state = state;
                        Ok((
                            consumed,
                            if state == State::Done {
                                Event::Done
                            } else {
                                Event::NeedMore
                            },
                        ))
                    }
                    Err(error) => self.fail(error),
                }
            }
        }
    }

    /// Takes bytes into the chunk-line scratch, up to and including the
    /// first newline, and answers whether a line is now complete.
    fn chunk_line(&mut self, input: &[u8]) -> Result<(usize, bool), HttpError> {
        let end = input
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(input.len(), |at| at.saturating_add(1));
        let piece = input.get(..end).unwrap_or(&[]);
        let filled = self.chunk_len.saturating_add(piece.len());
        let Some(slot) = self.chunk.get_mut(self.chunk_len..filled) else {
            return Err(HttpError::ChunkSize);
        };
        slot.copy_from_slice(piece);
        self.chunk_len = filled;
        Ok((end, piece.last() == Some(&b'\n')))
    }
}

/// The status line: a version, a code, and a reason phrase.
fn parse_status(start: usize, line: &[u8]) -> Result<(Status, Span), HttpError> {
    let Some(rest) = line
        .strip_prefix(b"HTTP/1.1 ")
        .or_else(|| line.strip_prefix(b"HTTP/1.0 "))
    else {
        return Err(HttpError::Version);
    };
    let Some(digits) = rest.first_chunk::<3>() else {
        return Err(HttpError::Status);
    };
    let mut code = 0u16;
    for digit in digits {
        let Some(value) = char::from(*digit).to_digit(10) else {
            return Err(HttpError::Status);
        };
        let Ok(value) = u16::try_from(value) else {
            return Err(HttpError::Status);
        };
        code = code
            .checked_mul(10)
            .and_then(|shifted| shifted.checked_add(value))
            .ok_or(HttpError::Status)?;
    }
    let reason = match rest.get(3..) {
        None | Some([]) => &[][..],
        Some([b' ', text @ ..]) => text,
        Some(_) => return Err(HttpError::Status),
    };
    if reason.iter().any(|byte| !usable(*byte)) {
        return Err(HttpError::Status);
    }
    let at = start
        .saturating_add(line.len())
        .saturating_sub(reason.len());
    Ok((
        Status(code),
        Span {
            start: at,
            len: reason.len(),
        },
    ))
}

/// One field line, as the two spans it puts in the head buffer.
fn parse_field(start: usize, line: &[u8]) -> Result<(Span, Span), HttpError> {
    if matches!(line.first(), Some(b' ' | b'\t')) {
        return Err(HttpError::ObsoleteFold);
    }
    let Some(colon) = line.iter().position(|byte| *byte == b':') else {
        return Err(HttpError::HeaderName);
    };
    let after = colon.saturating_add(1);
    let (Ok(name), Ok(value)) = (
        core::str::from_utf8(line.get(..colon).unwrap_or(&[])),
        core::str::from_utf8(line.get(after..).unwrap_or(&[])),
    ) else {
        return Err(HttpError::HeaderName);
    };
    // RFC 9112, section 5.1: no whitespace between the name and the
    // colon, which `is_token` refuses along with everything else that is
    // not a token.
    if !is_token(name) {
        return Err(HttpError::HeaderName);
    }
    let trimmed = trim(value);
    if !is_value(trimmed) {
        return Err(HttpError::HeaderValue);
    }
    let leading = value
        .len()
        .saturating_sub(value.trim_start_matches([' ', '\t']).len());
    Ok((
        Span {
            start,
            len: name.len(),
        },
        Span {
            start: start.saturating_add(after).saturating_add(leading),
            len: trimmed.len(),
        },
    ))
}

/// `line` without the bytes that ended it.
fn eol(line: &[u8]) -> &[u8] {
    let without_lf = line.strip_suffix(b"\n").unwrap_or(line);
    without_lf.strip_suffix(b"\r").unwrap_or(without_lf)
}

/// Whether `byte` may stand in a reason phrase.
const fn usable(byte: u8) -> bool {
    byte == b'\t' || byte.is_ascii_graphic() || byte == b' '
}

/// The one length every `Content-Length` field agrees on.
///
/// RFC 9112, section 6.3, point 5 accepts a comma-separated list whose
/// values are all valid and all the same, and nothing else.
fn single_length<'a, I: Iterator<Item = &'a str>>(values: I) -> Result<u64, HttpError> {
    let mut agreed: Option<u64> = None;
    for value in values {
        for part in value.split(',') {
            let text = trim(part);
            if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(HttpError::ContentLength);
            }
            let Ok(number) = text.parse::<u64>() else {
                return Err(HttpError::ContentLength);
            };
            if agreed.is_some_and(|held| held != number) {
                return Err(HttpError::ContentLength);
            }
            agreed = Some(number);
        }
    }
    agreed.ok_or(HttpError::ContentLength)
}

/// The size a chunk-size line names, with the extensions behind it
/// ignored.
fn chunk_size(line: &[u8]) -> Result<u64, HttpError> {
    let digits = line
        .iter()
        .position(|byte| *byte == b';')
        .map_or(line, |at| line.get(..at).unwrap_or(&[]));
    if digits.is_empty() || digits.len() > 16 {
        return Err(HttpError::ChunkSize);
    }
    let mut size = 0u64;
    for byte in digits {
        let Some(value) = char::from(*byte).to_digit(16) else {
            return Err(HttpError::ChunkSize);
        };
        size = size
            .checked_mul(16)
            .and_then(|shifted| shifted.checked_add(u64::from(value)))
            .ok_or(HttpError::ChunkSize)?;
    }
    Ok(size)
}
