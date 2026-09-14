// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Host key blobs, the signature over the exchange hash, and the rule
//! that admits a key.
//!
//! RFC 8709 publishes no vector, so the blobs are built a second time
//! here from the bytes the document prints in words, and the signatures
//! come from `crypto-ec`, whose own tests hold it to RFC 8032.

use crypto_ec::ed25519;
use crypto_hash::Sha256;

use crate::error::SshError;
use crate::hostkey::{
    BLOB_LEN, Fingerprint, Fingerprints, HostKey, SIGNATURE_BLOB_LEN, Trust, accept,
};
use crate::kex::HOST_KEY_ED25519;
use crate::tests::ssh_string;

/// The server's long-term secret, and a second one for the host this
/// client is not talking to.
const SERVER_SECRET: [u8; 32] = [0x33; 32];
/// Another host's.
const OTHER_SECRET: [u8; 32] = [0x44; 32];

/// An exchange hash to sign. Its value does not matter; that both sides
/// use the same one does.
const HASH: [u8; 32] = [0x55; 32];

/// The blob of RFC 8709, section 4, written by the test.
fn key_blob(name: &str, key: &[u8]) -> Vec<u8> {
    let mut blob = ssh_string(name.as_bytes());
    blob.extend_from_slice(&ssh_string(key));
    blob
}

/// The public key of `secret` as a blob.
fn server_blob(secret: &[u8; 32]) -> Vec<u8> {
    key_blob(HOST_KEY_ED25519, &ed25519::public_key(secret))
}

/// The signature blob of section 6 over `hash`.
fn signature_blob(secret: &[u8; 32], hash: &[u8; 32]) -> Vec<u8> {
    key_blob(HOST_KEY_ED25519, &ed25519::sign(secret, hash))
}

/// A rule that admits the key of `secret`.
fn rule_for(secret: &[u8; 32]) -> Fingerprint {
    Fingerprint::new(Sha256::digest(&server_blob(secret)))
}

#[test]
fn a_key_blob_is_a_name_and_thirty_two_octets() {
    let blob = server_blob(&SERVER_SECRET);
    assert_eq!(blob.len(), BLOB_LEN);

    let key = HostKey::parse(&blob).expect("the blob is the one of section 4");

    assert_eq!(key.algorithm(), HOST_KEY_ED25519);
    assert_eq!(key.public_key(), &ed25519::public_key(&SERVER_SECRET));
}

#[test]
fn a_parsed_key_writes_back_the_blob_it_was_read_from() {
    let blob = server_blob(&SERVER_SECRET);
    let key = HostKey::parse(&blob).expect("the blob is a key");

    let mut out = [0u8; BLOB_LEN];
    let written = key.write(&mut out).expect("the buffer is long enough");

    assert_eq!(written, blob.len());
    assert_eq!(out.get(..written), Some(blob.as_slice()));
}

#[test]
fn a_buffer_short_by_one_byte_takes_no_blob() {
    let key = HostKey::parse(&server_blob(&SERVER_SECRET)).expect("the blob is a key");

    let mut out = [0u8; BLOB_LEN - 1];

    assert!(matches!(
        key.write(&mut out),
        Err(SshError::OutOfBounds { .. })
    ));
}

#[test]
fn a_blob_of_another_algorithm_is_no_key_of_this_client() {
    let rsa = key_blob("ssh-rsa", &[0x01; 32]);
    let ed448 = key_blob("ssh-ed448", &[0x02; 57]);

    assert_eq!(HostKey::parse(&rsa), Err(SshError::HostKey));
    assert_eq!(HostKey::parse(&ed448), Err(SshError::HostKey));
}

#[test]
fn a_key_of_another_length_is_refused() {
    let short = key_blob(HOST_KEY_ED25519, &[0x01; 31]);
    let long = key_blob(HOST_KEY_ED25519, &[0x01; 33]);

    assert_eq!(HostKey::parse(&short), Err(SshError::HostKey));
    assert_eq!(HostKey::parse(&long), Err(SshError::HostKey));
}

#[test]
fn bytes_after_the_key_are_refused() {
    let mut blob = server_blob(&SERVER_SECRET);
    blob.push(0x00);

    assert_eq!(HostKey::parse(&blob), Err(SshError::HostKey));
}

#[test]
fn a_blob_that_ends_early_is_out_of_bounds() {
    let blob = server_blob(&SERVER_SECRET);

    for len in 0..blob.len() {
        let partial = blob.get(..len).unwrap_or_default();
        assert!(
            matches!(
                HostKey::parse(partial),
                Err(SshError::OutOfBounds { .. } | SshError::HostKey)
            ),
            "{len} bytes read as a whole blob"
        );
    }
}

#[test]
fn the_signature_of_section_6_verifies_over_the_exchange_hash() {
    let key = HostKey::parse(&server_blob(&SERVER_SECRET)).expect("the blob is a key");
    let signature = signature_blob(&SERVER_SECRET, &HASH);
    assert_eq!(signature.len(), SIGNATURE_BLOB_LEN);

    assert_eq!(key.verify(&HASH, &signature), Ok(()));
}

#[test]
fn a_signature_from_another_host_does_not_verify() {
    let key = HostKey::parse(&server_blob(&SERVER_SECRET)).expect("the blob is a key");
    let signature = signature_blob(&OTHER_SECRET, &HASH);

    assert_eq!(key.verify(&HASH, &signature), Err(SshError::Signature));
}

#[test]
fn one_bit_of_the_hash_decides_the_signature() {
    let key = HostKey::parse(&server_blob(&SERVER_SECRET)).expect("the blob is a key");
    let signature = signature_blob(&SERVER_SECRET, &HASH);

    let mut other = HASH;
    other[0] ^= 0x01;

    assert_eq!(key.verify(&other, &signature), Err(SshError::Signature));
}

#[test]
fn a_signature_blob_this_client_does_not_read_is_refused() {
    let key = HostKey::parse(&server_blob(&SERVER_SECRET)).expect("the blob is a key");
    let named_otherwise = key_blob("rsa-sha2-256", &ed25519::sign(&SERVER_SECRET, &HASH));
    let too_short = key_blob(HOST_KEY_ED25519, &[0x01; 63]);
    let mut trailing = signature_blob(&SERVER_SECRET, &HASH);
    trailing.push(0x00);

    assert_eq!(key.verify(&HASH, &named_otherwise), Err(SshError::HostKey));
    assert_eq!(key.verify(&HASH, &too_short), Err(SshError::HostKey));
    assert_eq!(key.verify(&HASH, &trailing), Err(SshError::HostKey));
    assert!(matches!(
        key.verify(&HASH, &[]),
        Err(SshError::OutOfBounds { .. })
    ));
}

#[test]
fn the_fingerprint_is_the_digest_of_the_blob() {
    let blob = server_blob(&SERVER_SECRET);
    let key = HostKey::parse(&blob).expect("the blob is a key");

    assert_eq!(key.fingerprint(), Sha256::digest(&blob));
}

#[test]
fn a_fingerprint_admits_one_host_and_no_other() {
    let rule = rule_for(&SERVER_SECRET);
    let server = HostKey::parse(&server_blob(&SERVER_SECRET)).expect("the blob is a key");
    let other = HostKey::parse(&server_blob(&OTHER_SECRET)).expect("the blob is a key");

    assert_eq!(rule.digest(), &server.fingerprint());
    assert!(rule.accepts(&server));
    assert!(!rule.accepts(&other));
}

#[test]
fn a_key_and_a_signature_together_are_what_a_client_accepts() {
    let blob = server_blob(&SERVER_SECRET);
    let signature = signature_blob(&SERVER_SECRET, &HASH);

    let key = accept(&blob, &HASH, &signature, &rule_for(&SERVER_SECRET))
        .expect("the rule admits the key and the signature is the peer's");

    assert_eq!(key.public_key(), &ed25519::public_key(&SERVER_SECRET));
}

#[test]
fn a_key_no_rule_admits_is_refused_before_its_signature_is_checked() {
    let blob = server_blob(&OTHER_SECRET);
    let signature = signature_blob(&SERVER_SECRET, &HASH);

    assert_eq!(
        accept(&blob, &HASH, &signature, &rule_for(&SERVER_SECRET)),
        Err(SshError::HostKeyRejected)
    );
}

#[test]
fn a_signature_that_is_not_the_peers_ends_the_exchange() {
    let blob = server_blob(&SERVER_SECRET);
    let signature = signature_blob(&OTHER_SECRET, &HASH);

    assert_eq!(
        accept(&blob, &HASH, &signature, &rule_for(&SERVER_SECRET)),
        Err(SshError::Signature)
    );
}

#[test]
fn a_blob_that_is_no_key_ends_it_before_the_rule_is_asked() {
    let rule = rule_for(&SERVER_SECRET);
    let signature = signature_blob(&SERVER_SECRET, &HASH);

    assert_eq!(
        accept(&key_blob("ssh-dss", &[0x01; 32]), &HASH, &signature, &rule),
        Err(SshError::HostKey)
    );
}

/// The fingerprint of the host key `secret` stands for.
fn digest_for(secret: &[u8; 32]) -> [u8; 32] {
    Sha256::digest(&server_blob(secret))
}

#[test]
fn a_set_of_fingerprints_admits_every_host_it_names() {
    let held = [digest_for(&OTHER_SECRET), digest_for(&SERVER_SECRET)];
    let rule = Fingerprints::new(&held);
    let blob = server_blob(&SERVER_SECRET);
    let signature = signature_blob(&SERVER_SECRET, &HASH);

    assert_eq!(rule.len(), 2);
    assert!(!rule.is_empty());
    assert!(accept(&blob, &HASH, &signature, &rule).is_ok());
}

#[test]
fn a_set_of_fingerprints_refuses_a_host_it_does_not_name() {
    let held = [digest_for(&OTHER_SECRET)];
    let rule = Fingerprints::new(&held);
    let blob = server_blob(&SERVER_SECRET);
    let signature = signature_blob(&SERVER_SECRET, &HASH);

    assert_eq!(
        accept(&blob, &HASH, &signature, &rule),
        Err(SshError::HostKeyRejected)
    );
}

#[test]
fn an_empty_set_of_fingerprints_admits_no_host() {
    let rule = Fingerprints::new(&[]);
    let blob = server_blob(&SERVER_SECRET);
    let signature = signature_blob(&SERVER_SECRET, &HASH);

    assert!(rule.is_empty());
    assert_eq!(
        accept(&blob, &HASH, &signature, &rule),
        Err(SshError::HostKeyRejected)
    );
}
