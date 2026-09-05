// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The pieces of a certificate on their own: the algorithm identifiers,
//! the public key information, the extensions, and the names. Building a
//! whole certificate for each malformed shape would say less and cost
//! more.

use audhsos_der::Reader;

use crate::algorithm::{SignatureAlgorithm, SubjectPublicKey};
use crate::builder::{Params, TestKey, build};
use crate::certificate::{
    BasicConstraints, Certificate, parse_basic_constraints, parse_extended_key_usage,
    parse_key_usage,
};
use crate::error::X509Error;
use crate::tests::{AUTHORITY_SECRET, LEAF_SECRET, build_certificate, early, late};

/// A value with the given tag and content.
fn encode(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag, u8::try_from(content.len()).unwrap_or(0)];
    out.extend_from_slice(content);
    out
}

#[test]
fn an_algorithm_this_crate_does_not_verify_is_refused() {
    // sha256WithRSAEncryption, which real chains use and this one does not.
    let rsa = [0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0B];
    let identifier = encode(0x30, &encode(0x06, &rsa));
    assert_eq!(
        SignatureAlgorithm::parse(&mut Reader::new(&identifier)).err(),
        Some(X509Error::UnsupportedAlgorithm)
    );

    // A signature algorithm with parameters, which none of the three take.
    let mut fields = encode(0x06, crate::oid::ED25519);
    fields.extend_from_slice(&encode(0x05, &[]));
    let identifier = encode(0x30, &fields);
    assert!(SignatureAlgorithm::parse(&mut Reader::new(&identifier)).is_err());
}

#[test]
fn a_public_key_must_match_its_algorithm() {
    let point = [0x04u8; 65];
    let key32 = [0x07u8; 32];

    // The right shapes parse.
    let mut algorithm = encode(0x06, crate::oid::EC_PUBLIC_KEY);
    algorithm.extend_from_slice(&encode(0x06, crate::oid::PRIME256V1));
    let mut bits = vec![0x00];
    bits.extend_from_slice(&point);
    let mut info = encode(0x30, &algorithm);
    info.extend_from_slice(&encode(0x03, &bits));
    let encoded = encode(0x30, &info);
    assert!(matches!(
        SubjectPublicKey::parse(&mut Reader::new(&encoded)),
        Ok(SubjectPublicKey::EcdsaP256(_))
    ));

    // Another curve.
    let mut algorithm = encode(0x06, crate::oid::EC_PUBLIC_KEY);
    algorithm.extend_from_slice(&encode(0x06, &[0x2B, 0x81, 0x04, 0x00, 0x22]));
    let mut info = encode(0x30, &algorithm);
    info.extend_from_slice(&encode(0x03, &bits));
    let encoded = encode(0x30, &info);
    assert_eq!(
        SubjectPublicKey::parse(&mut Reader::new(&encoded)).err(),
        Some(X509Error::UnsupportedAlgorithm)
    );

    // The right algorithm and the wrong length.
    let mut algorithm = encode(0x06, crate::oid::EC_PUBLIC_KEY);
    algorithm.extend_from_slice(&encode(0x06, crate::oid::PRIME256V1));
    let mut short = vec![0x00];
    short.extend_from_slice(&key32);
    let mut info = encode(0x30, &algorithm);
    info.extend_from_slice(&encode(0x03, &short));
    let encoded = encode(0x30, &info);
    assert_eq!(
        SubjectPublicKey::parse(&mut Reader::new(&encoded)).err(),
        Some(X509Error::BadPublicKey)
    );

    // Ed25519 with a point instead of a key.
    let algorithm = encode(0x06, crate::oid::ED25519);
    let mut info = encode(0x30, &algorithm);
    info.extend_from_slice(&encode(0x03, &bits));
    let encoded = encode(0x30, &info);
    assert_eq!(
        SubjectPublicKey::parse(&mut Reader::new(&encoded)).err(),
        Some(X509Error::BadPublicKey)
    );

    // An algorithm nothing here knows.
    let algorithm = encode(
        0x06,
        &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x01],
    );
    let mut info = encode(0x30, &algorithm);
    info.extend_from_slice(&encode(0x03, &short));
    let encoded = encode(0x30, &info);
    assert_eq!(
        SubjectPublicKey::parse(&mut Reader::new(&encoded)).err(),
        Some(X509Error::UnsupportedAlgorithm)
    );
}

#[test]
fn a_key_does_not_verify_under_the_wrong_algorithm() {
    let key = SubjectPublicKey::Ed25519(&[0u8; 32]);
    assert_eq!(
        key.verify(SignatureAlgorithm::EcdsaSha256, b"body", &[0u8; 64]),
        Err(X509Error::UnsupportedAlgorithm)
    );

    let point = SubjectPublicKey::EcdsaP256(&[0x04u8; 65]);
    assert_eq!(
        point.verify(SignatureAlgorithm::Ed25519, b"body", &[0u8; 64]),
        Err(X509Error::UnsupportedAlgorithm)
    );

    // A key of the right kind and the wrong length.
    let short = SubjectPublicKey::Ed25519(&[0u8; 31]);
    assert_eq!(
        short.verify(SignatureAlgorithm::Ed25519, b"body", &[0u8; 64]),
        Err(X509Error::BadPublicKey)
    );
}

#[test]
fn an_ecdsa_signature_must_be_two_integers_that_fit() {
    let authority = build_certificate(
        &Params::authority("A", "A", None, early(), late()),
        TestKey::EcdsaSha256(AUTHORITY_SECRET),
        TestKey::EcdsaSha256(AUTHORITY_SECRET),
    )
    .expect("the parameters fit");
    let authority = Certificate::parse(authority.as_slice()).expect("a well formed authority");

    for (name, signature) in [
        ("not a sequence", encode(0x02, &[0x01])),
        ("one integer", encode(0x30, &encode(0x02, &[0x01]))),
        (
            "an integer wider than the order",
            encode(
                0x30,
                &[encode(0x02, &[0x01u8; 33]), encode(0x02, &[0x01u8; 32])].concat(),
            ),
        ),
        ("trailing data", {
            let mut bytes = encode(
                0x30,
                &[encode(0x02, &[0x01u8; 32]), encode(0x02, &[0x01u8; 32])].concat(),
            );
            bytes.push(0x00);
            bytes
        }),
    ] {
        assert_eq!(
            authority
                .spki
                .verify(SignatureAlgorithm::EcdsaSha256, b"body", &signature),
            Err(X509Error::BadSignature),
            "{name}"
        );
    }
}

#[test]
fn the_algorithm_inside_the_body_must_match_the_one_beside_the_signature() {
    // Build with SHA-256 and turn the outer identifier into SHA-384, whose
    // encoding differs from it in one byte.
    let built = build_certificate(
        &Params::authority("A", "A", None, early(), late()),
        TestKey::EcdsaSha256(AUTHORITY_SECRET),
        TestKey::EcdsaSha256(AUTHORITY_SECRET),
    )
    .expect("the parameters fit");

    let position = built
        .as_slice()
        .windows(8)
        .rposition(|window| window == crate::oid::ECDSA_WITH_SHA256)
        .expect("the outer identifier is the last of the two");
    let mut bytes = built.bytes;
    if let Some(slot) = bytes.get_mut(position.wrapping_add(7)) {
        *slot = 0x03;
    }
    assert_eq!(
        Certificate::parse(bytes.get(..built.length).unwrap_or(&[])).err(),
        Some(X509Error::AlgorithmMismatch)
    );
}

#[test]
fn the_basic_constraints_must_be_consistent() {
    assert_eq!(
        parse_basic_constraints(&encode(0x30, &[])),
        Ok(BasicConstraints {
            ca: false,
            path_len: None
        })
    );
    assert_eq!(
        parse_basic_constraints(&encode(0x30, &encode(0x01, &[0xFF]))),
        Ok(BasicConstraints {
            ca: true,
            path_len: None
        })
    );

    // A path length without the authority bit says nothing.
    assert_eq!(
        parse_basic_constraints(&encode(0x30, &encode(0x02, &[0x01]))),
        Err(X509Error::BadExtension)
    );
    // The default value must not be encoded.
    assert_eq!(
        parse_basic_constraints(&encode(0x30, &encode(0x01, &[0x00]))),
        Err(X509Error::DefaultEncoded)
    );
    // A path length that does not fit.
    let long = [encode(0x01, &[0xFF]), encode(0x02, &[0x01u8; 8])].concat();
    assert_eq!(
        parse_basic_constraints(&encode(0x30, &long)),
        Err(X509Error::BadExtension)
    );
}

#[test]
fn the_key_usage_and_the_purposes_must_carry_something() {
    // One bit, the first: a key that signs and does not certify.
    let usage = parse_key_usage(&encode(0x03, &[0x07, 0x80])).expect("a well formed usage");
    assert!(usage.digital_signature());
    assert!(!usage.key_cert_sign());

    // The sixth bit, which is the one an authority needs.
    let usage = parse_key_usage(&encode(0x03, &[0x02, 0x04])).expect("a well formed usage");
    assert!(usage.key_cert_sign());
    assert!(!usage.digital_signature());

    assert_eq!(
        parse_key_usage(&encode(0x03, &[0x00])),
        Err(X509Error::BadExtension),
        "no bits at all"
    );

    assert_eq!(
        parse_extended_key_usage(&encode(0x30, &encode(0x06, crate::oid::SERVER_AUTH))),
        Ok(true)
    );
    assert_eq!(
        parse_extended_key_usage(&encode(0x30, &[])),
        Err(X509Error::BadExtension),
        "no purpose at all"
    );
}

#[test]
fn the_names_skip_what_is_not_a_dns_name_and_stop_at_what_is_broken() {
    let entries = [
        encode(0x81, b"someone@example.test"),
        encode(0x82, b"example.test"),
        encode(0x87, &[192, 0, 2, 1]),
        encode(0x82, b"www.example.test"),
    ]
    .concat();
    let names: Vec<&[u8]> = crate::certificate::DnsNames::from_names(&entries)
        .map(|name| name.expect("the entries are well formed"))
        .collect();
    assert_eq!(names, vec![&b"example.test"[..], &b"www.example.test"[..]]);

    // A malformed entry ends the iteration with an error rather than a
    // name that was never there.
    let broken = [encode(0x82, b"example.test"), vec![0x82, 0x7F]].concat();
    let mut iterator = crate::certificate::DnsNames::from_names(&broken);
    assert_eq!(iterator.next(), Some(Ok(&b"example.test"[..])));
    assert!(matches!(iterator.next(), Some(Err(_))));
    assert!(iterator.next().is_none());
}

#[test]
fn a_certificate_that_does_not_fit_is_refused_rather_than_truncated() {
    let params = Params::leaf("A", "b.test", &["b.test"], early(), late());
    let mut small = [0u8; 32];
    assert_eq!(
        build(
            &params,
            TestKey::Ed25519(LEAF_SECRET),
            TestKey::Ed25519(AUTHORITY_SECRET),
            &mut small
        ),
        Err(X509Error::BufferTooSmall)
    );
}
