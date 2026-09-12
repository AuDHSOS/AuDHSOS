// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Ed25519 against the vectors of RFC 8032, section 7.1.

use crate::ed25519::{Point, public_key, sign, verify};
use crate::error::EcError;
use crate::scalar::Scalar;
use crate::tests::{hex, unhex, unhex32};

/// One vector: a secret, the public key it expands to, a message, and the
/// signature that is the only correct one, signing being deterministic.
struct Case {
    /// The label of the vector.
    name: &'static str,
    /// The secret key.
    secret: &'static str,
    /// The public key.
    public: &'static str,
    /// The message.
    message: &'static str,
    /// The signature.
    signature: &'static str,
}

/// Four vectors of RFC 8032, section 7.1.
const CASES: [Case; 4] = [
    Case {
        name: "1",
        secret: "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
        public: "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
        message: "",
        signature: "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e0652249015\
                    55fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
    },
    Case {
        name: "2",
        secret: "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
        public: "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
        message: "72",
        signature: "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69d\
                    a085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
    },
    Case {
        name: "3",
        secret: "c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7",
        public: "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025",
        message: "af82",
        signature: "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3a\
                    c18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a",
    },
    Case {
        name: "long",
        secret: "f5e5767cf153319517630f226876b86c8160cc583bc013744c6bf255f5cc0ee5",
        public: "278117fc144c72340f67d0f2316e8386ceffbf2b2428c9c51fef7c597f1d426e",
        message: "08b8b2b733424243760fe426a4b54908632110a66c2f6591eabd3345e3e4eb98\
                  fa6e264bf09efe12ee50f8f54e9f77b1e355f6c50544e23fb1433ddf73be84d8\
                  79de7c0046dc4996d9e773f4bc9efe5738829adb26c81b37c93a1b270b20329d",
        signature: "f35b8b58cff047f8185f17acc239e92e43b4c6fa36468a40fa62ffc223f7cd1\
                    44bcb74317d31b052a2935c1c57486a1c4705fb693fb122605ed3bb685390da01",
    },
];

/// The signature of a case as a fixed-size array.
fn signature_of(case: &Case) -> [u8; 64] {
    let mut bytes = [0u8; 64];
    for (slot, byte) in bytes.iter_mut().zip(unhex(case.signature)) {
        *slot = byte;
    }
    bytes
}

#[test]
fn the_rfc_vectors_verify() {
    for case in &CASES {
        let public = unhex32(case.public);
        let message = unhex(case.message);
        assert_eq!(
            verify(&public, &message, &signature_of(case)),
            Ok(()),
            "vector {}",
            case.name
        );
    }
}

#[test]
fn signing_reproduces_the_signatures_of_the_rfc() {
    for case in &CASES {
        let secret = unhex32(case.secret);
        assert_eq!(
            hex(&public_key(&secret)),
            case.public,
            "key of {}",
            case.name
        );
        assert_eq!(
            hex(&sign(&secret, &unhex(case.message))),
            case.signature.replace(['\n', ' '], ""),
            "signature of {}",
            case.name
        );
    }
}

#[test]
fn a_changed_message_or_signature_does_not_verify() {
    let case = &CASES[1];
    let public = unhex32(case.public);
    let signature = signature_of(case);

    assert_eq!(
        verify(&public, b"\x73", &signature),
        Err(EcError::BadSignature),
        "another message"
    );

    for position in 0..32 {
        let mut damaged = signature;
        damaged[position] ^= 0x01;
        assert_ne!(
            verify(&public, &unhex(case.message), &damaged),
            Ok(()),
            "flipped byte {position} of the commitment"
        );
    }
}

#[test]
fn a_response_at_or_above_the_order_is_refused() {
    let case = &CASES[1];
    let public = unhex32(case.public);
    let order = unhex32("edd3f55c1a631258d69cf7a2def9de1400000000000000000000000000000010");

    let mut signature = signature_of(case);
    for (slot, byte) in signature.iter_mut().skip(32).zip(order) {
        *slot = byte;
    }
    assert_eq!(
        verify(&public, &unhex(case.message), &signature),
        Err(EcError::InvalidScalar)
    );

    let mut above = signature;
    above[32] = above[32].wrapping_add(1);
    assert_eq!(
        verify(&public, &unhex(case.message), &above),
        Err(EcError::InvalidScalar)
    );
}

#[test]
fn a_key_that_is_not_a_canonical_point_is_refused() {
    let case = &CASES[1];
    let signature = signature_of(case);
    let message = unhex(case.message);

    // A `y` coordinate at the prime, which is not a canonical encoding.
    let prime = unhex32("edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f");
    assert_eq!(
        verify(&prime, &message, &signature),
        Err(EcError::InvalidPoint)
    );

    // A `y` for which the curve equation has no solution.
    let mut not_a_point =
        unhex32("0200000000000000000000000000000000000000000000000000000000000000");
    for candidate in 2u8..40 {
        not_a_point[0] = candidate;
        if Point::decompress(&not_a_point).is_none() {
            assert_eq!(
                verify(&not_a_point, &message, &signature),
                Err(EcError::InvalidPoint),
                "y = {candidate}"
            );
            return;
        }
    }
    panic!("no non-point was found among the first candidates");
}

#[test]
fn a_key_of_small_order_is_refused() {
    let case = &CASES[1];
    let signature = signature_of(case);
    let message = unhex(case.message);

    for (name, encoded) in [
        (
            "the identity",
            "0100000000000000000000000000000000000000000000000000000000000000",
        ),
        (
            "the point of order two",
            "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        ),
        (
            "a point of order four",
            "0000000000000000000000000000000000000000000000000000000000000000",
        ),
        (
            "a point of order eight",
            "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a",
        ),
    ] {
        let key = unhex32(encoded);
        let point = Point::decompress(&key).expect("the encoding is a point");
        assert!(point.is_small_order(), "{name} has small order");
        assert_eq!(
            verify(&key, &message, &signature),
            Err(EcError::InvalidPoint),
            "{name} is refused"
        );
    }
}

#[test]
fn the_group_law_behaves() {
    let base = Point::base();
    assert!(!base.is_small_order());
    assert!(Point::IDENTITY.is_identity());
    assert_eq!(base.add(Point::IDENTITY).compress(), base.compress());
    assert_eq!(base.double().compress(), base.add(base).compress());
    assert_eq!(
        base.double().double().compress(),
        base.add(base).add(base).add(base).compress()
    );
}

#[test]
fn signing_and_verifying_round_trip_for_generated_keys() {
    for seed in 0u8..8 {
        let secret = [seed.wrapping_mul(37).wrapping_add(1); 32];
        let public = public_key(&secret);
        let message = [seed; 40];
        let signature = sign(&secret, &message);
        assert_eq!(verify(&public, &message, &signature), Ok(()), "seed {seed}");

        let mut other = message;
        other[0] ^= 0x01;
        assert_eq!(
            verify(&public, &other, &signature),
            Err(EcError::BadSignature),
            "seed {seed}"
        );
    }
}

#[test]
fn the_two_multiplications_answer_the_same_point() {
    // What the masked ladder is worth rests on its agreeing with the
    // formula it replaces, at the ends of the range as well as inside it.
    let mut scalars = vec![Scalar::ZERO, Scalar::from_bytes_reduced(&[0xFF; 32])];
    for seed in 0u8..6 {
        let mut bytes = [0u8; 32];
        for (index, slot) in bytes.iter_mut().enumerate() {
            let index = u8::try_from(index).unwrap_or(0);
            *slot = seed.wrapping_mul(53).wrapping_add(index.wrapping_mul(7));
        }
        scalars.push(Scalar::from_bytes_reduced(&bytes));
    }
    let base = Point::base();
    for scalar in scalars {
        assert_eq!(
            base.mul_secret(scalar).compress(),
            base.mul(scalar).compress()
        );
        let other = base.double().add(base);
        assert_eq!(
            other.mul_secret(scalar).compress(),
            other.mul(scalar).compress()
        );
    }
}

#[test]
fn the_two_scalar_products_answer_the_same_scalar() {
    let one = Scalar::from_bytes_reduced(&[1u8; 32]);
    let two = Scalar::from_bytes_reduced(&[0xFEu8; 32]);
    for left in [Scalar::ZERO, one, two] {
        for right in [Scalar::ZERO, one, two] {
            assert_eq!(left.mul_secret(right), left.mul(right));
        }
    }
}
