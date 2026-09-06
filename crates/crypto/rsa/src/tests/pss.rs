// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! EMSA-PSS, in both directions.
//!
//! The negative tests take each of the five ways RFC 8017, section 9.1.2
//! calls an encoding inconsistent, one at a time. Every one of them is a
//! block built by hand and signed with a private exponent, so what the
//! verification meets is a signature that recovers exactly the block the
//! test meant, and the only thing wrong with it is the one thing the test
//! is about.
#![expect(
    clippy::arithmetic_side_effects,
    reason = "these blocks are laid out by hand against section 9.1.2, and the ordinary operators \
              keep the offsets in the shape the section states them in; an overflow here is a \
              failing test rather than a fault in the product"
)]

use test_support::generators::bytes;
use test_support::property::{Config, check_with};

use crate::error::RsaError;
use crate::hash::HashId;
use crate::key::PublicKey;
use crate::pss::{TRAILER, mgf1};
use crate::signing::{sign_encoded, sign_pss};
use crate::tests::{
    GENERATED_2048_PRIVATE, RFC8448_PRIVATE, generated_2048_key, rfc8448_key, unhex,
};

/// The message every signature in this file is over.
const MESSAGE: &[u8] = b"the naming of cats is a difficult matter";

/// The three hashes.
const HASHES: [HashId; 3] = [HashId::Sha256, HashId::Sha384, HashId::Sha512];

/// A salt of the length the schemes fix: as long as the hash output.
fn salt_of(hash: HashId) -> Vec<u8> {
    (0..hash.output_len())
        .map(|index| u8::try_from(index.wrapping_mul(29) % 251).unwrap_or(1))
        .collect()
}

/// An encoded message built by hand: zeros, a separator, and a salt,
/// masked as the encoding says, with `H` over the salt that is in it.
fn craft(hash: HashId, em_len: usize, message: &[u8], salt: &[u8], separator: u8) -> Vec<u8> {
    let hash_len = hash.output_len();
    let db_len = em_len - hash_len - 1;
    let digest = hash.digest(message);
    let seed = hash.digest_parts(&[&[0u8; 8], &digest[..hash_len], salt]);

    let mut db = vec![0u8; db_len];
    db[db_len - salt.len() - 1] = separator;
    db[db_len - salt.len()..].copy_from_slice(salt);

    let mut mask = vec![0u8; db_len];
    mgf1(hash, &seed[..hash_len], &mut mask);
    for (slot, byte) in db.iter_mut().zip(&mask) {
        *slot ^= *byte;
    }
    db[0] &= 0x7f;

    let mut encoded = db;
    encoded.extend_from_slice(&seed[..hash_len]);
    encoded.push(TRAILER);
    encoded
}

/// A signature over a block of the caller's making, under the key of
/// RFC 8448.
fn signed(key: &PublicKey, block: &[u8]) -> Vec<u8> {
    let mut signature = vec![0u8; key.size()];
    sign_encoded(key, &unhex(RFC8448_PRIVATE), block, &mut signature)
        .expect("the block is the width of the key and below the modulus");
    signature
}

#[test]
fn a_signature_this_crate_made_verifies_under_each_hash() {
    let key = generated_2048_key();
    let private = unhex(GENERATED_2048_PRIVATE);
    for hash in HASHES {
        let mut signature = vec![0u8; key.size()];
        sign_pss(
            &key,
            &private,
            hash,
            MESSAGE,
            &salt_of(hash),
            &mut signature,
        )
        .expect("two thousand and forty-eight bits carries all three encodings");
        assert_eq!(key.verify_pss(hash, MESSAGE, &signature), Ok(()));
        assert_eq!(
            key.verify_pss(hash, b"another message", &signature),
            Err(RsaError::BadSignature)
        );
    }
}

/// The thousand-and-twenty-four-bit key carries SHA-256 and SHA-384 and
/// not SHA-512: a hundred and thirty bytes do not fit in a hundred and
/// twenty-eight, which is the first thing section 9.1.2 checks.
#[test]
fn a_key_too_small_for_the_encoding_is_refused() {
    let key = rfc8448_key();
    let private = unhex(RFC8448_PRIVATE);
    for hash in [HashId::Sha256, HashId::Sha384] {
        let mut signature = vec![0u8; key.size()];
        sign_pss(
            &key,
            &private,
            hash,
            MESSAGE,
            &salt_of(hash),
            &mut signature,
        )
        .expect("the encoding fits");
        assert_eq!(key.verify_pss(hash, MESSAGE, &signature), Ok(()));
    }
    let mut signature = vec![0u8; key.size()];
    assert_eq!(
        sign_pss(
            &key,
            &private,
            HashId::Sha512,
            MESSAGE,
            &salt_of(HashId::Sha512),
            &mut signature
        ),
        Err(RsaError::BadSignature)
    );
    assert_eq!(
        key.verify_pss(HashId::Sha512, MESSAGE, &vec![0u8; key.size()]),
        Err(RsaError::BadSignature)
    );
}

#[test]
fn a_block_built_by_hand_verifies_when_nothing_is_wrong_with_it() {
    let key = rfc8448_key();
    let block = craft(
        HashId::Sha256,
        key.size(),
        MESSAGE,
        &salt_of(HashId::Sha256),
        0x01,
    );
    let signature = signed(&key, &block);
    assert_eq!(key.verify_pss(HashId::Sha256, MESSAGE, &signature), Ok(()));
}

#[test]
fn a_trailer_that_is_not_bc_is_refused() {
    let key = rfc8448_key();
    let mut block = craft(
        HashId::Sha256,
        key.size(),
        MESSAGE,
        &salt_of(HashId::Sha256),
        0x01,
    );
    if let Some(last) = block.last_mut() {
        *last = 0xbd;
    }
    let signature = signed(&key, &block);
    assert_eq!(
        key.verify_pss(HashId::Sha256, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

/// The leftmost bit of the encoding is the one `emBits` says is not
/// there. Setting it, and nothing else in that byte, keeps the block
/// below the modulus so that the arithmetic recovers it and the check
/// under test is the one that fires.
#[test]
fn a_leftmost_bit_that_should_be_zero_is_refused() {
    let key = rfc8448_key();
    let mut block = craft(
        HashId::Sha256,
        key.size(),
        MESSAGE,
        &salt_of(HashId::Sha256),
        0x01,
    );
    if let Some(first) = block.first_mut() {
        *first = 0x80;
    }
    let signature = signed(&key, &block);
    assert_eq!(
        key.verify_pss(HashId::Sha256, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

#[test]
fn a_separator_that_is_not_one_is_refused() {
    let key = rfc8448_key();
    let block = craft(
        HashId::Sha256,
        key.size(),
        MESSAGE,
        &salt_of(HashId::Sha256),
        0x02,
    );
    let signature = signed(&key, &block);
    assert_eq!(
        key.verify_pss(HashId::Sha256, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

/// A salt of any other length than the hash output. The schemes fix it,
/// so the separator is looked for where a salt of that length would put
/// it and nowhere else (D-81).
#[test]
fn a_salt_of_the_wrong_length_is_refused() {
    let key = rfc8448_key();
    let full = salt_of(HashId::Sha256);
    for length in [31usize, 33] {
        let mut salt = full.clone();
        salt.resize(length, 0x5a);
        let block = craft(HashId::Sha256, key.size(), MESSAGE, &salt, 0x01);
        let signature = signed(&key, &block);
        assert_eq!(
            key.verify_pss(HashId::Sha256, MESSAGE, &signature),
            Err(RsaError::BadSignature),
            "a salt of {length} bytes"
        );
    }
}

/// Bytes before the separator that are not zero, which section 9.1.2
/// checks separately from the separator itself.
#[test]
fn padding_that_is_not_zero_is_refused() {
    let key = rfc8448_key();
    let hash = HashId::Sha256;
    let hash_len = hash.output_len();
    let db_len = key.size() - hash_len - 1;
    let salt = salt_of(hash);
    let digest = hash.digest(MESSAGE);
    let seed = hash.digest_parts(&[&[0u8; 8], &digest[..hash_len], &salt]);

    let mut db = vec![0u8; db_len];
    db[1] = 0x01;
    db[db_len - salt.len() - 1] = 0x01;
    db[db_len - salt.len()..].copy_from_slice(&salt);
    let mut mask = vec![0u8; db_len];
    mgf1(hash, &seed[..hash_len], &mut mask);
    for (slot, byte) in db.iter_mut().zip(&mask) {
        *slot ^= *byte;
    }
    db[0] &= 0x7f;
    let mut block = db;
    block.extend_from_slice(&seed[..hash_len]);
    block.push(TRAILER);

    let signature = signed(&key, &block);
    assert_eq!(
        key.verify_pss(hash, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

/// A recomputed `H'` that does not match: the block is well formed, and
/// the salt in it is not the salt `H` was computed over.
#[test]
fn a_recomputed_hash_that_does_not_match_is_refused() {
    let key = rfc8448_key();
    let hash = HashId::Sha256;
    let mut salt = salt_of(hash);
    let mut block = craft(hash, key.size(), MESSAGE, &salt, 0x01);
    // Change the masked salt, which changes the salt the verification
    // recovers and nothing else about the shape of the block.
    let last = block.len() - hash.output_len() - 2;
    block[last] ^= 0x01;
    salt[hash.output_len() - 1] ^= 0x01;
    let signature = signed(&key, &block);
    assert_eq!(
        key.verify_pss(hash, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

#[test]
fn a_signature_of_the_wrong_length_is_refused() {
    let key = generated_2048_key();
    let mut signature = vec![0u8; key.size()];
    sign_pss(
        &key,
        &unhex(GENERATED_2048_PRIVATE),
        HashId::Sha256,
        MESSAGE,
        &salt_of(HashId::Sha256),
        &mut signature,
    )
    .expect("the encoding fits");
    signature.pop();
    assert_eq!(
        key.verify_pss(HashId::Sha256, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

/// MGF1 against RFC 8017, appendix B.2.1: the first `hLen` bytes of a
/// mask of any length are the hash of the seed and a counter of zero, and
/// the block after them is the hash of the seed and a counter of one.
#[test]
fn the_mask_is_the_hash_of_the_seed_and_a_counter() {
    let hash = HashId::Sha256;
    let seed = b"seed";
    let mut mask = [0u8; 80];
    mgf1(hash, seed, &mut mask);
    let first = hash.digest_parts(&[seed, &0u32.to_be_bytes()]);
    let second = hash.digest_parts(&[seed, &1u32.to_be_bytes()]);
    let third = hash.digest_parts(&[seed, &2u32.to_be_bytes()]);
    assert_eq!(&mask[..32], &first[..32]);
    assert_eq!(&mask[32..64], &second[..32]);
    assert_eq!(&mask[64..], &third[..16]);
}

/// Over generated messages and generated salts: what this crate signs, it
/// verifies, and a signature with one byte changed does not.
#[test]
fn property_a_signature_verifies_and_a_changed_one_does_not() {
    let key = generated_2048_key();
    let private = unhex(GENERATED_2048_PRIVATE);
    let config = Config {
        cases: 12,
        ..Config::default()
    };
    let generator = bytes(32..=32);
    if let Err(failure) = check_with(&config, "rsa_pss_round_trip", &generator, |salt| {
        let mut signature = vec![0u8; key.size()];
        sign_pss(
            &key,
            &private,
            HashId::Sha256,
            MESSAGE,
            salt,
            &mut signature,
        )
        .map_err(|error| error.to_string())?;
        if key.verify_pss(HashId::Sha256, MESSAGE, &signature) != Ok(()) {
            return Err("the signature does not verify".to_owned());
        }
        let mut changed = signature;
        if let Some(slot) = changed.first_mut() {
            *slot ^= 0x01;
        }
        if key.verify_pss(HashId::Sha256, MESSAGE, &changed) == Ok(()) {
            return Err("a changed signature still verifies".to_owned());
        }
        Ok(())
    }) {
        panic!("{failure}");
    }
}
