// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Chain validation, against chains this crate builds.

use audhsos_time::CivilTime;

use crate::builder::{Params, TestKey};
use crate::certificate::{BasicConstraints, Certificate};
use crate::error::X509Error;
use crate::name::ServerName;
use crate::path::{TrustAnchor, TrustAnchors, verify_chain};
use crate::tests::{AUTHORITY_SECRET, Built, LEAF_SECRET, build_certificate, early, late};

/// The name the leaves of these tests carry.
const NAME: &str = "example.test";

/// A moment inside every window these tests build.
fn now() -> CivilTime {
    CivilTime {
        year: 2025,
        month: 6,
        day: 15,
        hour: 12,
        minute: 0,
        second: 0,
    }
}

/// The secret of the intermediate.
const INTERMEDIATE_SECRET: [u8; 32] = [0x44; 32];

/// A self-signed root.
fn root() -> Built {
    let params = Params::authority("Root", "Root", None, early(), late());
    build_certificate(
        &params,
        TestKey::Ed25519(AUTHORITY_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    )
    .expect("the parameters fit")
}

/// An intermediate under the root.
fn intermediate(path_len: Option<u32>) -> Built {
    let params = Params::authority("Root", "Intermediate", path_len, early(), late());
    build_certificate(
        &params,
        TestKey::Ed25519(INTERMEDIATE_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    )
    .expect("the parameters fit")
}

/// A leaf under the given issuer name, signed by the given key.
fn leaf_signed_by(issuer: &str, key: TestKey, window: (CivilTime, CivilTime)) -> Built {
    let names = [NAME];
    let params = Params::leaf(issuer, NAME, &names, window.0, window.1);
    build_certificate(&params, TestKey::Ed25519(LEAF_SECRET), key).expect("the parameters fit")
}

/// The anchor a parsed root stands for.
fn anchor<'a>(certificate: &Certificate<'a>) -> TrustAnchor<'a> {
    TrustAnchor {
        subject: certificate.subject,
        spki: certificate.spki_bytes,
    }
}

#[test]
fn a_chain_through_an_intermediate_reaches_the_anchor() {
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");
    let middle_bytes = intermediate(None);
    let middle = Certificate::parse(middle_bytes.as_slice()).expect("a well formed intermediate");
    let leaf_bytes = leaf_signed_by(
        "Intermediate",
        TestKey::Ed25519(INTERMEDIATE_SECRET),
        (early(), late()),
    );
    let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");

    let anchors = [anchor(&root)];
    assert_eq!(
        verify_chain(
            &leaf,
            &[middle],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            now()
        ),
        Ok(())
    );
}

#[test]
fn a_leaf_directly_under_the_anchor_reaches_it() {
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");
    let leaf_bytes = leaf_signed_by(
        "Root",
        TestKey::Ed25519(AUTHORITY_SECRET),
        (early(), late()),
    );
    let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");

    let anchors = [anchor(&root)];
    assert_eq!(
        verify_chain(
            &leaf,
            &[],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            now()
        ),
        Ok(())
    );
}

#[test]
fn intermediates_may_arrive_in_any_order_and_with_strangers_among_them() {
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");
    let middle_bytes = intermediate(None);
    let middle = Certificate::parse(middle_bytes.as_slice()).expect("a well formed intermediate");
    let stranger_bytes = build_certificate(
        &Params::authority("Elsewhere", "Elsewhere", None, early(), late()),
        TestKey::Ed25519([0x77; 32]),
        TestKey::Ed25519([0x77; 32]),
    )
    .expect("the parameters fit");
    let stranger = Certificate::parse(stranger_bytes.as_slice()).expect("a well formed stranger");

    let leaf_bytes = leaf_signed_by(
        "Intermediate",
        TestKey::Ed25519(INTERMEDIATE_SECRET),
        (early(), late()),
    );
    let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");

    let anchors = [anchor(&root)];
    assert_eq!(
        verify_chain(
            &leaf,
            &[stranger, middle],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            now()
        ),
        Ok(())
    );
}

#[test]
fn a_window_that_does_not_contain_the_moment_is_refused() {
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");
    let anchors = [anchor(&root)];

    let expired = CivilTime {
        year: 2021,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    };
    let bytes = leaf_signed_by(
        "Root",
        TestKey::Ed25519(AUTHORITY_SECRET),
        (early(), expired),
    );
    let leaf = Certificate::parse(bytes.as_slice()).expect("a well formed leaf");
    assert_eq!(
        verify_chain(
            &leaf,
            &[],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            now()
        ),
        Err(X509Error::Expired)
    );

    let future = CivilTime {
        year: 2029,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    };
    let bytes = leaf_signed_by("Root", TestKey::Ed25519(AUTHORITY_SECRET), (future, late()));
    let leaf = Certificate::parse(bytes.as_slice()).expect("a well formed leaf");
    assert_eq!(
        verify_chain(
            &leaf,
            &[],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            now()
        ),
        Err(X509Error::NotYetValid)
    );
}

#[test]
fn an_intermediate_outside_its_window_is_refused() {
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");
    let expired = CivilTime {
        year: 2021,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
    };
    let middle_bytes = build_certificate(
        &Params::authority("Root", "Intermediate", None, early(), expired),
        TestKey::Ed25519(INTERMEDIATE_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    )
    .expect("the parameters fit");
    let middle = Certificate::parse(middle_bytes.as_slice()).expect("a well formed intermediate");
    let leaf_bytes = leaf_signed_by(
        "Intermediate",
        TestKey::Ed25519(INTERMEDIATE_SECRET),
        (early(), late()),
    );
    let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");

    let anchors = [anchor(&root)];
    assert_eq!(
        verify_chain(
            &leaf,
            &[middle],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            now()
        ),
        Err(X509Error::Expired)
    );
}

#[test]
fn a_name_that_was_not_asked_for_is_refused() {
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");
    let leaf_bytes = leaf_signed_by(
        "Root",
        TestKey::Ed25519(AUTHORITY_SECRET),
        (early(), late()),
    );
    let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");

    let anchors = [anchor(&root)];
    assert_eq!(
        verify_chain(
            &leaf,
            &[],
            &TrustAnchors::new(&anchors),
            ServerName::Dns("other.test"),
            now()
        ),
        Err(X509Error::NameMismatch)
    );
}

#[test]
fn a_leaf_that_is_not_for_server_authentication_is_refused() {
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");

    let names = [NAME];
    let mut params = Params::leaf("Root", NAME, &names, early(), late());
    params.extended_key_usage = Some(false);
    let leaf_bytes = build_certificate(
        &params,
        TestKey::Ed25519(LEAF_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    )
    .expect("the parameters fit");
    let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");

    let anchors = [anchor(&root)];
    assert_eq!(
        verify_chain(
            &leaf,
            &[],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            now()
        ),
        Err(X509Error::NotForServerAuthentication)
    );
}

#[test]
fn a_signer_that_is_not_an_authority_is_refused() {
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");

    // An intermediate without the authority bit, and one that states a
    // usage without certificate signing.
    for (name, constraints, usage, expected) in [
        (
            "no constraints at all",
            None,
            Some(0x0400),
            X509Error::NotAnAuthority,
        ),
        (
            "the bit unset",
            Some(BasicConstraints {
                ca: false,
                path_len: None,
            }),
            Some(0x0400),
            X509Error::NotAnAuthority,
        ),
        (
            "a usage without certificate signing",
            Some(BasicConstraints {
                ca: true,
                path_len: None,
            }),
            Some(0x8000),
            X509Error::NotForCertificateSigning,
        ),
    ] {
        let mut params = Params::authority("Root", "Intermediate", None, early(), late());
        params.basic_constraints = constraints;
        params.key_usage = usage;
        let middle_bytes = build_certificate(
            &params,
            TestKey::Ed25519(INTERMEDIATE_SECRET),
            TestKey::Ed25519(AUTHORITY_SECRET),
        )
        .expect("the parameters fit");
        let middle =
            Certificate::parse(middle_bytes.as_slice()).expect("a well formed intermediate");
        let leaf_bytes = leaf_signed_by(
            "Intermediate",
            TestKey::Ed25519(INTERMEDIATE_SECRET),
            (early(), late()),
        );
        let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");

        let anchors = [anchor(&root)];
        assert_eq!(
            verify_chain(
                &leaf,
                &[middle],
                &TrustAnchors::new(&anchors),
                ServerName::Dns(NAME),
                now()
            ),
            Err(expected),
            "{name}"
        );
    }
}

#[test]
fn a_path_length_that_the_chain_exceeds_is_refused() {
    // An intermediate that may have nothing under it but a leaf carries a
    // path length of zero, which one intermediate below it exceeds.
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");

    let first_bytes = build_certificate(
        &Params::authority("Root", "First", Some(0), early(), late()),
        TestKey::Ed25519(INTERMEDIATE_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    )
    .expect("the parameters fit");
    let first = Certificate::parse(first_bytes.as_slice()).expect("a well formed intermediate");

    let second_bytes = build_certificate(
        &Params::authority("First", "Second", None, early(), late()),
        TestKey::Ed25519([0x55; 32]),
        TestKey::Ed25519(INTERMEDIATE_SECRET),
    )
    .expect("the parameters fit");
    let second = Certificate::parse(second_bytes.as_slice()).expect("a well formed intermediate");

    let leaf_bytes = leaf_signed_by("Second", TestKey::Ed25519([0x55; 32]), (early(), late()));
    let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");

    let anchors = [anchor(&root)];
    assert_eq!(
        verify_chain(
            &leaf,
            &[second, first],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            now()
        ),
        Err(X509Error::PathLengthExceeded)
    );
}

#[test]
fn a_chain_that_reaches_nothing_trusted_is_refused() {
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");
    let leaf_bytes = leaf_signed_by(
        "Intermediate",
        TestKey::Ed25519(INTERMEDIATE_SECRET),
        (early(), late()),
    );
    let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");
    let anchors = [anchor(&root)];

    // The intermediate is missing.
    assert_eq!(
        verify_chain(
            &leaf,
            &[],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            now()
        ),
        Err(X509Error::NoTrustAnchor)
    );

    // The intermediate is there and nothing is trusted.
    let middle_bytes = intermediate(None);
    let middle = Certificate::parse(middle_bytes.as_slice()).expect("a well formed intermediate");
    assert_eq!(
        verify_chain(
            &leaf,
            &[middle],
            &TrustAnchors::new(&[]),
            ServerName::Dns(NAME),
            now()
        ),
        Err(X509Error::NoTrustAnchor)
    );
}

#[test]
fn a_signature_that_does_not_belong_to_the_anchor_is_refused() {
    // An anchor with the right name and another key.
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");
    let other_bytes = build_certificate(
        &Params::authority("Root", "Root", None, early(), late()),
        TestKey::Ed25519([0x66; 32]),
        TestKey::Ed25519([0x66; 32]),
    )
    .expect("the parameters fit");
    let other = Certificate::parse(other_bytes.as_slice()).expect("a well formed root");

    let leaf_bytes = leaf_signed_by(
        "Root",
        TestKey::Ed25519(AUTHORITY_SECRET),
        (early(), late()),
    );
    let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");

    let anchors = [anchor(&other)];
    assert_eq!(
        verify_chain(
            &leaf,
            &[],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            now()
        ),
        Err(X509Error::SignatureFailed)
    );
    assert_eq!(root.subject, other.subject, "the two names are the same");
}

#[test]
fn a_chain_longer_than_the_limit_is_refused() {
    // Nine intermediates are more than a chain, and are refused before any
    // of them is looked at.
    let mut all = Vec::new();
    for step in 0u8..9 {
        let issuer = if step == 0 { 0u8 } else { step.wrapping_sub(1) };
        let params = Params::authority("Root", "Root", None, early(), late());
        all.push(
            build_certificate(
                &params,
                TestKey::Ed25519([step; 32]),
                TestKey::Ed25519([issuer; 32]),
            )
            .expect("the parameters fit"),
        );
    }
    let parsed: Vec<Certificate<'_>> = all
        .iter()
        .map(|built| Certificate::parse(built.as_slice()).expect("a well formed certificate"))
        .collect();

    let leaf_bytes = leaf_signed_by(
        "Root",
        TestKey::Ed25519(AUTHORITY_SECRET),
        (early(), late()),
    );
    let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");

    assert_eq!(
        verify_chain(
            &leaf,
            &parsed,
            &TrustAnchors::new(&[]),
            ServerName::Dns(NAME),
            now()
        ),
        Err(X509Error::ChainTooLong)
    );
}

#[test]
fn a_common_name_is_not_a_fallback_for_a_missing_alternative_name() {
    // The subject says the name that was asked for, and the certificate
    // carries no alternative name at all. RFC 6125 has not allowed that
    // fallback for years, and this crate does not either.
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");

    let mut params = Params::leaf("Root", NAME, &[], early(), late());
    params.dns_names = &[];
    let leaf_bytes = build_certificate(
        &params,
        TestKey::Ed25519(LEAF_SECRET),
        TestKey::Ed25519(AUTHORITY_SECRET),
    )
    .expect("the parameters fit");
    let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");
    assert!(leaf.subject_alt_name.is_none());

    let anchors = [anchor(&root)];
    assert_eq!(
        verify_chain(
            &leaf,
            &[],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            now()
        ),
        Err(X509Error::NameMismatch)
    );
}

#[test]
fn the_boundary_moments_of_a_window_are_inside_it() {
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");
    let anchors = [anchor(&root)];

    let opens = CivilTime {
        year: 2024,
        month: 3,
        day: 4,
        hour: 5,
        minute: 6,
        second: 7,
    };
    let closes = CivilTime {
        year: 2026,
        month: 8,
        day: 9,
        hour: 10,
        minute: 11,
        second: 12,
    };
    let bytes = leaf_signed_by("Root", TestKey::Ed25519(AUTHORITY_SECRET), (opens, closes));
    let leaf = Certificate::parse(bytes.as_slice()).expect("a well formed leaf");

    for moment in [opens, closes] {
        assert_eq!(
            verify_chain(
                &leaf,
                &[],
                &TrustAnchors::new(&anchors),
                ServerName::Dns(NAME),
                moment
            ),
            Ok(()),
            "the boundary itself is inside the window"
        );
    }

    let mut before = opens;
    before.second = before.second.wrapping_sub(1);
    assert_eq!(
        verify_chain(
            &leaf,
            &[],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            before
        ),
        Err(X509Error::NotYetValid)
    );

    let mut after = closes;
    after.second = after.second.wrapping_add(1);
    assert_eq!(
        verify_chain(
            &leaf,
            &[],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            after
        ),
        Err(X509Error::Expired)
    );
}

#[test]
fn a_self_signed_leaf_reaches_nothing() {
    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");

    let names = [NAME];
    let params = Params::leaf(NAME, NAME, &names, early(), late());
    let leaf_bytes = build_certificate(
        &params,
        TestKey::Ed25519(LEAF_SECRET),
        TestKey::Ed25519(LEAF_SECRET),
    )
    .expect("the parameters fit");
    let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");

    let anchors = [anchor(&root)];
    assert_eq!(
        verify_chain(
            &leaf,
            &[],
            &TrustAnchors::new(&anchors),
            ServerName::Dns(NAME),
            now()
        ),
        Err(X509Error::NoTrustAnchor)
    );
}

#[test]
fn property_no_damaged_chain_verifies() {
    use test_support::generators::range;
    use test_support::property::check;

    let root_bytes = root();
    let root = Certificate::parse(root_bytes.as_slice()).expect("a well formed root");
    let anchors = [anchor(&root)];
    let middle_bytes = intermediate(None);
    let leaf_bytes = leaf_signed_by(
        "Intermediate",
        TestKey::Ed25519(INTERMEDIATE_SECRET),
        (early(), late()),
    );
    let leaf = Certificate::parse(leaf_bytes.as_slice()).expect("a well formed leaf");
    let length = middle_bytes.length;

    check(
        "x509_damaged_chain",
        &range(0..=(length.saturating_sub(1))),
        |position| {
            let mut bytes = middle_bytes.bytes;
            if let Some(slot) = bytes.get_mut(*position) {
                *slot ^= 0x01;
            }
            let damaged = bytes.get(..length).unwrap_or(&[]);
            if damaged == middle_bytes.as_slice() {
                return Ok(());
            }
            let Ok(middle) = Certificate::parse(damaged) else {
                return Ok(());
            };
            if verify_chain(
                &leaf,
                &[middle],
                &TrustAnchors::new(&anchors),
                ServerName::Dns(NAME),
                now(),
            )
            .is_ok()
            {
                return Err(format!("a chain damaged at {position} still verified"));
            }
            Ok(())
        },
    );
}
