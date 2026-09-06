// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! PKCS #1 v1.5, in both directions.
//!
//! The negative tests are the point. Each of them is an encoded message
//! that a verifier which *decodes* would accept, signed with the private
//! exponent of the key of RFC 8448, section 2 so that the arithmetic
//! recovers it exactly. A construction has no slack to accept, so all of
//! them are refused — and a suite that only checked that valid signatures
//! verify would not notice if this crate stopped refusing them.

use crypto_bignum::Modulus;
use test_support::generators::bytes;
use test_support::property::{Config, check_with};

use crate::error::RsaError;
use crate::hash::HashId;
use crate::key::PublicKey;
use crate::pkcs1::{encode, prefix_of};
use crate::signing::{sign_encoded, sign_pkcs1};
use crate::tests::bigint::Big;
use crate::tests::{RFC8448_PRIVATE, fixed_modulus, hex, rfc8448_key, unhex};

/// The message every signature in this file is over.
const MESSAGE: &[u8] = b"the naming of cats is a difficult matter";

/// The three hashes, with the length of the `DigestInfo` each produces.
const HASHES: [(HashId, usize); 3] = [
    (HashId::Sha256, 51),
    (HashId::Sha384, 67),
    (HashId::Sha512, 83),
];

/// The encoded message this crate builds for `hash` under a key of `size`
/// bytes.
fn encoded(size: usize, hash: HashId) -> Vec<u8> {
    let mut out = vec![0u8; size];
    encode(hash, MESSAGE, &mut out).expect("the key is wide enough for the encoding");
    out
}

/// A signature over an encoded message of the caller's making.
fn signed(key: &PublicKey, block: &[u8]) -> Vec<u8> {
    let mut signature = vec![0u8; key.size()];
    sign_encoded(key, &unhex(RFC8448_PRIVATE), block, &mut signature)
        .expect("the block is the width of the key and below the modulus");
    signature
}

#[test]
fn a_signature_this_crate_made_verifies_under_each_hash() {
    let key = rfc8448_key();
    for (hash, _) in HASHES {
        let mut signature = vec![0u8; key.size()];
        sign_pkcs1(&key, &unhex(RFC8448_PRIVATE), hash, MESSAGE, &mut signature)
            .expect("the key is wide enough for all three encodings");
        assert_eq!(key.verify_pkcs1(hash, MESSAGE, &signature), Ok(()));
        assert_eq!(
            key.verify_pkcs1(hash, b"another message", &signature),
            Err(RsaError::BadSignature)
        );
    }
}

/// The encoding against RFC 8017, section 9.2, note 1, byte for byte.
#[test]
fn the_encoded_message_carries_the_documented_digest_info() {
    for (hash, t_len) in HASHES {
        let block = encoded(128, hash);
        assert_eq!(block.len(), 128);
        let prefix = prefix_of(hash);
        assert_eq!(prefix.len().saturating_add(hash.output_len()), t_len);

        let separator = 128 - t_len - 1;
        assert_eq!(block.first(), Some(&0x00));
        assert_eq!(block.get(1), Some(&0x01));
        assert!(
            block
                .get(2..separator)
                .is_some_and(|ps| { ps.len() >= 8 && ps.iter().all(|byte| *byte == 0xff) })
        );
        assert_eq!(block.get(separator), Some(&0x00));
        assert_eq!(
            block.get(separator + 1..separator + 1 + prefix.len()),
            Some(prefix)
        );
        let digest = hash.digest(MESSAGE);
        assert_eq!(
            block.get(separator + 1 + prefix.len()..),
            digest.get(..hash.output_len())
        );
    }
}

#[test]
fn a_padding_shorter_than_eight_bytes_is_refused() {
    let key = rfc8448_key();
    let mut block = vec![0x00u8, 0x01];
    block.extend_from_slice(&[0xff; 7]);
    block.push(0x00);
    block.extend_from_slice(prefix_of(HashId::Sha256));
    block.extend_from_slice(&HashId::Sha256.digest(MESSAGE)[..32]);
    block.resize(key.size(), 0x00);
    let signature = signed(&key, &block);
    assert_eq!(
        key.verify_pkcs1(HashId::Sha256, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

#[test]
fn a_digest_info_moved_inside_the_block_is_refused() {
    let key = rfc8448_key();
    let mut block = encoded(key.size(), HashId::Sha256);
    // Two bytes earlier, with the padding shortened to match and the two
    // bytes it gave up written after the digest as more padding.
    block.copy_within(75.., 73);
    for slot in block.iter_mut().skip(126) {
        *slot = 0xff;
    }
    assert_eq!(block.get(74), Some(&0x00));
    let signature = signed(&key, &block);
    assert_eq!(
        key.verify_pkcs1(HashId::Sha256, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

#[test]
fn bytes_after_the_digest_are_refused() {
    let key = rfc8448_key();
    let mut block = encoded(key.size(), HashId::Sha256);
    block.copy_within(74..125, 71);
    for slot in block.iter_mut().skip(122) {
        *slot = 0xa5;
    }
    let signature = signed(&key, &block);
    assert_eq!(
        key.verify_pkcs1(HashId::Sha256, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

#[test]
fn a_missing_separator_is_refused() {
    let key = rfc8448_key();
    let mut block = encoded(key.size(), HashId::Sha256);
    let separator = key.size() - 51 - 1;
    if let Some(slot) = block.get_mut(separator) {
        *slot = 0xff;
    }
    let signature = signed(&key, &block);
    assert_eq!(
        key.verify_pkcs1(HashId::Sha256, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

#[test]
fn a_first_byte_that_is_not_zero_is_refused() {
    let key = rfc8448_key();
    let mut block = encoded(key.size(), HashId::Sha256);
    if let Some(slot) = block.first_mut() {
        *slot = 0x01;
    }
    let signature = signed(&key, &block);
    assert_eq!(
        key.verify_pkcs1(HashId::Sha256, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

#[test]
fn a_second_byte_that_is_not_one_is_refused() {
    let key = rfc8448_key();
    let mut block = encoded(key.size(), HashId::Sha256);
    if let Some(slot) = block.get_mut(1) {
        *slot = 0x02;
    }
    let signature = signed(&key, &block);
    assert_eq!(
        key.verify_pkcs1(HashId::Sha256, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

/// The one incompatibility this crate states rather than discovers: a
/// `DigestInfo` whose `SEQUENCE` carries an indefinite length is BER and
/// not DER, PKCS #1 v1.5 accepts it (RFC 2313, section 10.2.3), and D-80
/// refuses it.
#[test]
fn a_digest_info_with_an_indefinite_length_is_refused() {
    let key = rfc8448_key();
    let digest = HashId::Sha256.digest(MESSAGE);
    // `30 80 ... 00 00` in place of `30 31 ...`: two bytes longer, so the
    // padding gives up two.
    let mut info = vec![0x30u8, 0x80];
    info.extend_from_slice(prefix_of(HashId::Sha256).get(2..).unwrap_or_default());
    info.extend_from_slice(&digest[..32]);
    info.extend_from_slice(&[0x00, 0x00]);
    assert_eq!(info.len(), 53);

    let mut block = vec![0x00u8, 0x01];
    block.resize(key.size() - info.len() - 1, 0xff);
    block.push(0x00);
    block.extend_from_slice(&info);
    assert_eq!(block.len(), key.size());

    let signature = signed(&key, &block);
    assert_eq!(
        key.verify_pkcs1(HashId::Sha256, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

#[test]
fn a_signature_of_the_wrong_length_is_refused() {
    let key = rfc8448_key();
    let mut signature = vec![0u8; key.size()];
    sign_pkcs1(
        &key,
        &unhex(RFC8448_PRIVATE),
        HashId::Sha256,
        MESSAGE,
        &mut signature,
    )
    .expect("the key is wide enough");
    let mut short = signature.clone();
    short.pop();
    assert_eq!(
        key.verify_pkcs1(HashId::Sha256, MESSAGE, &short),
        Err(RsaError::BadSignature)
    );
    let mut long = signature;
    long.push(0);
    assert_eq!(
        key.verify_pkcs1(HashId::Sha256, MESSAGE, &long),
        Err(RsaError::BadSignature)
    );
}

#[test]
fn a_signature_that_is_not_below_the_modulus_is_refused() {
    let key = rfc8448_key();
    let modulus = unhex(crate::tests::RFC8448_MODULUS);
    assert_eq!(
        key.verify_pkcs1(HashId::Sha256, MESSAGE, &modulus),
        Err(RsaError::BadSignature)
    );
}

#[test]
fn a_key_too_small_for_the_encoding_is_refused() {
    // Ninety-three bytes leaves `tLen + 11` at ninety-four for SHA-512.
    let key = PublicKey::new(&fixed_modulus(744), &unhex("010001"))
        .expect("the fixed modulus is well formed");
    let mut out = vec![0u8; key.size()];
    assert_eq!(
        encode(HashId::Sha512, MESSAGE, &mut out),
        Err(RsaError::BadSignature)
    );
    assert_eq!(
        key.verify_pkcs1(HashId::Sha512, MESSAGE, &vec![0u8; key.size()]),
        Err(RsaError::BadSignature)
    );
}

/// The forgery of Bleichenbacher's 2006 note, which is what the
/// construction of D-80 exists to refuse. Nothing signs it: the signature
/// is the integer cube root of a block whose high bytes are a short
/// padding and a well-formed `DigestInfo`, and whose low bytes are
/// whatever the cube happens to be. A verifier that decodes reads the
/// digest out of it and agrees; this one builds the block it expects and
/// does not.
#[test]
fn a_forgery_against_an_exponent_of_three_is_refused() {
    let modulus = fixed_modulus(3072);
    let key = PublicKey::new(&modulus, &[3]).expect("the fixed modulus is well formed");
    let size = key.size();

    let digest = HashId::Sha256.digest(MESSAGE);
    let mut target = vec![0x00u8, 0x01, 0xff, 0x00];
    target.extend_from_slice(prefix_of(HashId::Sha256));
    target.extend_from_slice(&digest[..32]);
    let prefix_len = target.len();
    target.resize(size, 0xff);

    let root = Big::from_be_bytes(&target).cube_root_floor(1024 + 8);
    let signature = root.to_be_bytes(size);
    let cube = root.mul(&root).mul(&root).to_be_bytes(size);

    // The cube really does carry the block a decoder would read, which is
    // what makes this a forgery rather than a random signature.
    let recovered = raise(&modulus, &signature, size);
    assert_eq!(hex(&recovered), hex(&cube));
    assert_eq!(
        recovered.get(..prefix_len).map(hex),
        target.get(..prefix_len).map(hex)
    );
    assert_ne!(recovered.get(prefix_len..), target.get(prefix_len..));

    assert_eq!(
        key.verify_pkcs1(HashId::Sha256, MESSAGE, &signature),
        Err(RsaError::BadSignature)
    );
}

/// The signature cubed modulo the modulus, which is what verification
/// starts from.
fn raise(modulus: &[u8], signature: &[u8], size: usize) -> Vec<u8> {
    let value = Modulus::new(modulus).expect("the fixed modulus is well formed");
    let mut out = vec![0u8; size];
    value
        .pow(signature, 3, &mut out)
        .expect("the signature is below the modulus");
    out
}

/// Over generated messages: the signature this crate makes verifies, and
/// a signature with one byte changed does not. The exponentiation is the
/// expensive part, so the count is small and the shapes above carry the
/// rest of the weight.
#[test]
fn property_a_signature_verifies_and_a_changed_one_does_not() {
    let key = rfc8448_key();
    let private = unhex(RFC8448_PRIVATE);
    let config = Config {
        cases: 16,
        ..Config::default()
    };
    let generator = bytes(0..=64);
    if let Err(failure) = check_with(&config, "rsa_pkcs1_round_trip", &generator, |message| {
        let mut signature = vec![0u8; key.size()];
        sign_pkcs1(&key, &private, HashId::Sha256, message, &mut signature)
            .map_err(|error| error.to_string())?;
        if key.verify_pkcs1(HashId::Sha256, message, &signature) != Ok(()) {
            return Err("the signature does not verify".to_owned());
        }
        for index in [0usize, 64, 127] {
            let mut changed = signature.clone();
            if let Some(slot) = changed.get_mut(index) {
                *slot ^= 0x01;
            }
            if key.verify_pkcs1(HashId::Sha256, message, &changed) == Ok(()) {
                return Err(format!("a change at byte {index} still verifies"));
            }
        }
        Ok(())
    }) {
        panic!("{failure}");
    }
}
