// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Why a request could not be written or a response could not be read.

use core::fmt;

use net_wire::WireError;

/// What this crate found wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HttpError {
    /// A cursor ran out of buffer.
    Wire(WireError),
    /// A status line longer than this client reads. The value is how long
    /// it had grown when it was given up on.
    StatusLine(usize),
    /// A header line longer than this client reads.
    HeaderLine(usize),
    /// More header fields than the decoder holds. The value is how many
    /// it holds.
    TooManyHeaders(usize),
    /// The head is longer than the buffer the caller gave it. The value
    /// is how long that buffer is.
    HeadTooLong(usize),
    /// A header line that begins with a space or a tab, which is the
    /// obsolete line folding of RFC 9112, section 5.2.
    ///
    /// That section has a user agent replace the fold with spaces rather
    /// than refuse the message, so refusing it is stricter than the
    /// document and deliberately so. A folded value is a value whose
    /// length is not its line's length, so a parser that unfolds and one
    /// that does not read two different messages out of the same bytes,
    /// which is the class of ambiguity
    /// [`ConflictingFraming`](HttpError::ConflictingFraming) is about.
    ObsoleteFold,
    /// The status line does not begin with a version this client speaks.
    Version,
    /// The status code is not three digits.
    Status,
    /// A header name that is not a token: RFC 9110, section 5.6.2 gives
    /// the bytes a name may be made of, and this is not one of them.
    HeaderName,
    /// A header value carrying a control character other than the
    /// horizontal tab.
    HeaderValue,
    /// A request target carrying a byte a target may not have.
    Target,
    /// `Content-Length` and `Transfer-Encoding` in one message. RFC 9112,
    /// section 6.1 lets a recipient prefer one; this one refuses both,
    /// because two readings of one message is the whole of request
    /// smuggling.
    ConflictingFraming,
    /// A `Content-Length` that is not a number, or two that disagree.
    ContentLength,
    /// A `Transfer-Encoding` other than `chunked`, or one with `chunked`
    /// anywhere but last.
    TransferEncoding,
    /// A chunk size that is not hexadecimal, or a size line longer than
    /// this decoder reads.
    ChunkSize,
    /// A chunk that is not followed by the two bytes that end it.
    Chunk,
    /// The connection closed in the middle of a message that had said how
    /// long it was.
    Truncated,
}

impl From<WireError> for HttpError {
    fn from(error: WireError) -> HttpError {
        HttpError::Wire(error)
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HttpError::Wire(error) => error.fmt(f),
            HttpError::StatusLine(len) => {
                write!(f, "a status line of {len} bytes is longer than one is read")
            }
            HttpError::HeaderLine(len) => {
                write!(f, "a header line of {len} bytes is longer than one is read")
            }
            HttpError::TooManyHeaders(limit) => {
                write!(f, "more header fields than the {limit} this decoder holds")
            }
            HttpError::HeadTooLong(room) => {
                write!(f, "the head is longer than the {room} bytes it has")
            }
            HttpError::ObsoleteFold => {
                f.write_str("a folded header line, which a recipient refuses")
            }
            HttpError::Version => f.write_str("the status line names no version this client reads"),
            HttpError::Status => f.write_str("the status code is not three digits"),
            HttpError::HeaderName => f.write_str("the header name is not a token"),
            HttpError::HeaderValue => f.write_str("the header value carries a control character"),
            HttpError::Target => f.write_str("the request target carries a byte it may not"),
            HttpError::ConflictingFraming => {
                f.write_str("a message with a length and a transfer encoding has two lengths")
            }
            HttpError::ContentLength => f.write_str("the content length is not one number"),
            HttpError::TransferEncoding => {
                f.write_str("the transfer encoding is not chunked and last")
            }
            HttpError::ChunkSize => f.write_str("the chunk size is not a hexadecimal number"),
            HttpError::Chunk => f.write_str("the chunk does not end where it said it would"),
            HttpError::Truncated => f.write_str("the connection closed inside the message"),
        }
    }
}
