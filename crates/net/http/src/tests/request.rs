// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What goes out, and what is refused before a byte of it does.

use net_wire::{WireError, Writer};

use crate::error::HttpError;
use crate::request::{Method, Request};

/// The bytes `request` writes.
fn written(request: &Request<'_>) -> Result<Vec<u8>, HttpError> {
    let mut buffer = [0u8; 512];
    let mut writer = Writer::new(&mut buffer);
    request.write(&mut writer)?;
    Ok(writer.finish().to_vec())
}

#[test]
fn a_minimal_get_is_a_request_line_and_a_host() {
    let bytes = written(&Request::get("/index.html", "example.com")).expect("a request");
    assert_eq!(
        core::str::from_utf8(&bytes).expect("text"),
        "GET /index.html HTTP/1.1\r\nHost: example.com\r\n\r\n"
    );
}

#[test]
fn the_fields_the_caller_gives_are_written_in_order() {
    let request = Request {
        method: Method::Get,
        target: "/a?b=c",
        host: "example.com:8080",
        headers: &[("Accept", "text/plain"), ("User-Agent", "audhsos/0.1")],
        body: &[],
    };
    let bytes = written(&request).expect("a request");
    assert_eq!(
        core::str::from_utf8(&bytes).expect("text"),
        "GET /a?b=c HTTP/1.1\r\nHost: example.com:8080\r\nAccept: text/plain\r\nUser-Agent: audhsos/0.1\r\n\r\n"
    );
}

#[test]
fn a_body_brings_its_own_length() {
    let request = Request {
        method: Method::Post,
        target: "/submit",
        host: "example.com",
        headers: &[("Content-Type", "text/plain")],
        body: b"hello",
    };
    let bytes = written(&request).expect("a request");
    assert_eq!(
        core::str::from_utf8(&bytes).expect("text"),
        "POST /submit HTTP/1.1\r\nHost: example.com\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello"
    );
}

#[test]
fn a_long_body_counts_its_bytes_right() {
    let body = vec![b'x'; 1234];
    let request = Request {
        method: Method::Post,
        target: "/",
        host: "h",
        headers: &[],
        body: &body,
    };
    let mut buffer = [0u8; 2048];
    let mut writer = Writer::new(&mut buffer);
    request.write(&mut writer).expect("room");
    let bytes = writer.finish();
    let text = core::str::from_utf8(bytes.get(..64).expect("a head")).expect("text");
    assert!(text.contains("Content-Length: 1234\r\n"), "{text}");
}

#[test]
fn a_value_that_would_end_its_field_is_refused_before_it_is_written() {
    for (name, value) in [
        ("X-Thing", "one\r\nX-Other: two"),
        ("X-Thing", "one\nX-Other: two"),
        ("X-Thing", "one\rtwo"),
        ("X-Thing", "one\0two"),
    ] {
        let request = Request {
            method: Method::Get,
            target: "/",
            host: "h",
            headers: &[(name, value)],
            body: &[],
        };
        assert_eq!(written(&request), Err(HttpError::HeaderValue));
    }
}

#[test]
fn a_name_that_is_not_a_token_is_refused() {
    for name in ["X Thing", "X:Thing", "", "X\r\nY"] {
        let request = Request {
            method: Method::Get,
            target: "/",
            host: "h",
            headers: &[(name, "v")],
            body: &[],
        };
        assert_eq!(written(&request), Err(HttpError::HeaderName));
    }
}

#[test]
fn the_three_fields_this_crate_writes_itself_may_not_be_written_twice() {
    for name in ["Host", "host", "Content-Length", "Transfer-Encoding"] {
        let request = Request {
            method: Method::Get,
            target: "/",
            host: "h",
            headers: &[(name, "x")],
            body: &[],
        };
        assert_eq!(written(&request), Err(HttpError::HeaderName));
    }
}

#[test]
fn a_target_that_is_not_an_origin_form_path_is_refused() {
    for target in [
        "index.html",
        "http://example.com/",
        "/with space",
        "/with\rreturn",
        "",
    ] {
        let request = Request::get(target, "example.com");
        assert_eq!(written(&request), Err(HttpError::Target));
    }
}

#[test]
fn a_host_that_is_not_a_value_is_refused() {
    for host in ["", "ex\r\nample.com", " example.com"] {
        assert_eq!(
            written(&Request::get("/", host)),
            Err(HttpError::HeaderValue)
        );
    }
}

#[test]
fn a_request_that_does_not_fit_its_buffer_says_so() {
    let request = Request::get("/index.html", "example.com");
    for room in 0..46usize {
        let mut buffer = vec![0u8; room];
        let mut writer = Writer::new(&mut buffer);
        assert!(
            matches!(
                request.write(&mut writer),
                Err(HttpError::Wire(WireError::OutOfBounds { .. }))
            ),
            "a request written into {room} bytes"
        );
    }
}

#[test]
fn a_method_carries_its_word_and_says_whether_an_answer_has_a_body() {
    assert_eq!(Method::Get.as_str(), "GET");
    assert_eq!(Method::Head.as_str(), "HEAD");
    assert_eq!(Method::Post.as_str(), "POST");
    assert!(Method::Get.expects_body());
    assert!(Method::Post.expects_body());
    assert!(!Method::Head.expects_body());
}
