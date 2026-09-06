// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What comes in: the head, the four framings, and everything that is
//! refused because it could be read two ways.

use core::fmt::Write as _;

use test_support::generators::range;
use test_support::property::check;

use crate::error::HttpError;
use crate::request::Method;
use crate::response::{Decoder, Event, MAX_HEADER_LINE, MAX_STATUS_LINE, Status};

/// What a whole response decoded to.
#[derive(Debug, Default, PartialEq, Eq)]
struct Decoded {
    /// The status code.
    status: u16,
    /// The reason phrase.
    reason: String,
    /// The fields, in the order they arrived.
    headers: Vec<(String, String)>,
    /// The body, put back together.
    body: Vec<u8>,
    /// Whether the decoder said the message is complete.
    done: bool,
}

/// Feeds `bytes` in pieces of `piece` bytes and answers what came out.
fn decode_in_pieces(bytes: &[u8], piece: usize, method: Method) -> Result<Decoded, HttpError> {
    let mut buffer = [0u8; 4096];
    let mut decoder = Decoder::<16>::new(&mut buffer, method);
    let mut out = Decoded::default();
    let mut head = false;
    let mut at = 0usize;
    let mut turns = 0usize;
    while at < bytes.len() {
        let end = at.saturating_add(piece.max(1)).min(bytes.len());
        let mut chunk = bytes.get(at..end).unwrap_or(&[]);
        while !chunk.is_empty() {
            let (consumed, event) = decoder.feed(chunk)?;
            match event {
                Event::Head => head = true,
                Event::Body(body) => out.body.extend_from_slice(body),
                Event::Done => out.done = true,
                Event::NeedMore => {}
            }
            chunk = chunk.get(consumed..).unwrap_or(&[]);
            turns = turns.saturating_add(1);
            assert!(turns < 100_000, "the decoder made no progress");
            if consumed == 0 {
                break;
            }
        }
        at = end;
    }
    // Everything that is left to say at the end of the input.
    loop {
        let (consumed, event) = decoder.feed(&[])?;
        match event {
            Event::Done => out.done = true,
            Event::Head => head = true,
            Event::NeedMore | Event::Body(_) => {}
        }
        if consumed == 0 {
            break;
        }
    }
    if head {
        let read = decoder.head().expect("a head");
        out.status = read.status.get();
        out.reason = read.reason().to_owned();
        out.headers = read
            .headers()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
    }
    Ok(out)
}

/// Feeds `bytes` in one go.
fn decode(bytes: &[u8]) -> Result<Decoded, HttpError> {
    decode_in_pieces(bytes, bytes.len().max(1), Method::Get)
}

#[test]
fn a_minimal_response_is_a_status_line_and_an_empty_line() {
    let out = decode(b"HTTP/1.1 204 No Content\r\n\r\n").expect("a response");
    assert_eq!(out.status, 204);
    assert_eq!(out.reason, "No Content");
    assert!(out.headers.is_empty());
    assert!(out.body.is_empty());
    assert!(out.done);
}

#[test]
fn a_body_of_a_declared_length_comes_out_whole() {
    let out = decode(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello").expect("a response");
    assert_eq!(out.status, 200);
    assert_eq!(
        out.headers,
        vec![("Content-Length".to_owned(), "5".to_owned())]
    );
    assert_eq!(out.body, b"hello");
    assert!(out.done);
}

#[test]
fn a_chunked_body_comes_out_whole() {
    let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
        5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
    let out = decode(response).expect("a response");
    assert_eq!(out.body, b"hello world");
    assert!(out.done);
}

#[test]
fn a_chunk_may_carry_an_extension_and_the_last_one_may_carry_trailers() {
    let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
        5;name=value\r\nhello\r\n0\r\nExpires: never\r\nX-Sum: 1\r\n\r\n";
    let out = decode(response).expect("a response");
    assert_eq!(out.body, b"hello");
    assert!(out.done);
    // Trailers are read past and are not fields of the head.
    assert_eq!(out.headers.len(), 1);
}

#[test]
fn the_zero_chunk_may_stand_on_its_own() {
    let out = decode(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n")
        .expect("a response");
    assert!(out.body.is_empty());
    assert!(out.done);
}

#[test]
fn a_response_split_at_any_boundary_decodes_to_the_same_thing() {
    let response =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nTransfer-Encoding: chunked\r\n\r\n\
        3;x\r\nabc\r\n4\r\ndefg\r\n0\r\nTrailer: yes\r\n\r\n";
    let whole = decode(response).expect("a response");
    assert_eq!(whole.body, b"abcdefg");
    for piece in 1..=response.len() {
        let split = decode_in_pieces(response, piece, Method::Get).expect("a response");
        assert_eq!(split, whole, "split into pieces of {piece}");
    }
}

#[test]
fn a_response_of_a_declared_length_split_anywhere_decodes_to_the_same_thing() {
    let response =
        b"HTTP/1.1 301 Moved\r\nLocation: /elsewhere\r\nContent-Length: 11\r\n\r\nhello world";
    let whole = decode(response).expect("a response");
    for piece in 1..=response.len() {
        assert_eq!(
            decode_in_pieces(response, piece, Method::Get).expect("a response"),
            whole,
            "split into pieces of {piece}"
        );
    }
}

#[test]
fn a_body_larger_than_one_read_comes_out_in_parts_without_loss() {
    let body = vec![b'z'; 5000];
    let mut response = b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\n\r\n".to_vec();
    response.extend_from_slice(&body);
    let out = decode_in_pieces(&response, 137, Method::Get).expect("a response");
    assert_eq!(out.body, body);
    assert!(out.done);
}

#[test]
fn a_body_that_ends_with_the_connection_needs_the_caller_to_say_so() {
    let mut buffer = [0u8; 512];
    let mut decoder = Decoder::<8>::new(&mut buffer, Method::Get);
    let head = b"HTTP/1.1 200 OK\r\n\r\n";
    let mut at = 0usize;
    while at < head.len() {
        let (consumed, _) = decoder.feed(head.get(at..).unwrap_or(&[])).expect("a head");
        at = at.saturating_add(consumed.max(1));
    }
    assert!(decoder.ends_at_close());
    let (consumed, event) = decoder.feed(b"body bytes").expect("a body");
    assert_eq!(consumed, 10);
    assert_eq!(event, Event::Body(b"body bytes"));
    assert!(!decoder.is_done());
    decoder.finish().expect("the peer closed");
    assert!(decoder.is_done());
    assert_eq!(decoder.feed(b"").expect("done"), (0, Event::Done));
}

#[test]
fn a_message_that_said_how_long_it_was_and_was_not_is_truncated() {
    let mut buffer = [0u8; 512];
    let mut decoder = Decoder::<8>::new(&mut buffer, Method::Get);
    let mut input = &b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort"[..];
    while !input.is_empty() {
        let (consumed, _) = decoder.feed(input).expect("a response");
        input = input.get(consumed.max(1)..).unwrap_or(&[]);
    }
    assert_eq!(decoder.finish(), Err(HttpError::Truncated));
    // And it says the same thing to every further call.
    assert_eq!(decoder.feed(b"more"), Err(HttpError::Truncated));
    assert_eq!(decoder.finish(), Err(HttpError::Truncated));
}

#[test]
fn a_head_response_and_the_bodiless_statuses_carry_no_body() {
    let with_length = b"HTTP/1.1 200 OK\r\nContent-Length: 99\r\n\r\n";
    let out = decode_in_pieces(with_length, 1024, Method::Head).expect("a response");
    assert!(out.done);
    assert!(out.body.is_empty());
    for code in [100u16, 101, 204, 304] {
        let response = format!("HTTP/1.1 {code} Something\r\nContent-Length: 99\r\n\r\n");
        let out = decode(response.as_bytes()).expect("a response");
        assert_eq!(out.status, code);
        assert!(out.done, "a {code} carried a body");
        assert!(out.body.is_empty());
    }
}

#[test]
fn a_message_with_two_framings_is_refused_rather_than_resolved() {
    let response =
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
    assert_eq!(decode(response), Err(HttpError::ConflictingFraming));
    let other =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\n0\r\n\r\n";
    assert_eq!(decode(other), Err(HttpError::ConflictingFraming));
}

#[test]
fn two_lengths_that_disagree_are_refused_and_two_that_agree_are_one() {
    assert_eq!(
        decode(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\nhello"),
        Err(HttpError::ContentLength)
    );
    assert_eq!(
        decode(b"HTTP/1.1 200 OK\r\nContent-Length: 5, 6\r\n\r\nhello"),
        Err(HttpError::ContentLength)
    );
    // RFC 9112, section 6.3, point 5: a list whose values are all the
    // same is that one value.
    let out = decode(b"HTTP/1.1 200 OK\r\nContent-Length: 5, 5\r\n\r\nhello").expect("a response");
    assert_eq!(out.body, b"hello");
    let out = decode(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\nhello")
        .expect("a response");
    assert_eq!(out.body, b"hello");
}

#[test]
fn a_length_that_is_not_a_number_is_refused() {
    for value in [
        "five",
        "5x",
        "",
        " ",
        "-1",
        "0x5",
        "99999999999999999999999",
    ] {
        let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {value}\r\n\r\n");
        assert_eq!(
            decode(response.as_bytes()),
            Err(HttpError::ContentLength),
            "a length of {value:?}"
        );
    }
}

#[test]
fn a_transfer_encoding_that_is_not_chunked_alone_is_refused() {
    for value in ["gzip", "gzip, chunked", "chunked, gzip", "", "chunked;q=1"] {
        let response = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: {value}\r\n\r\n");
        assert_eq!(
            decode(response.as_bytes()),
            Err(HttpError::TransferEncoding),
            "an encoding of {value:?}"
        );
    }
    let twice =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
    assert_eq!(decode(twice), Err(HttpError::TransferEncoding));
}

#[test]
fn a_folded_header_line_is_refused() {
    assert_eq!(
        decode(b"HTTP/1.1 200 OK\r\nX-Thing: one\r\n two\r\n\r\n"),
        Err(HttpError::ObsoleteFold)
    );
    assert_eq!(
        decode(b"HTTP/1.1 200 OK\r\nX-Thing: one\r\n\ttwo\r\n\r\n"),
        Err(HttpError::ObsoleteFold)
    );
}

#[test]
fn a_status_line_that_is_not_one_is_refused() {
    for (response, error) in [
        (&b"HTTP/2 200 OK\r\n\r\n"[..], HttpError::Version),
        (b"ICY 200 OK\r\n\r\n", HttpError::Version),
        (b"\r\n", HttpError::Version),
        (b"HTTP/1.1 20 OK\r\n\r\n", HttpError::Status),
        (b"HTTP/1.1 2000 OK\r\n\r\n", HttpError::Status),
        (b"HTTP/1.1 2x0 OK\r\n\r\n", HttpError::Status),
        (b"HTTP/1.1 200OK\r\n\r\n", HttpError::Status),
        (b"HTTP/1.1 200 O\x01K\r\n\r\n", HttpError::Status),
    ] {
        assert_eq!(decode(response), Err(error), "{response:?}");
    }
    // A version of 1.0 and an empty reason are both fine.
    let out = decode(b"HTTP/1.0 200\r\n\r\n").expect("a response");
    assert_eq!(out.status, 200);
    assert_eq!(out.reason, "");
}

#[test]
fn a_field_that_is_not_one_is_refused() {
    for (response, error) in [
        (
            &b"HTTP/1.1 200 OK\r\nNo colon here\r\n\r\n"[..],
            HttpError::HeaderName,
        ),
        (
            b"HTTP/1.1 200 OK\r\nX Thing: v\r\n\r\n",
            HttpError::HeaderName,
        ),
        (
            b"HTTP/1.1 200 OK\r\nX-Thing : v\r\n\r\n",
            HttpError::HeaderName,
        ),
        (b"HTTP/1.1 200 OK\r\n: v\r\n\r\n", HttpError::HeaderName),
        (
            b"HTTP/1.1 200 OK\r\nX-Thing: a\x01b\r\n\r\n",
            HttpError::HeaderValue,
        ),
    ] {
        assert_eq!(decode(response), Err(error), "{response:?}");
    }
    // A value may be empty, and the whitespace around it is not part of
    // it.
    let out = decode(b"HTTP/1.1 200 OK\r\nX-Empty:\r\nX-Padded:  \tvalue \t\r\n\r\n")
        .expect("a response");
    assert_eq!(
        out.headers,
        vec![
            ("X-Empty".to_owned(), String::new()),
            ("X-Padded".to_owned(), "value".to_owned())
        ]
    );
}

#[test]
fn a_head_longer_than_it_may_be_is_refused() {
    let long_reason = "R".repeat(MAX_STATUS_LINE);
    let response = format!("HTTP/1.1 200 {long_reason}\r\n\r\n");
    assert!(matches!(
        decode(response.as_bytes()),
        Err(HttpError::StatusLine(_))
    ));

    let long_value = "v".repeat(MAX_HEADER_LINE);
    let response = format!("HTTP/1.1 200 OK\r\nX-Thing: {long_value}\r\n\r\n");
    assert!(matches!(
        decode(response.as_bytes()),
        Err(HttpError::HeaderLine(_))
    ));

    let mut many = String::from("HTTP/1.1 200 OK\r\n");
    for index in 0..20 {
        let _ = write!(many, "X-{index}: v\r\n");
    }
    many.push_str("\r\n");
    assert_eq!(
        decode_in_pieces(many.as_bytes(), 4096, Method::Get),
        Err(HttpError::TooManyHeaders(16))
    );
}

#[test]
fn a_head_longer_than_the_buffer_it_was_given_is_refused() {
    let mut buffer = [0u8; 40];
    let mut decoder = Decoder::<8>::new(&mut buffer, Method::Get);
    let mut input = &b"HTTP/1.1 200 OK\r\nX-Thing: a-rather-long-value-here\r\n\r\n"[..];
    let mut answer = Ok((0, Event::NeedMore));
    while !input.is_empty() {
        answer = decoder.feed(input);
        let Ok((consumed, _)) = answer else { break };
        input = input.get(consumed.max(1)..).unwrap_or(&[]);
    }
    assert_eq!(answer, Err(HttpError::HeadTooLong(40)));
}

#[test]
fn a_chunk_that_is_not_one_is_refused() {
    for (body, error) in [
        ("zz\r\n", HttpError::ChunkSize),
        ("\r\n", HttpError::ChunkSize),
        ("ffffffffffffffff0\r\n", HttpError::ChunkSize),
        ("2\r\nabXY\r\n", HttpError::Chunk),
        ("2\r\nabcd\r\n", HttpError::Chunk),
    ] {
        let response = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{body}");
        assert_eq!(
            decode(response.as_bytes()),
            Err(error),
            "a body of {body:?}"
        );
    }
}

#[test]
fn a_chunk_size_line_longer_than_it_may_be_is_refused() {
    let extension = format!("1;{}\r\n", "x".repeat(300));
    let response =
        format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{extension}a\r\n0\r\n\r\n");
    assert_eq!(decode(response.as_bytes()), Err(HttpError::ChunkSize));
}

#[test]
fn a_redirect_is_reported_with_its_location_and_not_followed() {
    let mut buffer = [0u8; 512];
    let mut decoder = Decoder::<8>::new(&mut buffer, Method::Get);
    let mut input =
        &b"HTTP/1.1 302 Found\r\nLocation: https://example.org/there\r\nContent-Length: 0\r\n\r\n"
            [..];
    while !input.is_empty() {
        let (consumed, _) = decoder.feed(input).expect("a response");
        input = input.get(consumed.max(1)..).unwrap_or(&[]);
    }
    let head = decoder.head().expect("a head");
    assert!(head.status.is_redirection());
    assert!(head.status.is_redirect());
    assert_eq!(head.redirect(), Some("https://example.org/there"));
    assert_eq!(head.header("LOCATION"), Some("https://example.org/there"));
    assert_eq!(head.header("Nothing"), None);
    assert!(decoder.is_done());
}

#[test]
fn a_three_hundred_and_a_three_oh_four_are_not_redirects_a_client_follows() {
    for code in [300u16, 304] {
        let response = format!("HTTP/1.1 {code} Something\r\nLocation: /there\r\n\r\n");
        let mut buffer = [0u8; 512];
        let mut decoder = Decoder::<8>::new(&mut buffer, Method::Get);
        let mut input = response.as_bytes();
        while !input.is_empty() {
            let (consumed, _) = decoder.feed(input).expect("a response");
            input = input.get(consumed.max(1)..).unwrap_or(&[]);
        }
        let head = decoder.head().expect("a head");
        assert!(head.status.is_redirection());
        assert!(!head.status.is_redirect());
        assert_eq!(head.redirect(), None);
    }
}

#[test]
fn every_class_of_status_says_which_it_is() {
    assert!(Status::new(100).is_informational());
    assert!(Status::new(204).is_successful());
    assert!(Status::new(301).is_redirection());
    assert!(Status::new(404).is_client_error());
    assert!(Status::new(503).is_server_error());
    assert!(!Status::new(204).carries_body());
    assert!(!Status::new(304).carries_body());
    assert!(!Status::new(100).carries_body());
    assert!(Status::new(200).carries_body());
    assert_eq!(Status::new(418).get(), 418);
    assert_eq!(Status::default().get(), 0);
}

#[test]
fn a_decoder_that_has_read_nothing_has_no_head() {
    let mut buffer = [0u8; 64];
    let decoder = Decoder::<4>::new(&mut buffer, Method::Get);
    assert!(decoder.head().is_none());
    assert!(!decoder.is_done());
    assert!(!decoder.ends_at_close());
}

#[test]
fn no_byte_stream_and_no_split_makes_the_decoder_panic() {
    check(
        "http_response_survives_any_split",
        &range(1u8..=64),
        |piece| {
            let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
                3\r\nabc\r\n0\r\n\r\n";
            let out =
                decode_in_pieces(response, usize::from(*piece), Method::Get).expect("a response");
            assert_eq!(out.body, b"abc");
            assert!(out.done);
            Ok(())
        },
    );
}
