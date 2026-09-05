// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! P-384: the two moduli, the group, and ECDSA.
//!
//! The vectors come from the two documents kept under `docs/rfc/`. The
//! group is checked against RFC 5903, appendix 8.2, whose two key pairs
//! and shared point are three multiplications this file did not compute.
//! The signatures are RFC 6979, appendix A.2.6, which is the table one
//! curve up from the A.2.5 the P-256 file reads.

use crate::error::EcError;
use crate::p384::field::{Element, Order, Prime};
use crate::p384::point::Point;
use crate::p384::{public_key, sign};
use crate::tests::{hex, unhex, unhex48};

/// The field prime, `2^384 - 2^128 - 2^96 + 2^32 - 1`.
const PRIME_BYTES: &str = "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe\
                           ffffffff0000000000000000ffffffff";
/// The group order.
const ORDER_BYTES: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf\
                           581a0db248b0a77aecec196accc52973";

/// A small value as an element of the field.
fn small(value: u8) -> Element<Prime> {
    let mut bytes = [0u8; 48];
    bytes[47] = value;
    Element::from_canonical(&bytes).expect("a small value is below the modulus")
}

#[test]
fn the_field_multiplies_and_encodes() {
    assert_eq!(hex(&small(0).to_bytes()), hex(&[0u8; 48]));
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
    let bytes = unhex48(GENERATOR_X);
    let element = Element::<Prime>::from_canonical(&bytes).expect("below the modulus");
    assert_eq!(element.to_bytes(), bytes);

    let scalar = Element::<Order>::from_canonical(&bytes).expect("below the order");
    assert_eq!(scalar.to_bytes(), bytes);
}

#[test]
fn a_value_at_or_above_the_modulus_is_refused_but_can_be_reduced() {
    let prime = unhex48(PRIME_BYTES);
    assert!(Element::<Prime>::from_canonical(&prime).is_none());
    assert!(Element::<Prime>::from_bytes_reduced(&prime).is_zero());

    // The prime ends in `ff`, so one more than it is written out rather
    // than reached by adding to the last byte.
    let above = unhex48(
        "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe\
         ffffffff000000000000000100000000",
    );
    assert!(Element::<Prime>::from_canonical(&above).is_none());
    assert_eq!(Element::<Prime>::from_bytes_reduced(&above), small(1));

    let order = unhex48(ORDER_BYTES);
    assert!(Element::<Order>::from_canonical(&order).is_none());
    assert!(Element::<Order>::from_bytes_reduced(&order).is_zero());

    let mut above_order = order;
    above_order[47] = above_order[47].wrapping_add(1);
    assert!(Element::<Order>::from_canonical(&above_order).is_none());
    assert_eq!(
        hex(&Element::<Order>::from_bytes_reduced(&above_order).to_bytes()),
        hex(&unhex48(
            "0000000000000000000000000000000000000000000000000000000000000000\
             00000000000000000000000000000001"
        ))
    );

    let largest = [0xFFu8; 48];
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

    let bytes = unhex48(SECRET);
    let scalar = Element::<Order>::from_canonical(&bytes).expect("below the order");
    assert_eq!(scalar.mul(scalar.invert()), Element::<Order>::one());
    assert_eq!(scalar.neg().add(scalar), Element::<Order>::zero());
}

/// The base point of RFC 5903, section 3.2.
const GENERATOR_X: &str = "aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a38\
                           5502f25dbf55296c3a545e3872760ab7";
/// The base point of RFC 5903, section 3.2.
const GENERATOR_Y: &str = "3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c0\
                           0a60b1ce1d7e819d7a431d7c90ea0e5f";

/// The initiator's private value of RFC 5903, section 8.2.
const ECDH_I: &str = "099f3c7034d4a2c699884d73a375a67f7624ef7c6b3c0f160647b67414dce655\
                      e35b538041e649ee3faef896783ab194";
/// Its public value, `g^i`.
const ECDH_GIX: &str = "667842d7d180ac2cde6f74f37551f55755c7645c20ef73e31634fe72b4c55ee6\
                        de3ac808acb4bdb4c88732aee95f41aa";
/// Its public value, `g^i`.
const ECDH_GIY: &str = "9482ed1fc0eeb9cafc4984625ccfc23f65032149e0e144ada024181535a0f38e\
                        eb9fcff3c2c947dae69b4c634573a81c";
/// The responder's private value.
const ECDH_R: &str = "41cb0779b4bdb85d47846725fbec3c9430fab46cc8dc5060855cc9bda0aa2942\
                      e0308312916b8ed2960e4bd55a7448fc";
/// Its public value, `g^r`.
const ECDH_GRX: &str = "e558dbef53eecde3d3fccfc1aea08a89a987475d12fd950d83cfa41732bc509d\
                        0d1ac43a0336def96fda41d0774a3571";
/// Its public value, `g^r`.
const ECDH_GRY: &str = "dcfbec7aacf3196472169e838430367f66eebe3c6e70c416dd5f0c68759dd1ff\
                        f83fa40142209dff5eaad96db9e6386c";
/// The value both sides arrive at, `g^(ir)`.
const ECDH_GIRX: &str = "11187331c279962d93d604243fd592cb9d0a926f422e47187521287e7156c5c4\
                         d603135569b9e9d09cf5d4a270f59746";
/// The value both sides arrive at, `g^(ir)`.
const ECDH_GIRY: &str = "a2a9f38ef5cafbe2347cf7ec24bdd5e624bc93bfa82771f40d1b65d06256a852\
                         c983135d4669f8792f2c1d55718afbb4";

/// The affine coordinates of a point, for comparing points that may carry
/// different denominators.
fn affine(point: Point) -> Option<([u8; 48], [u8; 48])> {
    point.to_affine().map(|(x, y)| (x.to_bytes(), y.to_bytes()))
}

#[test]
fn the_generator_and_its_multiples_are_the_documented_points() {
    let (x, y) = Point::generator()
        .to_affine()
        .expect("the generator is not the neutral element");
    assert_eq!(hex(&x.to_bytes()), GENERATOR_X.replace(['\n', ' '], ""));
    assert_eq!(hex(&y.to_bytes()), GENERATOR_Y.replace(['\n', ' '], ""));

    // RFC 5903, section 8.2: two private values and the public values they
    // produce. Neither multiplication is one this file computed.
    let gi = Point::generator().mul(&unhex48(ECDH_I));
    assert_eq!(
        affine(gi),
        Some((unhex48(ECDH_GIX), unhex48(ECDH_GIY))),
        "g^i"
    );
    let gr = Point::generator().mul(&unhex48(ECDH_R));
    assert_eq!(
        affine(gr),
        Some((unhex48(ECDH_GRX), unhex48(ECDH_GRY))),
        "g^r"
    );

    // And the point both sides reach, which multiplies a point that is not
    // the generator, from either side.
    let expected = Some((unhex48(ECDH_GIRX), unhex48(ECDH_GIRY)));
    assert_eq!(affine(gi.mul(&unhex48(ECDH_R))), expected, "(g^i)^r");
    assert_eq!(affine(gr.mul(&unhex48(ECDH_I))), expected, "(g^r)^i");
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

    // Doubling and addition agree, and both agree with the multiplication.
    let mut two = [0u8; 48];
    two[47] = 2;
    assert_eq!(affine(g.double()), affine(g.add(g)));
    assert_eq!(affine(g.mul(&two)), affine(g.double()));

    // The multiple by the order is the neutral element, and the multiple
    // by one less is the negation of the generator.
    let order = unhex48(ORDER_BYTES);
    assert!(g.mul(&order).is_identity());

    let mut one_less = order;
    one_less[47] = one_less[47].wrapping_sub(1);
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

/// The secret of RFC 6979, appendix A.2.6.
const SECRET: &str = "6b9d3dad2e1b8c1c05b19875b6659f4de23c3b667bf297ba9aa47740787137d8\
                      96d5724e4c70a825f872c9ea60d2edf5";
/// Its public key, uncompressed.
const PUBLIC: &str = "04ec3a4e415b4e19a4568618029f427fa5da9a8bc4ae92e02e06aae5286b300c64\
                      def8f0ea9055866064a254515480bc13\
                      8015d9b72d7d57244ea8ef9ac0c621896708a59367f9dfb9f54ca84b3f1c9db1\
                      288b231c3ae0d4fe7344fd2533264720";

/// The public key of the appendix as a parsed key.
fn key() -> crate::p384::PublicKey {
    crate::p384::PublicKey::from_sec1(&unhex(PUBLIC)).expect("the appendix key is a point")
}

#[test]
fn the_rfc_6979_vectors_sign_and_verify_as_documented() {
    let secret = unhex48(SECRET);
    assert_eq!(
        hex(&public_key(&secret).expect("the secret is in range")),
        PUBLIC.replace(['\n', ' '], "")
    );

    // The rows of appendix A.2.6 whose hash is SHA-384, which is the hash
    // the generator of RFC 6979 uses here and the one this curve is paired
    // with in a certificate. The digests are of the messages the appendix
    // names, "sample" and "test".
    let cases = [
        (
            "9a9083505bc92276aec4be312696ef7bf3bf603f4bbd381196a029f340585312\
             313bca4a9b5b890efee42c77b1ee25fe",
            "94edbb92a5ecb8aad4736e56c691916b3f88140666ce9fa73d64c4ea95ad133c\
             81a648152e44acf96e36dd1e80fabe46",
            "99ef4aeb15f178cea1fe40db2603138f130e740a19624526203b6351d0a3a94f\
             a329c145786e679e7b82c71a38628ac8",
        ),
        (
            "768412320f7b0aa5812fce428dc4706b3cae50e02a64caa16a782249bfe8efc4\
             b7ef1ccb126255d196047dfedf17a0a9",
            "8203b63d3c853e8d77227fb377bcf7b7b772e97892a80f36ab775d509d7a5feb\
             0542a7f0812998da8f1dd3ca3cf023db",
            "ddd0760448d42d8a43af45af836fce4de8be06b485e9b61b827c2f13173923e0\
             6a739f040649a667bf3b828246baa5a5",
        ),
    ];
    for (digest, expected_r, expected_s) in cases {
        let digest = unhex48(digest);
        let (r, s) = sign(&secret, &digest).expect("the secret is in range");
        assert_eq!(hex(&r), expected_r.replace(['\n', ' '], ""), "r");
        assert_eq!(hex(&s), expected_s.replace(['\n', ' '], ""), "s");
        assert_eq!(key().verify(&digest, &r, &s), Ok(()));
    }
}

#[test]
fn a_digest_narrower_than_the_order_verifies_as_documented() {
    // The SHA-256 rows of the same appendix. This crate's signer uses the
    // hash the curve is paired with, so it does not produce these; the
    // point of the rows here is that a thirty-two byte digest verifies
    // against a forty-eight byte order, which is the case a certificate
    // signed `ecdsa-with-SHA256` under a P-384 key presents.
    let cases = [
        (
            "af2bdbe1aa9b6ec1e2ade1d694f41fc71a831d0268e9891562113d8a62add1bf",
            "21b13d1e013c7fa1392d03c5f99af8b30c570c6f98d4ea8e354b63a21d3daa33\
             bde1e888e63355d92fa2b3c36d8fb2cd",
            "f3aa443fb107745bf4bd77cb3891674632068a10ca67e3d45db2266fa7d1feeb\
             efdc63eccd1ac42ec0cb8668a4fa0ab0",
        ),
        (
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            "6d6defac9ab64dabafe36c6bf510352a4cc27001263638e5b16d9bb51d451559\
             f918eedaf2293be5b475cc8f0188636b",
            "2d46f3becbcc523d5f1a1256bf0c9b024d879ba9e838144c8ba6baeb4b53b47d\
             51ab373f9845c0514eefb14024787265",
        ),
    ];
    for (digest, r, s) in cases {
        let digest = unhex(digest);
        let r = unhex48(&r.replace(['\n', ' '], ""));
        let s = unhex48(&s.replace(['\n', ' '], ""));
        assert_eq!(digest.len(), 32);
        assert_eq!(key().verify(&digest, &r, &s), Ok(()));
    }
}

#[test]
fn a_signature_over_another_digest_does_not_verify() {
    let secret = unhex48(SECRET);
    let digest = unhex48(
        "9a9083505bc92276aec4be312696ef7bf3bf603f4bbd381196a029f340585312\
         313bca4a9b5b890efee42c77b1ee25fe",
    );
    let (r, s) = sign(&secret, &digest).expect("the secret is in range");

    let other = unhex48(
        "768412320f7b0aa5812fce428dc4706b3cae50e02a64caa16a782249bfe8efc4\
         b7ef1ccb126255d196047dfedf17a0a9",
    );
    assert_eq!(key().verify(&other, &r, &s), Err(EcError::BadSignature));

    let mut damaged = r;
    damaged[47] ^= 0x01;
    assert_eq!(
        key().verify(&digest, &damaged, &s),
        Err(EcError::BadSignature)
    );

    let mut damaged = s;
    damaged[47] ^= 0x01;
    assert_eq!(
        key().verify(&digest, &r, &damaged),
        Err(EcError::BadSignature)
    );
}

#[test]
fn a_component_that_is_zero_or_at_the_order_is_refused() {
    let digest = unhex48(
        "9a9083505bc92276aec4be312696ef7bf3bf603f4bbd381196a029f340585312\
         313bca4a9b5b890efee42c77b1ee25fe",
    );
    let (r, s) = sign(&unhex48(SECRET), &digest).expect("the secret is in range");
    let zero = [0u8; 48];
    let order = unhex48(ORDER_BYTES);
    let mut above = order;
    above[47] = above[47].wrapping_add(1);

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
    assert_eq!(valid.len(), 97);

    let mut wrong_prefix = valid.clone();
    wrong_prefix[0] = 0x02;
    assert_eq!(
        crate::p384::PublicKey::from_sec1(&wrong_prefix).err(),
        Some(EcError::InvalidPoint)
    );

    assert_eq!(
        crate::p384::PublicKey::from_sec1(&valid[..96]).err(),
        Some(EcError::InvalidPoint),
        "too short"
    );
    assert_eq!(
        crate::p384::PublicKey::from_sec1(&[]).err(),
        Some(EcError::InvalidPoint),
        "empty"
    );

    let mut off_curve = valid.clone();
    off_curve[96] ^= 0x01;
    assert_eq!(
        crate::p384::PublicKey::from_sec1(&off_curve).err(),
        Some(EcError::InvalidPoint),
        "not on the curve"
    );

    let mut coordinate_at_the_prime = valid;
    for (slot, byte) in coordinate_at_the_prime
        .iter_mut()
        .skip(1)
        .zip(unhex48(PRIME_BYTES))
    {
        *slot = byte;
    }
    assert_eq!(
        crate::p384::PublicKey::from_sec1(&coordinate_at_the_prime).err(),
        Some(EcError::InvalidPoint),
        "coordinate at the prime"
    );
}

#[test]
fn a_secret_outside_the_range_is_refused() {
    assert_eq!(public_key(&[0u8; 48]), Err(EcError::InvalidScalar));
    assert_eq!(
        public_key(&unhex48(ORDER_BYTES)),
        Err(EcError::InvalidScalar)
    );
    assert_eq!(sign(&[0u8; 48], &[7u8; 48]), Err(EcError::InvalidScalar));
}
