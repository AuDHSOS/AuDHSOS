// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The running hash of the handshake.

use crypto_hash::{Sha256, Sha384};

use crate::suite::CipherSuite;
use crate::tests::hex;
use crate::transcript::Transcript;

/// Two messages, as they would arrive.
const FIRST: &[u8] = b"\x01\x00\x00\x04ClientHello";
/// The second message.
const SECOND: &[u8] = b"\x02\x00\x00\x04ServerHello";

#[test]
fn the_transcript_is_the_hash_of_what_it_was_given() {
    let mut transcript = Transcript::new();
    transcript.update(FIRST);
    transcript.update(SECOND);

    let both = [FIRST, SECOND].concat();
    assert_eq!(
        hex(transcript.hash(CipherSuite::Aes128GcmSha256).as_bytes()),
        hex(Sha256::digest(&both).as_ref())
    );
    assert_eq!(
        hex(transcript.hash(CipherSuite::Aes256GcmSha384).as_bytes()),
        hex(Sha384::digest(&both).as_ref())
    );
}

#[test]
fn both_hashes_are_kept_so_that_the_suite_may_be_chosen_late() {
    // The client does not know the suite when it sends its first message,
    // and the transcript does not have to.
    let mut transcript = Transcript::default();
    transcript.update(FIRST);
    let short = transcript.hash(CipherSuite::Aes128GcmSha256);
    let long = transcript.hash(CipherSuite::Aes256GcmSha384);
    assert_eq!(short.len(), 32);
    assert_eq!(long.len(), 48);
    assert_eq!(
        hex(transcript
            .hash(CipherSuite::ChaCha20Poly1305Sha256)
            .as_bytes()),
        hex(short.as_bytes()),
        "the third suite hashes as the first"
    );
}

#[test]
fn the_pieces_of_a_message_hash_as_the_whole_of_it() {
    let mut whole = Transcript::new();
    whole.update(FIRST);

    let mut pieces = Transcript::new();
    for split in 0..FIRST.len() {
        let mut attempt = Transcript::new();
        let (head, tail) = FIRST.split_at(split);
        attempt.update(head);
        attempt.update(tail);
        assert_eq!(
            hex(attempt.hash(CipherSuite::Aes128GcmSha256).as_bytes()),
            hex(whole.hash(CipherSuite::Aes128GcmSha256).as_bytes()),
            "split at {split}"
        );
    }
    pieces.update(FIRST);
    assert_eq!(
        hex(pieces.hash(CipherSuite::Aes256GcmSha384).as_bytes()),
        hex(whole.hash(CipherSuite::Aes256GcmSha384).as_bytes())
    );
}

#[test]
fn a_retry_replaces_what_was_seen_with_the_hash_of_it() {
    let mut transcript = Transcript::new();
    transcript.update(FIRST);
    let before_short = transcript.hash(CipherSuite::Aes128GcmSha256);
    let before_long = transcript.hash(CipherSuite::Aes256GcmSha384);

    transcript.replace_with_message_hash();
    transcript.update(SECOND);

    // The substitution is the message type 254, a three-byte length, and
    // the hash of everything so far; each hash gets its own length.
    let mut expected_short = Sha256::new();
    expected_short.update(&[254, 0x00, 0x00, 32]);
    expected_short.update(before_short.as_bytes());
    expected_short.update(SECOND);
    assert_eq!(
        hex(transcript.hash(CipherSuite::Aes128GcmSha256).as_bytes()),
        hex(expected_short.finish().as_ref())
    );

    let mut expected_long = Sha384::new();
    expected_long.update(&[254, 0x00, 0x00, 48]);
    expected_long.update(before_long.as_bytes());
    expected_long.update(SECOND);
    assert_eq!(
        hex(transcript.hash(CipherSuite::Aes256GcmSha384).as_bytes()),
        hex(expected_long.finish().as_ref()),
        "the second hash stays usable"
    );
}

#[test]
fn the_checked_hash_agrees_with_the_length_the_suite_names() {
    let mut transcript = Transcript::new();
    transcript.update(FIRST);
    for suite in [
        CipherSuite::Aes128GcmSha256,
        CipherSuite::Aes256GcmSha384,
        CipherSuite::ChaCha20Poly1305Sha256,
    ] {
        let hash = transcript
            .checked_hash(suite)
            .expect("the lengths agree by construction");
        assert_eq!(hash.len(), suite.hash_len());
    }
}

#[test]
fn a_suite_is_the_code_point_it_is_named_by() {
    for suite in [
        CipherSuite::Aes128GcmSha256,
        CipherSuite::Aes256GcmSha384,
        CipherSuite::ChaCha20Poly1305Sha256,
    ] {
        assert_eq!(CipherSuite::from_code(suite.code()), Some(suite));
    }
    assert_eq!(CipherSuite::from_code(0x1304), None);
    assert_eq!(CipherSuite::from_code(0x009C), None, "a suite of TLS 1.2");
}
