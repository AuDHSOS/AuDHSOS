// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The six keys of RFC 4253, section 7.2, against the same computation
//! written a second time.

use crypto_hash::Sha256;

use crate::keys::{Key, derive};
use crate::tests::ssh_mpint;

/// The six letters, in the order the document lists them.
const KEYS: [(Key, u8); 6] = [
    (Key::IvClientToServer, b'A'),
    (Key::IvServerToClient, b'B'),
    (Key::EncryptionClientToServer, b'C'),
    (Key::EncryptionServerToClient, b'D'),
    (Key::IntegrityClientToServer, b'E'),
    (Key::IntegrityServerToClient, b'F'),
];

/// `HASH(K || H || X || session_id)`, built here rather than taken from
/// the code under test.
fn first(shared: &[u8], exchange: &[u8; 32], session: &[u8; 32], letter: u8) -> [u8; 32] {
    let mut preimage = ssh_mpint(shared);
    preimage.extend_from_slice(exchange);
    preimage.push(letter);
    preimage.extend_from_slice(session);
    Sha256::digest(&preimage)
}

/// `HASH(K || H || <key so far>)`, the extension of section 7.2.
fn extend(shared: &[u8], exchange: &[u8; 32], so_far: &[u8]) -> [u8; 32] {
    let mut preimage = ssh_mpint(shared);
    preimage.extend_from_slice(exchange);
    preimage.extend_from_slice(so_far);
    Sha256::digest(&preimage)
}

#[test]
fn one_hash_is_the_whole_key_when_one_hash_is_enough() {
    let shared = [0x11u8; 32];
    let exchange = [0x22u8; 32];
    let session = [0x33u8; 32];
    for (key, letter) in KEYS {
        let mut out = [0u8; 32];
        derive(&shared, &exchange, &session, key, &mut out);
        assert_eq!(out, first(&shared, &exchange, &session, letter));
        assert_eq!(key.letter(), letter);
    }
}

#[test]
fn a_key_shorter_than_the_hash_is_its_first_bytes() {
    let shared = [0x11u8; 32];
    let exchange = [0x22u8; 32];
    let session = [0x33u8; 32];
    let whole = first(&shared, &exchange, &session, b'C');
    for len in [1usize, 16, 31] {
        let mut out = vec![0u8; len];
        derive(
            &shared,
            &exchange,
            &session,
            Key::EncryptionClientToServer,
            &mut out,
        );
        assert_eq!(out.as_slice(), whole.get(..len).unwrap_or(&[]));
    }
}

#[test]
fn a_key_longer_than_the_hash_feeds_the_whole_key_back() {
    // The 64 bytes `chacha20-poly1305@openssh.com` needs are two hashes,
    // and the second is over K, H and the first.
    let shared = [0x11u8; 32];
    let exchange = [0x22u8; 32];
    let session = [0x33u8; 32];
    let mut out = [0u8; 64];
    derive(
        &shared,
        &exchange,
        &session,
        Key::EncryptionClientToServer,
        &mut out,
    );

    let k1 = first(&shared, &exchange, &session, b'C');
    let k2 = extend(&shared, &exchange, &k1);
    let mut expected = [0u8; 64];
    expected
        .get_mut(..32)
        .unwrap_or(&mut [])
        .copy_from_slice(&k1);
    expected
        .get_mut(32..)
        .unwrap_or(&mut [])
        .copy_from_slice(&k2);
    assert_eq!(out, expected);

    // And a length between the two hashes takes the first bytes of the
    // second, not the whole of it.
    let mut short = [0u8; 48];
    derive(
        &shared,
        &exchange,
        &session,
        Key::EncryptionClientToServer,
        &mut short,
    );
    assert_eq!(short.get(..32), expected.get(..32));
    assert_eq!(short.get(32..48), expected.get(32..48));
}

#[test]
fn the_six_keys_of_one_exchange_are_six_different_keys() {
    let shared = [0x11u8; 32];
    let exchange = [0x22u8; 32];
    let session = [0x33u8; 32];
    let mut seen: Vec<[u8; 32]> = Vec::new();
    for (key, _) in KEYS {
        let mut out = [0u8; 32];
        derive(&shared, &exchange, &session, key, &mut out);
        assert!(!seen.contains(&out));
        seen.push(out);
    }
    assert_eq!(seen.len(), 6);
}

#[test]
fn the_shared_secret_is_hashed_as_an_mpint_and_not_as_it_stands() {
    // The trap of 14.15, in the place it is felt second: a value whose
    // top bit is set is hashed with a zero byte in front, one whose top
    // bit is clear without, and leading zeros are not hashed at all.
    let exchange = [0x22u8; 32];
    let session = [0x33u8; 32];
    for shared in [[0x80u8; 32], [0x7fu8; 32]] {
        let mut out = [0u8; 32];
        derive(
            &shared,
            &exchange,
            &session,
            Key::IvClientToServer,
            &mut out,
        );
        assert_eq!(out, first(&shared, &exchange, &session, b'A'));
    }

    let mut leading = [0x11u8; 32];
    if let Some(byte) = leading.first_mut() {
        *byte = 0;
    }
    let mut out = [0u8; 32];
    derive(
        &leading,
        &exchange,
        &session,
        Key::IvClientToServer,
        &mut out,
    );
    assert_eq!(out, first(&leading, &exchange, &session, b'A'));
    assert_eq!(
        out,
        first(leading.get(1..).unwrap_or(&[]), &exchange, &session, b'A'),
        "a leading zero is not part of the value"
    );
}
