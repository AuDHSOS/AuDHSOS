// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Record boundaries and headers.

use crate::error::TlsError;
use crate::record::{
    ContentType, HEADER_LEN, MAX_CIPHERTEXT, MAX_PLAINTEXT, MAX_RECORD, read, write_header,
};

#[test]
fn a_record_is_read_when_all_of_it_has_arrived() {
    let mut bytes = vec![22, 0x03, 0x03, 0x00, 0x04];
    bytes.extend_from_slice(&[1, 2, 3, 4]);

    let (header, body, used) = read(&bytes)
        .expect("the record is well formed")
        .expect("the record is complete");
    assert_eq!(header.content_type, ContentType::Handshake);
    assert_eq!(header.length, 4);
    assert_eq!(body, &[1, 2, 3, 4]);
    assert_eq!(used, HEADER_LEN + 4);
}

#[test]
fn a_record_that_has_not_arrived_in_full_is_not_a_record_yet() {
    let whole = [23u8, 0x03, 0x03, 0x00, 0x02, 0xAA, 0xBB];
    for prefix in 0..whole.len() {
        let partial = whole.get(..prefix).unwrap_or(&[]);
        assert_eq!(read(partial), Ok(None), "{prefix} bytes");
    }
    assert!(read(&whole).expect("well formed").is_some());
}

#[test]
fn what_follows_a_record_is_left_alone() {
    let mut bytes = vec![21u8, 0x03, 0x03, 0x00, 0x02, 0x01, 0x02];
    bytes.extend_from_slice(&[22, 0x03, 0x03, 0x00, 0x00]);

    let (header, body, used) = read(&bytes)
        .expect("well formed")
        .expect("the first record is complete");
    assert_eq!(header.content_type, ContentType::Alert);
    assert_eq!(body, &[0x01, 0x02]);

    let rest = bytes.get(used..).unwrap_or(&[]);
    let (second, body, _) = read(rest)
        .expect("well formed")
        .expect("the second record is complete");
    assert_eq!(second.content_type, ContentType::Handshake);
    assert!(body.is_empty());
}

#[test]
fn the_version_field_is_not_checked() {
    // RFC 8446 section 5.1: a receiver must ignore it.
    for version in [[0x03u8, 0x01], [0x03, 0x03], [0xFF, 0xFF]] {
        let bytes = [22, version[0], version[1], 0x00, 0x00];
        let (header, _, _) = read(&bytes)
            .expect("well formed")
            .expect("the record is complete");
        assert_eq!(header.content_type, ContentType::Handshake);
    }
}

#[test]
fn a_type_that_is_not_one_of_the_four_is_refused() {
    for byte in [0u8, 19, 24, 255] {
        let bytes = [byte, 0x03, 0x03, 0x00, 0x00];
        assert_eq!(read(&bytes), Err(TlsError::UnknownContentType), "{byte}");
        assert_eq!(
            ContentType::from_byte(byte),
            Err(TlsError::UnknownContentType)
        );
    }
    for (byte, expected) in [
        (20u8, ContentType::ChangeCipherSpec),
        (21, ContentType::Alert),
        (22, ContentType::Handshake),
        (23, ContentType::ApplicationData),
    ] {
        assert_eq!(ContentType::from_byte(byte), Ok(expected));
        assert_eq!(expected.to_byte(), byte);
    }
}

#[test]
fn a_length_beyond_the_limit_is_refused_before_the_body_arrives() {
    let too_long = u16::try_from(MAX_CIPHERTEXT + 1).expect("the limit fits");
    let bytes = [
        23,
        0x03,
        0x03,
        too_long.to_be_bytes()[0],
        too_long.to_be_bytes()[1],
    ];
    assert_eq!(read(&bytes), Err(TlsError::RecordOverflow));

    // The largest that is allowed is allowed.
    let allowed = u16::try_from(MAX_CIPHERTEXT).expect("the limit fits");
    let mut bytes = vec![
        23,
        0x03,
        0x03,
        allowed.to_be_bytes()[0],
        allowed.to_be_bytes()[1],
    ];
    assert_eq!(read(&bytes), Ok(None), "the body has not arrived");
    bytes.extend(core::iter::repeat_n(0u8, MAX_CIPHERTEXT));
    assert!(read(&bytes).expect("well formed").is_some());
    assert_eq!(bytes.len(), MAX_RECORD);
}

#[test]
fn a_header_is_written_as_it_is_read() {
    let mut out = [0u8; HEADER_LEN];
    write_header(ContentType::ApplicationData, 0x1234, &mut out).expect("the buffer is big enough");
    assert_eq!(out, [23, 0x03, 0x03, 0x12, 0x34]);

    let mut bytes = out.to_vec();
    bytes.extend(core::iter::repeat_n(0u8, 0x1234));
    let (header, _, _) = read(&bytes)
        .expect("well formed")
        .expect("the record is complete");
    assert_eq!(header.content_type, ContentType::ApplicationData);
    assert_eq!(header.length, 0x1234);
}

#[test]
fn a_header_that_does_not_fit_or_does_not_hold_is_refused() {
    let mut small = [0u8; HEADER_LEN - 1];
    assert_eq!(
        write_header(ContentType::Handshake, 1, &mut small),
        Err(TlsError::BufferTooSmall)
    );

    let mut out = [0u8; HEADER_LEN];
    assert_eq!(
        write_header(ContentType::Handshake, MAX_CIPHERTEXT + 1, &mut out),
        Err(TlsError::RecordOverflow)
    );
    assert_eq!(MAX_PLAINTEXT + 256, MAX_CIPHERTEXT);
}
