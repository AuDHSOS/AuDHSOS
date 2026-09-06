// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! RSA in a certificate: the identifiers, the parameters rule that is a
//! rule per algorithm, the key of RFC 3279, section 2.3.1, the size bound
//! of D-79, and chains under each of the six schemes.

use audhsos_der::Reader;

use crate::algorithm::{MAX_RSA_BITS, MIN_RSA_BITS, SignatureAlgorithm, SubjectPublicKey};
use crate::builder::{Params, RsaScheme, RsaTestKey, TestKey};
use crate::error::X509Error;
use crate::oid;
use crate::test_keys::{RSA_2048, RSA_2048_MODULUS, RSA_4096};
use crate::tests::{build_certificate, early, late};

/// A value with the given tag and content, for the identifiers this file
/// writes by hand.
fn encode(tag: u8, content: &[u8]) -> Vec<u8> {
    let length = content.len();
    let mut out = vec![tag];
    if length < 0x80 {
        out.push(u8::try_from(length).unwrap_or(0));
    } else if length <= 0xFF {
        out.push(0x81);
        out.push(u8::try_from(length).unwrap_or(0));
    } else {
        out.push(0x82);
        out.push(u8::try_from(length >> 8).unwrap_or(0));
        out.push(u8::try_from(length & 0xFF).unwrap_or(0));
    }
    out.extend_from_slice(content);
    out
}

/// An algorithm identifier with the given object identifier and whatever
/// parameters follow it.
fn identifier(algorithm: &[u8], parameters: &[u8]) -> Vec<u8> {
    let mut fields = encode(0x06, algorithm);
    fields.extend_from_slice(parameters);
    encode(0x30, &fields)
}

/// The `RSASSA-PSS-params` for one hash and one salt length.
fn pss_parameters(hash: &[u8], salt: u8, mask_hash: &[u8], null: bool) -> Vec<u8> {
    let hash_algorithm = if null {
        identifier(hash, &encode(0x05, &[]))
    } else {
        identifier(hash, &[])
    };
    let mask_algorithm = identifier(oid::MGF1, &identifier(mask_hash, &[]));
    let mut params = encode(0xA0, &hash_algorithm);
    params.extend_from_slice(&encode(0xA1, &mask_algorithm));
    params.extend_from_slice(&encode(0xA2, &encode(0x02, &[salt])));
    encode(0x30, &params)
}

/// The algorithm the identifier names, or the error.
fn parse(bytes: &[u8]) -> Result<SignatureAlgorithm, X509Error> {
    SignatureAlgorithm::parse(&mut Reader::new(bytes))
}

/// RFC 4055, section 5: NULL, and an implementation must accept the field
/// absent as well. No other algorithm in this crate allows both.
#[test]
fn the_three_rsa_signature_algorithms_take_null_and_absent_parameters() {
    for (arc, expected) in [
        (oid::SHA256_WITH_RSA, SignatureAlgorithm::RsaPkcs1Sha256),
        (oid::SHA384_WITH_RSA, SignatureAlgorithm::RsaPkcs1Sha384),
        (oid::SHA512_WITH_RSA, SignatureAlgorithm::RsaPkcs1Sha512),
    ] {
        assert_eq!(parse(&identifier(arc, &encode(0x05, &[]))), Ok(expected));
        assert_eq!(parse(&identifier(arc, &[])), Ok(expected));
        // Anything else in the field is still refused.
        assert!(parse(&identifier(arc, &encode(0x02, &[0x01]))).is_err());
    }
}

/// The other half of the rule: absence is the only right answer for the
/// algorithms that are not RSA, so a NULL is refused there.
#[test]
fn the_other_algorithms_still_refuse_a_null() {
    for arc in [oid::ECDSA_WITH_SHA256, oid::ECDSA_WITH_SHA384, oid::ED25519] {
        assert!(parse(&identifier(arc, &encode(0x05, &[]))).is_err());
    }
}

#[test]
fn the_three_pss_parameter_sets_are_read_and_no_others_are() {
    for (hash, salt, expected) in [
        (oid::SHA256, 32u8, SignatureAlgorithm::RsaPssSha256),
        (oid::SHA384, 48, SignatureAlgorithm::RsaPssSha384),
        (oid::SHA512, 64, SignatureAlgorithm::RsaPssSha512),
    ] {
        let parameters = pss_parameters(hash, salt, hash, false);
        assert_eq!(
            parse(&identifier(oid::RSASSA_PSS, &parameters)),
            Ok(expected)
        );
        // RFC 4055, section 2.1: the hash identifier may carry a NULL.
        let with_null = pss_parameters(hash, salt, hash, true);
        assert_eq!(
            parse(&identifier(oid::RSASSA_PSS, &with_null)),
            Ok(expected)
        );
    }
}

#[test]
fn a_pss_parameter_set_this_client_does_not_offer_is_refused() {
    // A salt that is not as long as the hash output (D-81).
    let wrong_salt = pss_parameters(oid::SHA256, 20, oid::SHA256, false);
    assert_eq!(
        parse(&identifier(oid::RSASSA_PSS, &wrong_salt)),
        Err(X509Error::UnsupportedAlgorithm)
    );
    // A mask over another hash than the one that made the digest.
    let split = pss_parameters(oid::SHA256, 32, oid::SHA384, false);
    assert_eq!(
        parse(&identifier(oid::RSASSA_PSS, &split)),
        Err(X509Error::UnsupportedAlgorithm)
    );
    // A hash this crate does not know: `id-sha224`.
    let sha224: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x04];
    let unknown = pss_parameters(sha224, 28, sha224, false);
    assert_eq!(
        parse(&identifier(oid::RSASSA_PSS, &unknown)),
        Err(X509Error::UnsupportedAlgorithm)
    );
    // A mask generation function that is not MGF1.
    let hash_algorithm = identifier(oid::SHA256, &[]);
    let mask = identifier(oid::SHA256, &identifier(oid::SHA256, &[]));
    let mut params = encode(0xA0, &hash_algorithm);
    params.extend_from_slice(&encode(0xA1, &mask));
    params.extend_from_slice(&encode(0xA2, &encode(0x02, &[32])));
    assert_eq!(
        parse(&identifier(oid::RSASSA_PSS, &encode(0x30, &params))),
        Err(X509Error::UnsupportedAlgorithm)
    );
    // No parameters at all, which the three sets never are.
    assert!(parse(&identifier(oid::RSASSA_PSS, &[])).is_err());
}

/// The subject public key information of RFC 3279, section 2.3.1: the
/// parameters MUST be NULL, and the key is a sequence of two integers.
#[test]
fn an_rsa_public_key_is_the_two_integers_of_rfc_3279() {
    let mut pair = encode(0x02, &{
        let mut modulus = vec![0x00];
        modulus.extend_from_slice(RSA_2048_MODULUS);
        modulus
    });
    pair.extend_from_slice(&encode(0x02, &[0x01, 0x00, 0x01]));
    let key = encode(0x30, &pair);
    let mut bits = vec![0x00];
    bits.extend_from_slice(&key);

    let algorithm = identifier(oid::RSA_ENCRYPTION, &encode(0x05, &[]));
    let mut info = algorithm;
    info.extend_from_slice(&encode(0x03, &bits));
    let encoded = encode(0x30, &info);

    assert_eq!(
        SubjectPublicKey::parse(&mut Reader::new(&encoded)),
        Ok(SubjectPublicKey::Rsa {
            modulus: RSA_2048_MODULUS,
            exponent: &[0x01, 0x00, 0x01],
        })
    );

    // Without the NULL parameters, which RFC 3279 requires.
    let algorithm = identifier(oid::RSA_ENCRYPTION, &[]);
    let mut info = algorithm;
    info.extend_from_slice(&encode(0x03, &bits));
    let encoded = encode(0x30, &info);
    assert!(SubjectPublicKey::parse(&mut Reader::new(&encoded)).is_err());
}

/// A modulus of `bits` bits: odd, with its top bit set, and nothing else
/// about it true. `check_usable` judges the size and the shape, and a
/// number that is not a product of two primes is neither.
fn modulus_of(bits: usize) -> Vec<u8> {
    let mut bytes = vec![0x55u8; bits / 8];
    if let Some(first) = bytes.first_mut() {
        *first |= 0x80;
    }
    if let Some(last) = bytes.last_mut() {
        *last |= 0x01;
    }
    bytes
}

/// The size bound of D-79, at each edge. It is here and not in
/// `crypto-rsa`, which is what lets the thousand-and-twenty-four-bit key
/// of RFC 8448 exercise the primitive while no chain carries one.
#[test]
fn the_size_bound_is_applied_where_a_certificate_is_judged() {
    assert_eq!(MIN_RSA_BITS, 2048);
    assert_eq!(MAX_RSA_BITS, 4096);
    for bits in [MIN_RSA_BITS, 3072, MAX_RSA_BITS] {
        let modulus = modulus_of(bits);
        let key = SubjectPublicKey::Rsa {
            modulus: &modulus,
            exponent: &[0x01, 0x00, 0x01],
        };
        assert_eq!(key.check_usable(), Ok(()), "{bits} bits");
    }
    for bits in [1024usize, 2040, 4104] {
        let modulus = modulus_of(bits);
        let key = SubjectPublicKey::Rsa {
            modulus: &modulus,
            exponent: &[0x01, 0x00, 0x01],
        };
        assert_eq!(
            key.check_usable(),
            Err(X509Error::BadPublicKey),
            "{bits} bits"
        );
    }
    // And a key the primitive itself refuses.
    let key = SubjectPublicKey::Rsa {
        modulus: &modulus_of(2048),
        exponent: &[0x01],
    };
    assert_eq!(key.check_usable(), Err(X509Error::BadPublicKey));
}

/// A chain of two certificates under one scheme: an authority that signs
/// itself and a leaf it signs.
fn chain_verifies(key: RsaTestKey, scheme: RsaScheme) {
    let signer = TestKey::Rsa(key, scheme);
    let authority = build_certificate(
        &Params::authority("root", "root", None, early(), late()),
        signer,
        signer,
    )
    .expect("the authority is built");
    let leaf = build_certificate(
        &Params::leaf("root", "leaf", &["leaf.example"], early(), late()),
        signer,
        signer,
    )
    .expect("the leaf is built");

    for built in [&authority, &leaf] {
        let certificate = crate::certificate::Certificate::parse(built.as_slice())
            .expect("the certificate parses");
        assert_eq!(certificate.algorithm, scheme.algorithm());
        let issuer = crate::certificate::Certificate::parse(authority.as_slice())
            .expect("the authority parses");
        issuer
            .spki
            .verify(
                certificate.algorithm,
                certificate.tbs,
                certificate.signature,
            )
            .expect("the signature verifies");
    }
}

#[test]
fn a_chain_verifies_under_each_of_the_six_schemes() {
    for scheme in [
        RsaScheme::Pkcs1Sha256,
        RsaScheme::Pkcs1Sha384,
        RsaScheme::Pkcs1Sha512,
        RsaScheme::PssSha256,
        RsaScheme::PssSha384,
        RsaScheme::PssSha512,
    ] {
        chain_verifies(RSA_2048, scheme);
    }
}

/// The widest key this system accepts, which is what took
/// `MAX_CERTIFICATE` past a kibibyte.
#[test]
fn a_four_thousand_and_ninety_six_bit_certificate_is_built_and_verified() {
    chain_verifies(RSA_4096, RsaScheme::Pkcs1Sha256);
}

#[test]
fn a_changed_body_does_not_verify_under_an_rsa_key() {
    let signer = TestKey::Rsa(RSA_2048, RsaScheme::Pkcs1Sha256);
    let built = build_certificate(
        &Params::leaf("root", "leaf", &["leaf.example"], early(), late()),
        signer,
        signer,
    )
    .expect("the leaf is built");
    let certificate = crate::certificate::Certificate::parse(built.as_slice()).expect("it parses");
    let mut body = certificate.tbs.to_vec();
    if let Some(last) = body.last_mut() {
        *last ^= 0x01;
    }
    assert_eq!(
        certificate
            .spki
            .verify(certificate.algorithm, &body, certificate.signature),
        Err(X509Error::SignatureFailed)
    );
}

#[test]
fn an_rsa_key_does_not_verify_under_an_algorithm_that_is_not_rsa() {
    let key = SubjectPublicKey::Rsa {
        modulus: RSA_2048_MODULUS,
        exponent: &[0x01, 0x00, 0x01],
    };
    assert_eq!(
        key.verify(SignatureAlgorithm::EcdsaSha256, b"body", &[0u8; 64]),
        Err(X509Error::UnsupportedAlgorithm)
    );
    // And a curve key does not verify under an RSA algorithm.
    let curve = SubjectPublicKey::Ed25519(&[0u8; 32]);
    assert_eq!(
        curve.verify(SignatureAlgorithm::RsaPkcs1Sha256, b"body", &[0u8; 64]),
        Err(X509Error::UnsupportedAlgorithm)
    );
}

#[test]
fn a_key_the_primitive_refuses_does_not_verify() {
    let key = SubjectPublicKey::Rsa {
        modulus: RSA_2048_MODULUS,
        exponent: &[0x02],
    };
    assert_eq!(
        key.verify(SignatureAlgorithm::RsaPkcs1Sha256, b"body", &[0u8; 256]),
        Err(X509Error::BadPublicKey)
    );
}
