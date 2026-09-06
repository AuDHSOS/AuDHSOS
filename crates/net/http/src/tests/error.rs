// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Every error says what it found, once.

use net_wire::WireError;

use crate::error::HttpError;

#[test]
fn every_error_reads_as_a_sentence() {
    let cases = [
        (
            HttpError::Wire(WireError::OutOfBounds {
                needed: 40,
                available: 8,
            }),
            "40 bytes were needed and 8 are left",
        ),
        (
            HttpError::StatusLine(300),
            "a status line of 300 bytes is longer than one is read",
        ),
        (
            HttpError::HeaderLine(2000),
            "a header line of 2000 bytes is longer than one is read",
        ),
        (
            HttpError::TooManyHeaders(16),
            "more header fields than the 16 this decoder holds",
        ),
        (
            HttpError::HeadTooLong(512),
            "the head is longer than the 512 bytes it has",
        ),
        (
            HttpError::ObsoleteFold,
            "a folded header line, which a recipient refuses",
        ),
        (
            HttpError::Version,
            "the status line names no version this client reads",
        ),
        (HttpError::Status, "the status code is not three digits"),
        (HttpError::HeaderName, "the header name is not a token"),
        (
            HttpError::HeaderValue,
            "the header value carries a control character",
        ),
        (
            HttpError::Target,
            "the request target carries a byte it may not",
        ),
        (
            HttpError::ConflictingFraming,
            "a message with a length and a transfer encoding has two lengths",
        ),
        (
            HttpError::ContentLength,
            "the content length is not one number",
        ),
        (
            HttpError::TransferEncoding,
            "the transfer encoding is not chunked and last",
        ),
        (
            HttpError::ChunkSize,
            "the chunk size is not a hexadecimal number",
        ),
        (
            HttpError::Chunk,
            "the chunk does not end where it said it would",
        ),
        (
            HttpError::Truncated,
            "the connection closed inside the message",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
    }
}

#[test]
fn the_error_of_the_crate_below_arrives_unchanged() {
    let wire = WireError::Address;
    assert_eq!(HttpError::from(wire), HttpError::Wire(wire));
}
