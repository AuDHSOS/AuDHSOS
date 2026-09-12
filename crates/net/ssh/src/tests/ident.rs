// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The identification string of RFC 4253, section 4.2.

use crate::error::SshError;
use crate::ident::{self, CLIENT, Greeting, MAX_LEN, PREFIX};

#[test]
fn what_this_client_sends_is_what_the_document_describes() {
    let mut out = [0u8; 64];
    let len = ident::write(&mut out).unwrap_or_default();
    let written = out.get(..len).unwrap_or(&[]);
    assert!(written.starts_with(PREFIX.as_bytes()));
    assert!(written.ends_with(b"\r\n"));
    assert_eq!(len, CLIENT.len() + 2);
    assert!(len <= MAX_LEN);
    // Section 4.2: the software version is printable US-ASCII with no
    // space and no minus in it.
    let software = CLIENT.strip_prefix(PREFIX).unwrap_or_default();
    assert!(!software.is_empty());
    assert!(
        software
            .bytes()
            .all(|byte| (0x21..=0x7e).contains(&byte) && byte != b'-')
    );
}

#[test]
fn a_buffer_too_small_for_the_identification_writes_nothing() {
    let mut out = [0u8; 4];
    assert_eq!(
        ident::write(&mut out),
        Err(SshError::OutOfBounds {
            needed: CLIENT.len() + 2,
            available: 4
        })
    );
    assert_eq!(out, [0u8; 4]);
}

#[test]
fn the_peers_identification_is_the_line_without_its_carriage_return() {
    let bytes = b"SSH-2.0-OpenSSH_9.6\r\n\x00\x00\x00\x0c";
    assert_eq!(
        ident::read(bytes),
        Ok(Greeting::Identification {
            line: "SSH-2.0-OpenSSH_9.6",
            length: 21
        })
    );
}

#[test]
fn the_lines_a_server_sends_first_are_skipped_and_counted() {
    // Section 4.2: the server MAY send other lines first, they MUST NOT
    // begin with "SSH-", and a client MUST be able to process them.
    let bytes = b"a banner\r\nand another\r\nSSH-2.0-OpenSSH_9.6\r\nrest";
    assert_eq!(
        ident::read(bytes),
        Ok(Greeting::Identification {
            line: "SSH-2.0-OpenSSH_9.6",
            length: 44
        })
    );
    let consumed = match ident::read(bytes) {
        Ok(Greeting::Identification { length, .. }) => length,
        _ => 0,
    };
    assert_eq!(bytes.get(consumed..), Some(b"rest".as_slice()));
}

#[test]
fn nothing_is_said_before_a_line_has_ended() {
    assert_eq!(ident::read(b""), Ok(Greeting::Incomplete));
    assert_eq!(
        ident::read(b"SSH-2.0-OpenSSH_9.6"),
        Ok(Greeting::Incomplete)
    );
    assert_eq!(
        ident::read(b"SSH-2.0-OpenSSH_9.6\r"),
        Ok(Greeting::Incomplete)
    );
    assert_eq!(
        ident::read(b"a banner\r\nSSH-2.0-X"),
        Ok(Greeting::Incomplete)
    );
}

#[test]
fn a_line_that_cannot_be_one_is_refused_where_it_is_read() {
    // Longer than the 255 characters of section 4.2, as an
    // identification and as a line before one, ended and unended: the
    // answer is the same four times, so it does not turn on how the bytes
    // arrived.
    let mut identification = b"SSH-2.0-".to_vec();
    identification.extend(core::iter::repeat_n(b'x', MAX_LEN));
    identification.extend_from_slice(b"\r\n");
    assert_eq!(ident::read(&identification), Err(SshError::Identification));

    let mut banner = vec![b'x'; MAX_LEN];
    assert_eq!(ident::read(&banner), Err(SshError::Identification));
    banner.extend_from_slice(b"\r\n");
    assert_eq!(ident::read(&banner), Err(SshError::Identification));

    // One byte under the limit is a line, ended or not.
    let mut short = vec![b'x'; MAX_LEN - 2];
    assert_eq!(ident::read(&short), Ok(Greeting::Incomplete));
    short.extend_from_slice(b"\r\nSSH-2.0-X\r\n");
    assert_eq!(
        ident::read(&short),
        Ok(Greeting::Identification {
            line: "SSH-2.0-X",
            length: MAX_LEN + 11
        })
    );
}

#[test]
fn a_version_this_client_does_not_speak_is_refused() {
    // Section 5.1 lets a server that speaks both versions say 1.99. This
    // client speaks one version and takes only that one.
    assert_eq!(
        ident::read(b"SSH-1.99-OpenSSH_9.6\r\n"),
        Err(SshError::Identification)
    );
    assert_eq!(
        ident::read(b"SSH-1.5-OldServer\r\n"),
        Err(SshError::Identification)
    );
    assert_eq!(ident::read(b"SSH-2.0-\r\n"), Err(SshError::Identification));
}

#[test]
fn an_identification_that_is_not_printable_us_ascii_is_refused() {
    assert_eq!(
        ident::read("SSH-2.0-Serverü\r\n".as_bytes()),
        Err(SshError::Identification)
    );
    assert_eq!(
        ident::read(b"SSH-2.0-Server\x07\r\n"),
        Err(SshError::Identification)
    );
    assert_eq!(
        ident::read(b"SSH-2.0-Server\x00x\r\n"),
        Err(SshError::Identification)
    );
    // Bytes that are not UTF-8 at all, which no str can hold.
    assert_eq!(
        ident::read(b"SSH-2.0-Server\xff\r\n"),
        Err(SshError::Identification)
    );
}

#[test]
fn what_was_written_is_what_a_peer_would_read() {
    let mut out = [0u8; 64];
    let len = ident::write(&mut out).unwrap_or_default();
    assert_eq!(
        ident::read(out.get(..len).unwrap_or(&[])),
        Ok(Greeting::Identification {
            line: CLIENT,
            length: len
        })
    );
}
