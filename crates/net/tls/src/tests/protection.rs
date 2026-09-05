// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Protecting and unprotecting records.

use crypto_aead::{Aead, Aes128Gcm};
use test_support::generators::range;
use test_support::property::check;

use crate::error::TlsError;
use crate::keys::traffic_keys;
use crate::protection::RecordProtection;
use crate::record::{ContentType, HEADER_LEN, MAX_PLAINTEXT, MAX_RECORD, read};
use crate::suite::CipherSuite;

/// A protection under a fixed secret.
fn protection(suite: CipherSuite) -> RecordProtection {
    let keys = traffic_keys(suite, &[0x5A; 48]).expect("the derivation fits");
    RecordProtection::new(keys).expect("the key fits the cipher")
}

#[test]
fn what_is_sealed_opens_again_under_every_suite() {
    for suite in [
        CipherSuite::Aes128GcmSha256,
        CipherSuite::Aes256GcmSha384,
        CipherSuite::ChaCha20Poly1305Sha256,
    ] {
        let mut sender = protection(suite);
        let mut receiver = protection(suite);

        let mut record = [0u8; 256];
        let length = sender
            .seal(ContentType::Handshake, b"a handshake message", &mut record)
            .expect("the buffer is big enough");

        let (header, body, used) = read(record.get(..length).unwrap_or(&[]))
            .expect("well formed")
            .expect("the record is complete");
        assert_eq!(
            header.content_type,
            ContentType::ApplicationData,
            "an encrypted record says nothing about what it carries"
        );
        assert_eq!(used, length);

        let mut body = body.to_vec();
        let head = record.get(..HEADER_LEN).unwrap_or(&[]).to_vec();
        let (inner, plaintext) = receiver.open(&head, &mut body).expect("the record opens");
        assert_eq!(inner, ContentType::Handshake, "{suite:?}");
        assert_eq!(plaintext, b"a handshake message");
    }
}

#[test]
fn every_record_of_an_epoch_uses_the_next_number() {
    let mut sender = protection(CipherSuite::Aes128GcmSha256);
    let mut receiver = protection(CipherSuite::Aes128GcmSha256);
    assert_eq!(sender.sequence(), 0);

    for round in 0..4u8 {
        let mut record = [0u8; 128];
        let length = sender
            .seal(ContentType::ApplicationData, &[round; 8], &mut record)
            .expect("the buffer is big enough");
        assert_eq!(sender.sequence(), u64::from(round) + 1);

        let head = record.get(..HEADER_LEN).unwrap_or(&[]).to_vec();
        let mut body = record.get(HEADER_LEN..length).unwrap_or(&[]).to_vec();
        let (inner, plaintext) = receiver.open(&head, &mut body).expect("the record opens");
        assert_eq!(inner, ContentType::ApplicationData);
        assert_eq!(plaintext, &[round; 8]);
        assert_eq!(receiver.sequence(), u64::from(round) + 1);
    }
}

#[test]
fn a_record_of_one_number_does_not_open_under_another() {
    let mut sender = protection(CipherSuite::Aes128GcmSha256);
    let mut receiver = protection(CipherSuite::Aes128GcmSha256);

    // Throw the first record away, so that the receiver is a record behind.
    let mut first = [0u8; 128];
    sender
        .seal(ContentType::Handshake, b"one", &mut first)
        .expect("the buffer is big enough");

    let mut second = [0u8; 128];
    let length = sender
        .seal(ContentType::Handshake, b"two", &mut second)
        .expect("the buffer is big enough");

    let head = second.get(..HEADER_LEN).unwrap_or(&[]).to_vec();
    let mut body = second.get(HEADER_LEN..length).unwrap_or(&[]).to_vec();
    assert_eq!(
        receiver.open(&head, &mut body).err(),
        Some(TlsError::BadRecord),
        "the numbers no longer agree"
    );
}

#[test]
fn a_damaged_record_does_not_open() {
    let mut sender = protection(CipherSuite::Aes128GcmSha256);
    let mut record = [0u8; 128];
    let length = sender
        .seal(ContentType::Handshake, b"a message", &mut record)
        .expect("the buffer is big enough");

    for position in 0..length {
        let mut damaged = record;
        if let Some(slot) = damaged.get_mut(position) {
            *slot ^= 0x01;
        }
        let head = damaged.get(..HEADER_LEN).unwrap_or(&[]).to_vec();
        let mut body = damaged.get(HEADER_LEN..length).unwrap_or(&[]).to_vec();
        let mut receiver = protection(CipherSuite::Aes128GcmSha256);
        assert_eq!(
            receiver.open(&head, &mut body).err(),
            Some(TlsError::BadRecord),
            "damaged at {position}"
        );
    }
}

#[test]
fn a_body_shorter_than_a_tag_is_not_a_record() {
    let mut receiver = protection(CipherSuite::Aes128GcmSha256);
    let head = [23u8, 0x03, 0x03, 0x00, 0x04];
    let mut body = [0u8; 4];
    assert_eq!(
        receiver.open(&head, &mut body).err(),
        Some(TlsError::BadRecord)
    );
}

#[test]
fn padding_is_stripped_and_a_record_of_only_padding_is_refused() {
    // The sealing of this crate writes no padding, so the padded record is
    // built here with the same keys and the same nonce.
    let keys =
        traffic_keys(CipherSuite::Aes128GcmSha256, &[0x5A; 48]).expect("the derivation fits");
    let cipher = Aes128Gcm::new(keys.key()).expect("the key fits");

    for (name, inner, expected) in [
        ("padded", &b"hello\x16\x00\x00\x00"[..], Ok(&b"hello"[..])),
        ("only padding", &[0x00, 0x00][..], Err(TlsError::BadRecord)),
        ("no type at all", &[][..], Err(TlsError::BadRecord)),
    ] {
        let mut body = inner.to_vec();
        let fragment = body.len() + 16;
        let mut head = [0u8; HEADER_LEN];
        crate::record::write_header(ContentType::ApplicationData, fragment, &mut head)
            .expect("the buffer is big enough");

        let tag = cipher
            .seal(&keys.nonce(0), &head, &mut body)
            .expect("the message is well formed");
        body.extend_from_slice(&tag);

        let mut receiver = protection(CipherSuite::Aes128GcmSha256);
        match (receiver.open(&head, &mut body), expected) {
            (Ok((inner_type, plaintext)), Ok(wanted)) => {
                assert_eq!(inner_type, ContentType::Handshake, "{name}");
                assert_eq!(plaintext, wanted, "{name}");
            }
            (Err(found), Err(wanted)) => assert_eq!(found, wanted, "{name}"),
            (found, wanted) => panic!("{name}: {found:?} against {wanted:?}"),
        }
    }
}

#[test]
fn a_plaintext_beyond_the_limit_and_a_buffer_below_it_are_refused() {
    let mut sender = protection(CipherSuite::Aes128GcmSha256);
    let long = vec![0u8; MAX_PLAINTEXT + 1];
    let mut out = vec![0u8; MAX_RECORD];
    assert_eq!(
        sender.seal(ContentType::ApplicationData, &long, &mut out),
        Err(TlsError::RecordOverflow)
    );

    let mut small = [0u8; 8];
    assert_eq!(
        sender.seal(ContentType::Handshake, b"a message", &mut small),
        Err(TlsError::BufferTooSmall)
    );
    assert_eq!(sender.sequence(), 0, "a refused record counts for nothing");
}

#[test]
fn property_a_message_of_any_length_survives_the_round_trip() {
    check("tls_record_round_trip", &range(0..=2048usize), |length| {
        let mut sender = protection(CipherSuite::ChaCha20Poly1305Sha256);
        let mut receiver = protection(CipherSuite::ChaCha20Poly1305Sha256);

        let plaintext: Vec<u8> = (0..*length)
            .map(|index| u8::try_from(index % 251).unwrap_or(0))
            .collect();
        let mut record = vec![0u8; MAX_RECORD];
        let written = sender
            .seal(ContentType::ApplicationData, &plaintext, &mut record)
            .map_err(|error| format!("seal: {error}"))?;

        let head = record.get(..HEADER_LEN).unwrap_or(&[]).to_vec();
        let mut body = record.get(HEADER_LEN..written).unwrap_or(&[]).to_vec();
        let (inner, opened) = receiver
            .open(&head, &mut body)
            .map_err(|error| format!("open: {error}"))?;
        if inner != ContentType::ApplicationData {
            return Err("the type did not survive".to_owned());
        }
        if opened != plaintext {
            return Err("the message did not survive".to_owned());
        }
        Ok(())
    });
}

/// The last number of an epoch is used once, and then the epoch is over.
#[test]
fn the_last_number_of_an_epoch_is_spent_once_and_never_again() {
    let mut sender = protection(CipherSuite::Aes128GcmSha256);
    let mut receiver = protection(CipherSuite::Aes128GcmSha256);
    sender.seek(u64::MAX);
    receiver.seek(u64::MAX);

    // The last record still goes out, and still opens.
    let mut record = [0u8; 128];
    let length = sender
        .seal(ContentType::ApplicationData, b"the last one", &mut record)
        .expect("the last number is a usable one");
    let head = record.get(..HEADER_LEN).unwrap_or(&[]).to_vec();
    let mut body = record.get(HEADER_LEN..length).unwrap_or(&[]).to_vec();
    let (inner, plaintext) = receiver.open(&head, &mut body).expect("the record opens");
    assert_eq!(inner, ContentType::ApplicationData);
    assert_eq!(plaintext, b"the last one");

    // And after it there is nothing left: no second record under the same
    // number, in either direction.
    assert_eq!(
        sender.seal(ContentType::ApplicationData, b"one more", &mut record),
        Err(TlsError::SequenceExhausted)
    );
    let mut again = record.get(HEADER_LEN..length).unwrap_or(&[]).to_vec();
    assert_eq!(
        receiver.open(&head, &mut again),
        Err(TlsError::SequenceExhausted)
    );
}
