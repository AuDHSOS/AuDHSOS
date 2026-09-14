// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The anchor table an image carries.

use crate::anchors::{Anchors, HEADER_LEN, MAGIC, VERSION, table_len, write};
use crate::builder::{Params, TestKey};
use crate::error::X509Error;
use crate::path::TrustAnchor;
use crate::tests::{AUTHORITY_SECRET, Built, build_certificate, early, late};

/// Room for the tables these tests write.
const ROOM: usize = 8192;

/// A self-signed root under the name it is given.
fn root(name: &str) -> Built {
    let params = Params::authority(name, name, None, early(), late());
    build_certificate(
        &params,
        TestKey::Ed25519(AUTHORITY_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    )
    .expect("the parameters fit")
}

/// The table of `certificates`, and how much of the buffer it takes.
fn written(certificates: &[&[u8]]) -> ([u8; ROOM], usize) {
    let mut out = [0u8; ROOM];
    let len = write(certificates, &mut out).expect("the table fits");
    (out, len)
}

#[test]
fn a_table_of_no_anchors_is_a_header_and_nothing_else() {
    let (bytes, len) = written(&[]);
    assert_eq!(len, HEADER_LEN);
    let table = Anchors::parse(bytes.get(..len).expect("the table was written"))
        .expect("a header this crate wrote");
    assert_eq!(table.len(), 0);
    assert!(table.is_empty());
    assert_eq!(table.certificates().count(), 0);
}

#[test]
fn every_certificate_written_comes_back_byte_for_byte() {
    let first = root("First");
    let second = root("Second");
    let (bytes, len) = written(&[first.as_slice(), second.as_slice()]);
    let table =
        Anchors::parse(bytes.get(..len).expect("the table was written")).expect("a valid table");

    assert_eq!(table.len(), 2);
    let read: Vec<&[u8]> = table.certificates().collect();
    assert_eq!(read, vec![first.as_slice(), second.as_slice()]);
    assert_eq!(
        len,
        table_len(&[first.as_slice(), second.as_slice()]).unwrap()
    );
}

#[test]
fn the_anchors_of_a_table_are_the_subjects_and_keys_of_its_certificates() {
    let first = root("First");
    let second = root("Second");
    let (bytes, len) = written(&[first.as_slice(), second.as_slice()]);
    let table =
        Anchors::parse(bytes.get(..len).expect("the table was written")).expect("a valid table");

    let mut anchors = [TrustAnchor {
        subject: &[],
        spki: &[],
    }; 4];
    assert_eq!(table.read_into(&mut anchors).expect("the anchors read"), 2);
    for (anchor, certificate) in anchors.iter().zip([first.as_slice(), second.as_slice()]) {
        let expected = TrustAnchor::from_certificate(certificate).expect("a usable certificate");
        assert_eq!(anchor.subject, expected.subject);
        assert_eq!(anchor.spki, expected.spki);
    }
    assert_ne!(anchors[0].subject, anchors[1].subject);
}

#[test]
fn an_array_shorter_than_the_table_is_refused_before_a_certificate_is_read() {
    let first = root("First");
    let second = root("Second");
    let (bytes, len) = written(&[first.as_slice(), second.as_slice()]);
    let table =
        Anchors::parse(bytes.get(..len).expect("the table was written")).expect("a valid table");

    let mut one = [TrustAnchor {
        subject: &[],
        spki: &[],
    }; 1];
    assert_eq!(table.read_into(&mut one), Err(X509Error::BufferTooSmall));
}

#[test]
fn a_buffer_shorter_than_the_table_takes_nothing() {
    let first = root("First");
    let mut out = [0u8; HEADER_LEN + 4];
    assert_eq!(
        write(&[first.as_slice()], &mut out),
        Err(X509Error::BufferTooSmall)
    );
}

#[test]
fn a_header_this_crate_did_not_write_is_refused() {
    let first = root("First");
    let (mut bytes, len) = written(&[first.as_slice()]);

    let mut wrong_magic = bytes;
    wrong_magic[0] = MAGIC[0] ^ 0xff;
    assert_eq!(
        Anchors::parse(wrong_magic.get(..len).expect("written")).unwrap_err(),
        X509Error::BadAnchorTable
    );

    let next_version = VERSION.saturating_add(1).to_le_bytes();
    bytes[4] = next_version[0];
    bytes[5] = next_version[1];
    assert_eq!(
        Anchors::parse(bytes.get(..len).expect("written")).unwrap_err(),
        X509Error::BadAnchorTable
    );

    assert_eq!(
        Anchors::parse(b"ANCH").unwrap_err(),
        X509Error::BadAnchorTable
    );
}

#[test]
fn a_length_that_reaches_past_the_end_is_refused() {
    let first = root("First");
    let (bytes, len) = written(&[first.as_slice()]);
    let mut truncated: Vec<u8> = bytes.get(..len).expect("written").to_vec();
    truncated.pop();
    assert_eq!(
        Anchors::parse(&truncated).unwrap_err(),
        X509Error::BadAnchorTable
    );
}

#[test]
fn a_record_of_no_bytes_is_refused() {
    let mut table = Vec::new();
    table.extend_from_slice(&MAGIC);
    table.extend_from_slice(&VERSION.to_le_bytes());
    table.extend_from_slice(&1u16.to_le_bytes());
    table.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        Anchors::parse(&table).unwrap_err(),
        X509Error::BadAnchorTable
    );
    assert_eq!(table_len(&[&[]]), Err(X509Error::BadAnchorTable));
}

#[test]
fn a_byte_behind_the_last_record_is_refused() {
    let first = root("First");
    let (bytes, len) = written(&[first.as_slice()]);
    let mut appended: Vec<u8> = bytes.get(..len).expect("written").to_vec();
    appended.push(0);
    assert_eq!(
        Anchors::parse(&appended).unwrap_err(),
        X509Error::BadAnchorTable
    );
}

#[test]
fn a_record_that_is_no_certificate_is_refused_where_it_is_read_and_not_where_it_is_parsed() {
    let (bytes, len) = written(&[&[0x30, 0x00]]);
    let table =
        Anchors::parse(bytes.get(..len).expect("written")).expect("the table is well formed");
    assert_eq!(table.len(), 1);

    let mut anchors = [TrustAnchor {
        subject: &[],
        spki: &[],
    }; 1];
    assert!(table.read_into(&mut anchors).is_err());
}
