// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The little of HTTP/1.1 a probe needs: a request to write, and a
//! response to read.
//!
//! This is not the HTTP crate the system will have. It exists so that the
//! bytes carried over the record layer are a real protocol rather than a
//! ping, and so that a successful run says something a caller recognises:
//! a status line and a body that came from the server that was named.
//!
//! `Connection: close` is sent, so the response is delimited by the
//! close of the stream when it carries neither a length nor chunks.

use crate::error::HttpError;

/// A `GET` for `path` on `host`, ready to hand to the record layer.
#[must_use]
pub fn get(host: &str, path: &str) -> Vec<u8> {
    let mut request = String::new();
    request.push_str("GET ");
    request.push_str(path);
    request.push_str(" HTTP/1.1\r\nHost: ");
    request.push_str(host);
    request.push_str("\r\nUser-Agent: audhsos-tls-probe/0.1\r\n");
    request.push_str("Accept: */*\r\nConnection: close\r\n\r\n");
    request.into_bytes()
}

/// What a server answered.
#[derive(Debug)]
pub struct Response {
    /// The three digits.
    pub status: u16,
    /// The text after them, which servers are free to choose.
    pub reason: String,
    /// The header fields, in the order they arrived, names lowercased.
    pub headers: Vec<(String, String)>,
    /// The body, with any chunk framing removed.
    pub body: Vec<u8>,
}

impl Response {
    /// The first field with this name, compared without case.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        let wanted = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(field, _)| *field == wanted)
            .map(|(_, value)| value.as_str())
    }
}

/// Reads a whole response.
///
/// # Errors
///
/// One of the [`HttpError`] variants: the head must end in a blank line,
/// the status line must have its three parts, every header line must carry
/// a colon, and chunk framing must be well formed.
pub fn parse(raw: &[u8]) -> Result<Response, HttpError> {
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(HttpError::NoHead)?;
    let head = raw.get(..split).ok_or(HttpError::NoHead)?;
    let rest = raw
        .get(split.saturating_add(4)..)
        .ok_or(HttpError::NoHead)?;

    let head = core::str::from_utf8(head).map_err(|_| HttpError::NotText)?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or(HttpError::BadStatusLine)?;
    let (status, reason) = parse_status_line(status_line)?;

    let mut headers = Vec::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(HttpError::BadHeader)?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }

    let chunked = headers
        .iter()
        .any(|(name, value)| name == "transfer-encoding" && value.contains("chunked"));
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok());

    let body = if chunked {
        dechunk(rest)?
    } else if let Some(length) = length {
        // A body shorter than the length announced is what a truncated
        // stream looks like; the probe reports what arrived rather than
        // inventing the rest.
        rest.get(..length.min(rest.len())).unwrap_or(&[]).to_vec()
    } else {
        rest.to_vec()
    };

    Ok(Response {
        status,
        reason,
        headers,
        body,
    })
}

/// The code and the text of a status line.
fn parse_status_line(line: &str) -> Result<(u16, String), HttpError> {
    let mut parts = line.splitn(3, ' ');
    let version = parts.next().ok_or(HttpError::BadStatusLine)?;
    if !version.starts_with("HTTP/1.") {
        return Err(HttpError::BadStatusLine);
    }
    let code = parts.next().ok_or(HttpError::BadStatusLine)?;
    if code.len() != 3 {
        return Err(HttpError::BadStatus);
    }
    let status = code.parse::<u16>().map_err(|_| HttpError::BadStatus)?;
    let reason = parts.next().unwrap_or("").to_owned();
    Ok((status, reason))
}

/// Removes chunk framing, stopping at the terminating zero chunk.
fn dechunk(mut input: &[u8]) -> Result<Vec<u8>, HttpError> {
    let mut body = Vec::new();
    loop {
        let end = find(input, b"\r\n").ok_or(HttpError::BadChunk)?;
        let line = input.get(..end).ok_or(HttpError::BadChunk)?;
        // RFC 9112: a chunk size may be followed by extensions.
        let digits = line.split(|byte| *byte == b';').next().unwrap_or(line);
        let size = hex_size(digits)?;
        let after_line = input
            .get(end.saturating_add(2)..)
            .ok_or(HttpError::BadChunk)?;
        if size == 0 {
            return Ok(body);
        }
        let chunk = after_line.get(..size).ok_or(HttpError::BadChunk)?;
        body.extend_from_slice(chunk);
        input = after_line
            .get(size.saturating_add(2)..)
            .ok_or(HttpError::BadChunk)?;
    }
}

/// The first position of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// A chunk size, which is hexadecimal and has no sign.
fn hex_size(digits: &[u8]) -> Result<usize, HttpError> {
    if digits.is_empty() {
        return Err(HttpError::BadChunk);
    }
    let mut size = 0usize;
    for byte in digits {
        let value = match byte {
            b'0'..=b'9' => u32::from(byte.wrapping_sub(b'0')),
            b'a'..=b'f' => u32::from(byte.wrapping_sub(b'a')).saturating_add(10),
            b'A'..=b'F' => u32::from(byte.wrapping_sub(b'A')).saturating_add(10),
            _ => return Err(HttpError::BadChunk),
        };
        size = size
            .checked_mul(16)
            .and_then(|shifted| shifted.checked_add(value as usize))
            .ok_or(HttpError::BadChunk)?;
    }
    Ok(size)
}
