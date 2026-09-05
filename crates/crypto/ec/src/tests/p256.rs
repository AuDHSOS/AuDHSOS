// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! P-256: the two moduli, the group, and ECDSA.

use crate::error::EcError;
use crate::p256::field::{Element, Order, Prime};
use crate::p256::point::Point;
use crate::p256::{public_key, sign};
use crate::tests::{hex, unhex, unhex32};

/// A small value as an element of the field.
fn small(value: u8) -> Element<Prime> {
    let mut bytes = [0u8; 32];
    bytes[31] = value;
    Element::from_canonical(&bytes).expect("a small value is below the modulus")
}

#[test]
fn the_field_multiplies_and_encodes() {
    assert_eq!(hex(&small(0).to_bytes()), hex(&[0u8; 32]));
    assert_eq!(small(1), Element::<Prime>::one());
    assert_eq!(small(2).mul(small(3)), small(6));
    assert_eq!(small(7).square(), small(49));
    assert_eq!(small(9).add(small(8)), small(17));
    assert_eq!(small(9).sub(small(8)), small(1));
    assert_eq!(small(3).double(), small(6));
    assert!(small(0).is_zero());
    assert!(!format!("{:?}", small(1)).is_empty());
}

#[test]
fn the_round_trip_through_bytes_keeps_the_value() {
    let bytes = unhex32("6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296");
    let element = Element::<Prime>::from_canonical(&bytes).expect("below the modulus");
    assert_eq!(element.to_bytes(), bytes);

    let scalar = Element::<Order>::from_canonical(&bytes).expect("below the order");
    assert_eq!(scalar.to_bytes(), bytes);
}

#[test]
fn a_value_at_or_above_the_modulus_is_refused_but_can_be_reduced() {
    let prime = unhex32("ffffffff00000001000000000000000000000000ffffffffffffffffffffffff");
    assert!(Element::<Prime>::from_canonical(&prime).is_none());
    assert!(Element::<Prime>::from_bytes_reduced(&prime).is_zero());

    let above = unhex32("ffffffff00000001000000000000000000000001000000000000000000000000");
    assert!(Element::<Prime>::from_canonical(&above).is_none());
    assert_eq!(Element::<Prime>::from_bytes_reduced(&above), small(1));

    let order = unhex32("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551");
    assert!(Element::<Order>::from_canonical(&order).is_none());
    assert!(Element::<Order>::from_bytes_reduced(&order).is_zero());

    let above_order = unhex32("ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632552");
    assert!(Element::<Order>::from_canonical(&above_order).is_none());
    assert_eq!(
        hex(&Element::<Order>::from_bytes_reduced(&above_order).to_bytes()),
        hex(&unhex32(
            "0000000000000000000000000000000000000000000000000000000000000001"
        ))
    );

    let largest = [0xFFu8; 32];
    assert!(Element::<Prime>::from_canonical(&largest).is_none());
    assert!(!Element::<Prime>::from_bytes_reduced(&largest).is_zero());
}

#[test]
fn inversion_undoes_multiplication_in_both_moduli() {
    for value in 1u8..12 {
        let element = small(value);
        assert_eq!(
            element.mul(element.invert()),
            Element::<Prime>::one(),
            "field, value {value}"
        );
    }

    let bytes = unhex32("c9afa9d845ba75166b5c215767b1d6934e50c3db36e89b127b8a622b120f6721");
    let scalar = Element::<Order>::from_canonical(&bytes).expect("below the order");
    assert_eq!(scalar.mul(scalar.invert()), Element::<Order>::one());
    assert_eq!(scalar.neg().add(scalar), Element::<Order>::zero());
}

/// The secret of RFC 6979, appendix A.2.5.
const SECRET: &str = "c9afa9d845ba75166b5c215767b1d6934e50c3db36e89b127b8a622b120f6721";
/// Its public key, uncompressed.
const PUBLIC: &str = "0460fed4ba255a9d31c961eb74c6356d68c049b8923b61fa6ce669622e60f29fb6\
                      7903fe1008b8bc99a41ae9e95628bc64f2f1b20c2d7e9f5177a3c294d4462299";
/// The group order.
const ORDER_BYTES: &str = "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551";

/// The affine coordinates of a point, for comparing points that may carry
/// different denominators.
fn affine(point: Point) -> Option<([u8; 32], [u8; 32])> {
    point.to_affine().map(|(x, y)| (x.to_bytes(), y.to_bytes()))
}

/// The public key of the appendix as a parsed key.
fn key() -> crate::p256::PublicKey {
    crate::p256::PublicKey::from_sec1(&unhex(PUBLIC)).expect("the appendix key is a point")
}

#[test]
fn the_generator_and_its_multiples_are_the_documented_points() {
    let (x, y) = Point::generator()
        .to_affine()
        .expect("the generator is not the neutral element");
    assert_eq!(
        hex(&x.to_bytes()),
        "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296"
    );
    assert_eq!(
        hex(&y.to_bytes()),
        "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5"
    );

    let (x, y) = Point::generator()
        .double()
        .to_affine()
        .expect("twice the generator is not the neutral element");
    assert_eq!(
        hex(&x.to_bytes()),
        "7cf27b188d034f7e8a52380304b51ac3c08969e277f21b35a60b48fc47669978"
    );
    assert_eq!(
        hex(&y.to_bytes()),
        "07775510db8ed040293d9ac69f7430dbba7dade63ce982299e04b79d227873d1"
    );

    let (x, y) = Point::generator()
        .double()
        .add(Point::generator())
        .to_affine()
        .expect("three times the generator is not the neutral element");
    assert_eq!(
        hex(&x.to_bytes()),
        "5ecbe4d1a6330a44c8f7ef951d4bf165e6c6b721efada985fb41661bc6e7fd6c"
    );
    assert_eq!(
        hex(&y.to_bytes()),
        "8734640c4998ff7e374b06ce1a64a2ecd82ab036384fb83d9a79b127a27d5032"
    );
}

#[test]
fn the_group_law_behaves() {
    let g = Point::generator();
    assert!(Point::identity().is_identity());
    assert!(!g.is_identity());
    assert_eq!(affine(g.add(Point::identity())), affine(g));
    assert_eq!(affine(Point::identity().add(g)), affine(g));
    assert!(Point::identity().double().is_identity());
    assert!(Point::identity().to_affine().is_none());

    // The multiple by the order is the neutral element, and the multiple
    // by one less is the negation of the generator.
    let order = unhex32(ORDER_BYTES);
    assert!(g.mul(&order).is_identity());

    let mut one_less = order;
    one_less[31] = one_less[31].wrapping_sub(1);
    let (x, y) = g
        .mul(&one_less)
        .to_affine()
        .expect("the multiple is not the neutral element");
    let (gx, gy) = g.to_affine().expect("the generator is a point");
    assert_eq!(x.to_bytes(), gx.to_bytes());
    assert_eq!(y.to_bytes(), gy.neg().to_bytes());

    // A point added to its own negation is the neutral element.
    let negated = Point::from_affine(gx, gy.neg()).expect("the negation is a point");
    assert!(g.add(negated).is_identity());
}

#[test]
fn a_pair_that_is_not_on_the_curve_is_not_a_point() {
    let (x, y) = Point::generator()
        .to_affine()
        .expect("the generator is a point");
    assert!(Point::from_affine(x, y).is_some());
    assert!(Point::from_affine(x, y.add(Element::<Prime>::one())).is_none());
    assert!(Point::from_affine(x.add(Element::<Prime>::one()), y).is_none());
}

#[test]
fn the_rfc_6979_vectors_sign_and_verify_as_documented() {
    let secret = unhex32(SECRET);
    assert_eq!(
        hex(&public_key(&secret).expect("the secret is in range")),
        PUBLIC.replace(['\n', ' '], "")
    );

    let cases = [
        (
            "af2bdbe1aa9b6ec1e2ade1d694f41fc71a831d0268e9891562113d8a62add1bf",
            "efd48b2aacb6a8fd1140dd9cd45e81d69d2c877b56aaf991c34d0ea84eaf3716",
            "f7cb1c942d657c41d436c7a1b6e29f65f3e900dbb9aff4064dc4ab2f843acda8",
        ),
        (
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            "f1abb023518351cd71d881567b1ea663ed3efcf6c5132b354f28d3b0b7d38367",
            "019f4113742a2b14bd25926b49c649155f267e60d3814b4c0cc84250e46f0083",
        ),
    ];
    for (digest, expected_r, expected_s) in cases {
        let digest = unhex32(digest);
        let (r, s) = sign(&secret, &digest).expect("the secret is in range");
        assert_eq!(hex(&r), expected_r, "r of {digest:?}");
        assert_eq!(hex(&s), expected_s, "s of {digest:?}");
        assert_eq!(key().verify(&digest, &r, &s), Ok(()));
    }
}

#[test]
fn a_signature_over_another_digest_does_not_verify() {
    let secret = unhex32(SECRET);
    let digest = unhex32("af2bdbe1aa9b6ec1e2ade1d694f41fc71a831d0268e9891562113d8a62add1bf");
    let (r, s) = sign(&secret, &digest).expect("the secret is in range");

    let other = unhex32("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08");
    assert_eq!(key().verify(&other, &r, &s), Err(EcError::BadSignature));

    let mut damaged = r;
    damaged[31] ^= 0x01;
    assert_eq!(
        key().verify(&digest, &damaged, &s),
        Err(EcError::BadSignature)
    );

    let mut damaged = s;
    damaged[31] ^= 0x01;
    assert_eq!(
        key().verify(&digest, &r, &damaged),
        Err(EcError::BadSignature)
    );
}

#[test]
fn a_component_that_is_zero_or_at_the_order_is_refused() {
    let digest = unhex32("af2bdbe1aa9b6ec1e2ade1d694f41fc71a831d0268e9891562113d8a62add1bf");
    let (r, s) = sign(&unhex32(SECRET), &digest).expect("the secret is in range");
    let zero = [0u8; 32];
    let order = unhex32(ORDER_BYTES);
    let mut above = order;
    above[31] = above[31].wrapping_add(1);

    for (name, bad_r, bad_s) in [
        ("r zero", zero, s),
        ("s zero", r, zero),
        ("r at the order", order, s),
        ("s at the order", r, order),
        ("r above the order", above, s),
        ("s above the order", r, above),
    ] {
        assert_eq!(
            key().verify(&digest, &bad_r, &bad_s),
            Err(EcError::InvalidScalar),
            "{name}"
        );
    }
}

#[test]
fn a_key_that_is_not_an_uncompressed_point_is_refused() {
    let valid = unhex(PUBLIC);

    let mut wrong_prefix = valid.clone();
    wrong_prefix[0] = 0x02;
    assert_eq!(
        crate::p256::PublicKey::from_sec1(&wrong_prefix).err(),
        Some(EcError::InvalidPoint)
    );

    assert_eq!(
        crate::p256::PublicKey::from_sec1(&valid[..64]).err(),
        Some(EcError::InvalidPoint),
        "too short"
    );
    assert_eq!(
        crate::p256::PublicKey::from_sec1(&[]).err(),
        Some(EcError::InvalidPoint),
        "empty"
    );

    let mut off_curve = valid.clone();
    off_curve[64] ^= 0x01;
    assert_eq!(
        crate::p256::PublicKey::from_sec1(&off_curve).err(),
        Some(EcError::InvalidPoint),
        "not on the curve"
    );

    let mut coordinate_at_the_prime = valid;
    for (slot, byte) in coordinate_at_the_prime.iter_mut().skip(1).zip(unhex32(
        "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff",
    )) {
        *slot = byte;
    }
    assert_eq!(
        crate::p256::PublicKey::from_sec1(&coordinate_at_the_prime).err(),
        Some(EcError::InvalidPoint),
        "coordinate at the prime"
    );
}

#[test]
fn a_longer_digest_is_truncated_to_the_width_of_the_order() {
    let secret = unhex32(SECRET);
    let long = unhex(
        "9a9083505bc92276aec4be312696ef7bf3bf603f4bbd381196a029f340585312\
         313bca4a9b5b890efee42c77b1ee25fe",
    );
    let mut leftmost = [0u8; 32];
    for (slot, byte) in leftmost.iter_mut().zip(long.iter()) {
        *slot = *byte;
    }

    let (r, s) = sign(&secret, &leftmost).expect("the secret is in range");
    assert_eq!(key().verify(&long, &r, &s), Ok(()), "the tail is ignored");

    let mut changed_tail = long.clone();
    if let Some(last) = changed_tail.last_mut() {
        *last ^= 0xFF;
    }
    assert_eq!(key().verify(&changed_tail, &r, &s), Ok(()));

    let mut changed_head = long;
    changed_head[0] ^= 0x01;
    assert_eq!(
        key().verify(&changed_head, &r, &s),
        Err(EcError::BadSignature)
    );
}

#[test]
fn a_secret_outside_the_range_is_refused() {
    assert_eq!(public_key(&[0u8; 32]), Err(EcError::InvalidScalar));
    assert_eq!(
        public_key(&unhex32(ORDER_BYTES)),
        Err(EcError::InvalidScalar)
    );
    assert_eq!(sign(&[0u8; 32], &[7u8; 32]), Err(EcError::InvalidScalar));
}
