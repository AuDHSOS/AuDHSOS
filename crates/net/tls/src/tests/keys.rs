// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The key schedule.
//!
//! RFC 8446 publishes no vectors of its own; the traces of RFC 8448 do,
//! and they need whole handshake messages, which arrive with the state
//! machine. Until then the expected values here were computed from
//! section 7.1 by an implementation outside this repository, over inputs
//! this file fixes: a shared value of the bytes zero to thirty-one, and
//! two transcripts that are the hashes of two literal strings.

use crypto_hash::{Sha256, Sha384};

use crate::error::TlsError;
use crate::keys::{Schedule, expand_label, finished_key, next_traffic_secret, traffic_keys};
use crate::secret::Secret;
use crate::suite::CipherSuite;
use crate::tests::hex;

/// The stand-in shared value.
fn shared() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    for (slot, value) in bytes.iter_mut().zip(0u8..) {
        *slot = value;
    }
    bytes
}

/// The transcript of the first stage, as the reference computed it.
fn first_transcript(suite: CipherSuite) -> Secret {
    match suite {
        CipherSuite::Aes256GcmSha384 => {
            Secret::from_slice(Sha384::digest(b"ClientHello...ServerHello").as_ref())
        }
        _ => Secret::from_slice(Sha256::digest(b"ClientHello...ServerHello").as_ref()),
    }
}

/// The transcript of the second stage.
fn second_transcript(suite: CipherSuite) -> Secret {
    match suite {
        CipherSuite::Aes256GcmSha384 => {
            Secret::from_slice(Sha384::digest(b"ClientHello...server Finished").as_ref())
        }
        _ => Secret::from_slice(Sha256::digest(b"ClientHello...server Finished").as_ref()),
    }
}

/// What one suite's schedule must produce.
struct Expected {
    /// The suite.
    suite: CipherSuite,
    /// The early secret.
    early: &'static str,
    /// The handshake secret.
    handshake: &'static str,
    /// The client's handshake traffic secret.
    client_handshake: &'static str,
    /// The server's handshake traffic secret.
    server_handshake: &'static str,
    /// The master secret.
    master: &'static str,
    /// The client's application traffic secret.
    client_application: &'static str,
    /// The key of the client's handshake secret.
    key: &'static str,
    /// The nonce base of the client's handshake secret.
    iv: &'static str,
    /// The finished key of the client's handshake secret.
    finished: &'static str,
    /// The application secret after one update.
    updated: &'static str,
}

/// The two suites, with the values the reference computed.
const CASES: [Expected; 2] = [
    Expected {
        suite: CipherSuite::Aes128GcmSha256,
        early: "33ad0a1c607ec03b09e6cd9893680ce210adf300aa1f2660e1b22e10f170f92a",
        handshake: "ddbe37614d014a8c19db0a47955ee6930b3ee727c408386ba274344962b0e015",
        client_handshake: "ec5086eb35c0c12c9da5b0cd676d5a635db96461e0dc101193094c52e02891f6",
        server_handshake: "5cf96070f839db4d2f977be4f406e98e7475b0dc67152ca4cef8cb93af3b99ad",
        master: "055796c9a2f048dd920b01351ba00d131ba528efa37749efb03e31b643c615b2",
        client_application: "b48c5081db11310cb7806fbc4a18173a95ce267062fcd6e5449772856882f769",
        key: "2ad4f2d2992682a32ae219379f7d602b",
        iv: "7d0c24b9d9c06d126248dcdd",
        finished: "b90546fcd1951203c95fefb37266d040039804716310625c9c832cb480dafbb0",
        updated: "828e12b391b0ff105833441003f99c53c9429db9b0750f30465b05528a92d302",
    },
    Expected {
        suite: CipherSuite::Aes256GcmSha384,
        early: "7ee8206f5570023e6dc7519eb1073bc4e791ad37b5c382aa10ba18e2357e7169\
                71f9362f2c2fe2a76bfd78dfec4ea9b5",
        handshake: "03e67b8797b8868b32c737ad09d2454266b47b828fe83999484501c1d0f12bfe\
                    fdecf2341ab055c50f8f81a4d91610dd",
        client_handshake: "fa59931bd914b4fb6e690141e40077b75f4bd1ad218a88724d53545d50b5cc21\
                           52f700f6cedceb2512a3f33803ffe3ba",
        server_handshake: "4ce52ed76c36f9313bf6602507c414e4fe0d1029e3580ffb16b278856ffc0645\
                           8e506b5d3a92eef9d239eceaf7d6b4c1",
        master: "16d23b35c31232dd08cf8189bfb64942b4232e1b8787e94fa3fbf16413dc38ed\
                 5a7b3410e633a2cc9c790e22541ffae7",
        client_application: "300dfd9aea2ca2b97f72f0ebb7ad01de9c42c098bd0f1ee58c1c6def402bae02\
                             0286ec31fd61a2b66170f9cccb84ee17",
        key: "a96b4f3dbf6f120445018850001873a0addc1f237866345780d274e571f14210",
        iv: "14c3b8a387d26dd189400fa5",
        finished: "b507925fa184e4530e2666ae2ebd09fcf0cdee6174a6dc75bc68ed18d0285544\
                   9c75571c510e86c07ad83e30e0ad94e3",
        updated: "cdebe4e0dc6c11d8209e475b3136f78046e0e8e5004af095ed5d1a7fb0cfce24\
                  782f2cd3645f19f403d7243814b047cc",
    },
];

/// A hexadecimal constant without the wrapping the source needed.
fn joined(text: &str) -> String {
    text.replace([' ', '\n'], "")
}

#[test]
fn the_schedule_reaches_the_secrets_the_specification_prescribes() {
    for case in &CASES {
        let suite = case.suite;
        let mut schedule = Schedule::new(suite);
        assert_eq!(hex(schedule.secret()), joined(case.early), "early");

        schedule
            .advance(&shared())
            .expect("the shared value is well formed");
        assert_eq!(hex(schedule.secret()), joined(case.handshake), "handshake");

        let transcript = first_transcript(suite);
        let client = schedule
            .derive(b"c hs traffic", transcript.as_bytes())
            .expect("the derivation fits");
        let server = schedule
            .derive(b"s hs traffic", transcript.as_bytes())
            .expect("the derivation fits");
        assert_eq!(hex(client.as_bytes()), joined(case.client_handshake));
        assert_eq!(hex(server.as_bytes()), joined(case.server_handshake));

        schedule.advance_to_master().expect("the derivation fits");
        assert_eq!(hex(schedule.secret()), joined(case.master), "master");

        let second = second_transcript(suite);
        let application = schedule
            .derive(b"c ap traffic", second.as_bytes())
            .expect("the derivation fits");
        assert_eq!(hex(application.as_bytes()), joined(case.client_application));

        let keys = traffic_keys(suite, client.as_bytes()).expect("the derivation fits");
        assert_eq!(hex(keys.key()), joined(case.key), "key");
        assert_eq!(hex(keys.iv()), joined(case.iv), "iv");
        assert_eq!(keys.key().len(), suite.key_len());

        let finished = finished_key(suite, client.as_bytes()).expect("the derivation fits");
        assert_eq!(hex(finished.as_bytes()), joined(case.finished), "finished");

        let updated =
            next_traffic_secret(suite, application.as_bytes()).expect("the derivation fits");
        assert_eq!(hex(updated.as_bytes()), joined(case.updated), "updated");
    }
}

#[test]
fn the_third_suite_shares_the_schedule_of_the_first() {
    // Both name SHA-256, so every secret is the same; only the key length
    // differs, because the ciphers do.
    let mut first = Schedule::new(CipherSuite::Aes128GcmSha256);
    let mut third = Schedule::new(CipherSuite::ChaCha20Poly1305Sha256);
    first.advance(&shared()).expect("the value is well formed");
    third.advance(&shared()).expect("the value is well formed");
    assert_eq!(hex(first.secret()), hex(third.secret()));

    let keys = traffic_keys(CipherSuite::ChaCha20Poly1305Sha256, first.secret())
        .expect("the derivation fits");
    assert_eq!(keys.key().len(), 32);
    assert_eq!(keys.suite(), CipherSuite::ChaCha20Poly1305Sha256);
}

#[test]
fn the_nonce_is_the_base_with_the_sequence_number_in_its_tail() {
    let keys =
        traffic_keys(CipherSuite::Aes128GcmSha256, &[0x42; 32]).expect("the derivation fits");
    let base = *keys.iv();

    assert_eq!(keys.nonce(0), base, "the first record uses the base itself");

    let one = keys.nonce(1);
    let (head, tail) = base.split_at(4);
    assert_eq!(one.get(..4), Some(head), "the head is untouched");
    let mut expected = [0u8; 8];
    for (slot, byte) in expected.iter_mut().zip(tail) {
        *slot = *byte;
    }
    if let Some(last) = expected.last_mut() {
        *last ^= 1;
    }
    assert_eq!(one.get(4..), Some(&expected[..]));

    // A number that touches every byte of the tail.
    let many = keys.nonce(u64::MAX);
    for (index, byte) in many.iter().enumerate().skip(4) {
        let original = base.get(index).copied().unwrap_or(0);
        assert_eq!(*byte, original ^ 0xFF, "byte {index}");
    }
}

#[test]
fn a_derivation_that_does_not_fit_is_refused() {
    let mut out = [0u8; 16];
    assert_eq!(
        expand_label(
            CipherSuite::Aes128GcmSha256,
            &[0x01; 32],
            b"a label that is far too long to be one",
            &[],
            &mut out
        ),
        Err(TlsError::BadDerivation),
        "the label"
    );

    assert_eq!(
        expand_label(
            CipherSuite::Aes128GcmSha256,
            &[0x01; 32],
            b"key",
            &[0x02; 64],
            &mut out
        ),
        Err(TlsError::BadDerivation),
        "the context"
    );

    let mut huge = vec![0u8; 70_000];
    assert_eq!(
        expand_label(
            CipherSuite::Aes128GcmSha256,
            &[0x01; 32],
            b"key",
            &[],
            &mut huge
        ),
        Err(TlsError::BadDerivation),
        "the output"
    );
}

#[test]
fn a_secret_says_its_length_and_nothing_else() {
    let secret = Secret::from_slice(&[0xAB; 32]);
    assert_eq!(secret.len(), 32);
    assert!(!secret.is_empty());
    assert!(Secret::zero(0).is_empty());
    assert_eq!(format!("{secret:?}"), "Secret<32>");
    assert!(secret.ct_eq(&Secret::from_slice(&[0xAB; 32])).is_true());
    assert!(!secret.ct_eq(&Secret::from_slice(&[0xAC; 32])).is_true());
}

#[test]
fn the_errors_render_a_message() {
    for error in [
        TlsError::RecordOverflow,
        TlsError::UnknownContentType,
        TlsError::UnexpectedMessage,
        TlsError::BadRecord,
        TlsError::SequenceExhausted,
        TlsError::BufferTooSmall,
        TlsError::BadDerivation,
        TlsError::TranscriptNotStarted,
    ] {
        assert!(!format!("{error}").is_empty());
        assert!(!format!("{error:?}").is_empty());
    }
}
