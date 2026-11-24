// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! `PrivateKey`: the ladder signs what the test exponentiation signs, and
//! every refusal leaves a wiped buffer.

use crate::error::RsaError;
use crate::hash::HashId;
use crate::key::PublicKey;
use crate::private::PrivateKey;
use crate::signing::sign_pkcs1;
use crate::tests::{
    GENERATED_2048_PRIVATE, RFC8448_PRIVATE, RFC8448_PUBLIC, generated_2048_key, rfc8448_key, unhex,
};

/// The message every signature in this file is over.
const MESSAGE: &[u8] = b"hello@maps.berlin";

/// A 1032-bit modulus: 129 bytes, while the ladder writes 136. Generated
/// once outside this repository (D-83) with
///
/// ```text
/// openssl genrsa 1032
/// ```
///
/// and transcribed with its private exponent. The public exponent is
/// sixty-five thousand five hundred and thirty-seven.
const GENERATED_1032_MODULUS: &str = "\
    d184d4ef08daf29382bdb8654974fa1e674941c398a2d54a25736a19af44ac60\
    a7ebd7fcd843c067bd439f682a83047866cb740a244bcd6d25a0bf7c9623e798\
    8182c0cfe541f2bd3eb97375243a2fd6bf5224de7ec2488de9d66663409bd4c7\
    c997b50c2f7a286c0801131d4620ef1e6655a6a93751de179fd4374e1671d815\
    27";

/// The private exponent of that pair.
const GENERATED_1032_PRIVATE: &str = "\
    5b57e9ed7c92d806ef8cff03498f322c67234d04955b085a1711c64404619035\
    638032de0cfb1cbb1d8c89c6c421f2a4fb4bdfff3bebc057c17bdf7fd58539c6\
    faac4f2447e5b28d4f4e3cd17603d783293b39ee456c8201da13bd5169915871\
    5498320af94e0337c29fc78cd8b744a4119df1a4e4a203df83331c0fe1b89788\
    01";

/// The 1032-bit pair's public half.
fn generated_1032_key() -> PublicKey {
    PublicKey::new(&unhex(GENERATED_1032_MODULUS), &unhex(RFC8448_PUBLIC))
        .expect("the generated key is odd, normalized, and has an odd exponent")
}

/// The three keys of this crate's tests, each with its private exponent.
fn keys() -> [(PublicKey, Vec<u8>); 3] {
    [
        (rfc8448_key(), unhex(RFC8448_PRIVATE)),
        (generated_1032_key(), unhex(GENERATED_1032_PRIVATE)),
        (generated_2048_key(), unhex(GENERATED_2048_PRIVATE)),
    ]
}

#[test]
fn the_ladder_signs_what_the_test_exponentiation_signs() {
    for (public, exponent) in keys() {
        for hash in [HashId::Sha256, HashId::Sha384] {
            let mut expected = vec![0u8; public.size()];
            sign_pkcs1(&public, &exponent, hash, MESSAGE, &mut expected)
                .expect("the test signing accepts the key");
            let key = PrivateKey::new(public.clone(), &exponent).expect("the exponent fits");
            let mut signature = vec![0u8; public.size()];
            assert_eq!(key.sign_pkcs1(hash, MESSAGE, &mut signature), Ok(()));
            assert_eq!(signature, expected);
            assert_eq!(public.verify_pkcs1(hash, MESSAGE, &signature), Ok(()));
        }
    }
}

#[test]
fn sha512_signs_under_the_keys_wide_enough_for_it() {
    let key = PrivateKey::new(generated_2048_key(), &unhex(GENERATED_2048_PRIVATE))
        .expect("the exponent fits");
    let mut signature = vec![0u8; 256];
    assert_eq!(
        key.sign_pkcs1(HashId::Sha512, MESSAGE, &mut signature),
        Ok(())
    );
    assert_eq!(
        key.public()
            .verify_pkcs1(HashId::Sha512, MESSAGE, &signature),
        Ok(())
    );
}

#[test]
fn leading_zero_bytes_of_the_exponent_are_dropped() {
    let mut padded = vec![0u8, 0];
    padded.extend(unhex(GENERATED_1032_PRIVATE));
    let key = PrivateKey::new(generated_1032_key(), &padded).expect("the digits fit");
    let mut signature = vec![0u8; 129];
    assert_eq!(
        key.sign_pkcs1(HashId::Sha256, MESSAGE, &mut signature),
        Ok(())
    );
}

#[test]
fn a_zero_or_too_wide_exponent_is_refused() {
    for exponent in [vec![], vec![0u8; 3], vec![1u8; 257]] {
        assert!(matches!(
            PrivateKey::new(generated_2048_key(), &exponent),
            Err(RsaError::InvalidExponent)
        ));
    }
}

#[test]
fn an_exponent_of_another_key_signs_nothing_and_wipes_the_buffer() {
    let key = PrivateKey::new(generated_2048_key(), &[3]).expect("three fits");
    let mut signature = vec![0xa5u8; 256];
    assert_eq!(
        key.sign_pkcs1(HashId::Sha256, MESSAGE, &mut signature),
        Err(RsaError::BadSignature)
    );
    assert!(signature.iter().all(|byte| *byte == 0));
}

#[test]
fn a_buffer_of_the_wrong_length_is_refused_and_wiped() {
    let key = PrivateKey::new(rfc8448_key(), &unhex(RFC8448_PRIVATE)).expect("the exponent fits");
    for length in [0usize, 127, 129] {
        let mut signature = vec![0xa5u8; length];
        assert_eq!(
            key.sign_pkcs1(HashId::Sha256, MESSAGE, &mut signature),
            Err(RsaError::BadSignature)
        );
        assert!(signature.iter().all(|byte| *byte == 0));
    }
}
