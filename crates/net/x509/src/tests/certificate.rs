// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Parsing and signature verification, against certificates this crate
//! builds and then damages.

use test_support::generators::range;
use test_support::property::check;

use crate::algorithm::{SignatureAlgorithm, SubjectPublicKey};
use crate::builder::{Params, TestKey};
use crate::certificate::{BasicConstraints, Certificate};
use crate::error::X509Error;
use crate::tests::{AUTHORITY_SECRET, LEAF_SECRET, build_certificate, early, late};

/// The names of the certificate the tests parse.
const NAMES: [&str; 2] = ["example.test", "www.example.test"];

/// A leaf signed by the authority, under the given key kinds.
fn leaf(subject: TestKey, issuer: TestKey) -> crate::tests::Built {
    let params = Params::leaf("Test Authority", "example.test", &NAMES, early(), late());
    build_certificate(&params, subject, issuer).expect("the parameters fit the buffer")
}

/// The authority, self-signed.
fn authority(key: TestKey) -> crate::tests::Built {
    let params = Params::authority("Test Authority", "Test Authority", Some(1), early(), late());
    build_certificate(&params, key, key).expect("the parameters fit the buffer")
}

#[test]
fn a_built_leaf_parses_into_the_fields_it_was_given() {
    let built = leaf(
        TestKey::Ed25519(LEAF_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    );
    let certificate =
        Certificate::parse(built.as_slice()).expect("the builder writes what the parser reads");

    assert_eq!(certificate.serial, &[0x01]);
    assert_eq!(certificate.algorithm, SignatureAlgorithm::Ed25519);
    assert_eq!(certificate.validity.not_before, early());
    assert_eq!(certificate.validity.not_after, late());
    assert!(matches!(certificate.spki, SubjectPublicKey::Ed25519(key) if key.len() == 32));
    assert_eq!(certificate.signature.len(), 64);
    assert_eq!(certificate.extended_key_usage, Some(true));
    assert!(certificate.basic_constraints.is_none());
    assert!(
        certificate
            .key_usage
            .expect("a leaf carries one")
            .digital_signature()
    );

    let names: Vec<&[u8]> = certificate
        .dns_names()
        .map(|name| name.expect("the names are well formed"))
        .collect();
    assert_eq!(names, vec![&b"example.test"[..], &b"www.example.test"[..]]);

    // The body that was signed is a slice of the input, so it is the body
    // that will be hashed.
    assert!(
        built
            .as_slice()
            .windows(certificate.tbs.len())
            .any(|window| window == certificate.tbs)
    );
}

#[test]
fn an_authority_parses_with_its_constraints() {
    let built = authority(TestKey::Ed25519(AUTHORITY_SECRET));
    let certificate = Certificate::parse(built.as_slice()).expect("a well formed authority");

    assert_eq!(
        certificate.basic_constraints,
        Some(BasicConstraints {
            ca: true,
            path_len: Some(1)
        })
    );
    assert!(
        certificate
            .key_usage
            .expect("an authority carries one")
            .key_cert_sign()
    );
    assert!(certificate.extended_key_usage.is_none());
    assert_eq!(certificate.issuer, certificate.subject, "it is self-signed");
}

#[test]
fn every_supported_algorithm_signs_and_verifies() {
    for (name, subject, issuer) in [
        (
            "ed25519",
            TestKey::Ed25519(LEAF_SECRET),
            TestKey::Ed25519(AUTHORITY_SECRET),
        ),
        (
            "ecdsa with sha-256",
            TestKey::EcdsaSha256(LEAF_SECRET),
            TestKey::EcdsaSha256(AUTHORITY_SECRET),
        ),
        (
            "ecdsa with sha-384",
            TestKey::EcdsaSha384(LEAF_SECRET),
            TestKey::EcdsaSha384(AUTHORITY_SECRET),
        ),
        (
            "an ecdsa leaf under an ed25519 authority",
            TestKey::EcdsaSha256(LEAF_SECRET),
            TestKey::Ed25519(AUTHORITY_SECRET),
        ),
    ] {
        let signer = authority(issuer);
        let signer = Certificate::parse(signer.as_slice()).expect("a well formed authority");
        let built = leaf(subject, issuer);
        let certificate = Certificate::parse(built.as_slice()).expect("a well formed leaf");
        assert_eq!(certificate.verify_signature(&signer), Ok(()), "{name}");
    }
}

#[test]
fn a_certificate_does_not_verify_under_the_wrong_key() {
    let other = authority(TestKey::Ed25519([0x33; 32]));
    let other = Certificate::parse(other.as_slice()).expect("a well formed authority");

    let built = leaf(
        TestKey::Ed25519(LEAF_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    );
    let certificate = Certificate::parse(built.as_slice()).expect("a well formed leaf");
    assert_eq!(
        certificate.verify_signature(&other),
        Err(X509Error::SignatureFailed)
    );
}

#[test]
fn a_version_other_than_three_is_refused() {
    let built = leaf(
        TestKey::Ed25519(LEAF_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    );
    let mut bytes = built.bytes;
    let position = built
        .as_slice()
        .windows(5)
        .position(|window| window == [0xA0, 0x03, 0x02, 0x01, 0x02])
        .expect("the version is written as an explicit integer");
    if let Some(slot) = bytes.get_mut(position.wrapping_add(4)) {
        *slot = 0x01;
    }
    assert_eq!(
        Certificate::parse(bytes.get(..built.length).unwrap_or(&[])).err(),
        Some(X509Error::NotVersionThree)
    );
}

#[test]
fn an_extension_marked_critical_with_the_default_value_is_refused() {
    // The builder writes `critical` as TRUE; turning it into FALSE
    // encodes a default, which the distinguished rules forbid.
    let built = authority(TestKey::Ed25519(AUTHORITY_SECRET));
    let mut bytes = built.bytes;
    let position = built
        .as_slice()
        .windows(3)
        .position(|window| window == [0x01, 0x01, 0xFF])
        .expect("the builder marks the constraints critical");
    if let Some(slot) = bytes.get_mut(position.wrapping_add(2)) {
        *slot = 0x00;
    }
    assert_eq!(
        Certificate::parse(bytes.get(..built.length).unwrap_or(&[])).err(),
        Some(X509Error::DefaultEncoded)
    );
}

#[test]
fn an_unknown_critical_extension_is_refused() {
    let mut params = Params::leaf("Test Authority", "example.test", &NAMES, early(), late());
    params.unknown_critical = true;
    let built = build_certificate(
        &params,
        TestKey::Ed25519(LEAF_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    )
    .expect("the parameters fit the buffer");

    assert_eq!(
        Certificate::parse(built.as_slice()).err(),
        Some(X509Error::UnknownCriticalExtension)
    );
}

/// An extension nobody has to understand is passed over, and the
/// certificate around it is read as if it were not there.
#[test]
fn an_unknown_extension_that_is_not_critical_is_passed_over() {
    let mut params = Params::leaf("Test Authority", "example.test", &NAMES, early(), late());
    params.unknown_harmless = true;
    let built = build_certificate(
        &params,
        TestKey::Ed25519(LEAF_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    )
    .expect("the parameters fit the buffer");

    let certificate = Certificate::parse(built.as_slice()).expect("the extension is skipped");
    assert!(
        certificate
            .dns_names()
            .any(|name| name == Ok(&b"example.test"[..])),
        "the names around it are still read"
    );
}

#[test]
fn a_repeated_extension_is_refused() {
    let mut params =
        Params::authority("Test Authority", "Test Authority", Some(1), early(), late());
    params.duplicate_extension = true;
    let built = build_certificate(
        &params,
        TestKey::Ed25519(AUTHORITY_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    )
    .expect("the parameters fit the buffer");

    assert_eq!(
        Certificate::parse(built.as_slice()).err(),
        Some(X509Error::DuplicateExtension)
    );
}

#[test]
fn an_extended_key_usage_without_server_authentication_is_seen() {
    let mut params = Params::leaf("Test Authority", "example.test", &NAMES, early(), late());
    params.extended_key_usage = Some(false);
    let built = build_certificate(
        &params,
        TestKey::Ed25519(LEAF_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    )
    .expect("the parameters fit the buffer");

    let certificate = Certificate::parse(built.as_slice()).expect("a well formed leaf");
    assert_eq!(certificate.extended_key_usage, Some(false));
}

#[test]
fn a_certificate_without_names_has_none_to_iterate() {
    let built = authority(TestKey::Ed25519(AUTHORITY_SECRET));
    let certificate = Certificate::parse(built.as_slice()).expect("a well formed authority");
    assert!(certificate.subject_alt_name.is_none());
    assert_eq!(certificate.dns_names().count(), 0);
}

#[test]
fn the_errors_render_a_message() {
    for error in [
        X509Error::Encoding(audhsos_der::DerError::Truncated),
        X509Error::NotVersionThree,
        X509Error::AlgorithmMismatch,
        X509Error::UnsupportedAlgorithm,
        X509Error::BadPublicKey,
        X509Error::BadSignature,
        X509Error::SignatureFailed,
        X509Error::DuplicateExtension,
        X509Error::UnknownCriticalExtension,
        X509Error::BadExtension,
        X509Error::DefaultEncoded,
        X509Error::BadName,
        X509Error::BufferTooSmall,
    ] {
        assert!(!format!("{error}").is_empty());
        assert!(!format!("{error:?}").is_empty());
    }
}

#[test]
fn property_no_damaged_certificate_parses_and_verifies() {
    let signer = authority(TestKey::Ed25519(AUTHORITY_SECRET));
    let signer = Certificate::parse(signer.as_slice()).expect("a well formed authority");
    let built = leaf(
        TestKey::Ed25519(LEAF_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    );
    let length = built.length;

    check(
        "x509_damaged",
        &range(0..=(length.saturating_sub(1))),
        |position| {
            for flip in [0x01u8, 0x80] {
                let mut bytes = built.bytes;
                if let Some(slot) = bytes.get_mut(*position) {
                    *slot ^= flip;
                }
                let damaged = bytes.get(..length).unwrap_or(&[]);
                if let Ok(certificate) = Certificate::parse(damaged)
                    && certificate.verify_signature(&signer).is_ok()
                    && damaged != built.as_slice()
                {
                    return Err(format!(
                        "a certificate damaged at {position} with {flip:#04x} still verified"
                    ));
                }
            }
            Ok(())
        },
    );
}
