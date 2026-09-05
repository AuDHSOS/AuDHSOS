// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The record layer against arbitrary bytes: framing, opening under keys
//! the input does not know, and the round trip.
//!
//! A record that frames must say how long it is, and no record may open
//! under keys that were never used to seal it. The round trip runs the
//! other way: what this crate seals, this crate opens again, and the bytes
//! that come back are the bytes that went in.

#![cfg_attr(fuzzing, no_main)]

use audhsos_tls::keys::traffic_keys;
use audhsos_tls::record::{self, HEADER_LEN, MAX_PLAINTEXT};
use audhsos_tls::{CipherSuite, ContentType, RecordProtection};

/// The suite the target works in. One is enough: the framing does not
/// depend on it, and the ciphers have targets of their own in `crypto-aead`.
const SUITE: CipherSuite = CipherSuite::Aes128GcmSha256;

/// A secret nobody negotiated, which is the point: no input may open a
/// record under it.
const SECRET: [u8; 32] = [0x2B; 32];

/// Protection under that secret.
fn protection() -> RecordProtection {
    let keys = traffic_keys(SUITE, &SECRET).expect("the suite and the secret fit");
    RecordProtection::new(keys).expect("the keys fit the cipher")
}

fuzz_support::fuzz_target!(|bytes: &[u8]| {
    frame(bytes);
    round_trip(bytes);
});

/// Reads the input as a record and opens it under keys it does not know.
fn frame(bytes: &[u8]) {
    // The framing.
    if let Ok(Some((_, body, used))) = record::read(bytes) {
        assert!(used <= bytes.len(), "a record longer than the input");
        assert_eq!(
            used,
            HEADER_LEN.saturating_add(body.len()),
            "a record whose length is not its header and its body"
        );

        // What a receiver does with it.
        let header = bytes.get(..HEADER_LEN).unwrap_or(&[]).to_vec();
        let mut copy = body.to_vec();
        if let Ok((_, plaintext)) = protection().open(&header, &mut copy) {
            assert!(
                plaintext.len() < body.len(),
                "a plaintext no shorter than the record that carried it"
            );
        }
    }
}

/// Seals the input and opens it again.
fn round_trip(bytes: &[u8]) {
    let plaintext = bytes.get(..bytes.len().min(MAX_PLAINTEXT)).unwrap_or(&[]);
    let mut out = vec![0u8; plaintext.len().saturating_add(HEADER_LEN + 32)];
    let mut sender = protection();
    let Ok(length) = sender.seal(ContentType::ApplicationData, plaintext, &mut out) else {
        return;
    };
    let (header, rest) = out.split_at(HEADER_LEN);
    let header = header.to_vec();
    let mut sealed = rest
        .get(..length.saturating_sub(HEADER_LEN))
        .unwrap_or(&[])
        .to_vec();
    let (kind, opened) = protection()
        .open(&header, &mut sealed)
        .expect("what this crate sealed, this crate opens");
    assert_eq!(
        kind,
        ContentType::ApplicationData,
        "the type did not survive"
    );
    assert_eq!(opened, plaintext, "the message did not survive");
}
