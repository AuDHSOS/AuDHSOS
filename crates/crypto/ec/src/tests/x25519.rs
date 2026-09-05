// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! X25519 against the vectors of RFC 7748, sections 5.2 and 6.1.

use crate::error::EcError;
use crate::tests::{hex, unhex32};
use crate::x25519::{base_point, x25519};

#[test]
fn the_two_vectors_of_the_rfc_multiply_as_documented() {
    let scalar = unhex32("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4");
    let point = unhex32("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c");
    assert_eq!(
        hex(&x25519(&scalar, &point).expect("the vector is a valid exchange")),
        "c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552"
    );

    let scalar = unhex32("4b66e9d4d1b4673c5ad22691957d6af5c11b6421e0ea01d42ca4169e7918ba0d");
    let point = unhex32("e5210f12786811d3f4b7959d0538ae2c31dbe7106fc03c3efc4cd549c715a493");
    assert_eq!(
        hex(&x25519(&scalar, &point).expect("the vector is a valid exchange")),
        "95cbde9476e8907d7aade45cb4b873f88b595a68799fa152e6f8f7647aac7957"
    );
}

#[test]
fn the_exchange_of_the_rfc_reaches_the_documented_secret() {
    let alice = unhex32("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
    let bob = unhex32("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");

    let alice_public = base_point(&alice);
    let bob_public = base_point(&bob);
    assert_eq!(
        hex(&alice_public),
        "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"
    );
    assert_eq!(
        hex(&bob_public),
        "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"
    );

    let from_alice = x25519(&alice, &bob_public).expect("the exchange is valid");
    let from_bob = x25519(&bob, &alice_public).expect("the exchange is valid");
    assert_eq!(from_alice, from_bob);
    assert_eq!(
        hex(&from_alice),
        "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742"
    );
}

#[test]
fn the_iterated_vector_agrees_after_one_and_after_a_thousand_rounds() {
    let mut scalar = [0u8; 32];
    if let Some(slot) = scalar.first_mut() {
        *slot = 9;
    }
    let mut point = scalar;

    for round in 1..=1000u32 {
        let next = x25519(&scalar, &point).expect("no round reaches a small-order point");
        point = scalar;
        scalar = next;
        if round == 1 {
            assert_eq!(
                hex(&scalar),
                "422c8e7a6227d7bca1350b3e2bb7279f7897b87bb6854b783c60e80311ae3079"
            );
        }
    }
    assert_eq!(
        hex(&scalar),
        "684cf59ba83309552800ef566f2f4d3c1c3887c49360e3875f2eb94d99532c51"
    );
}

#[test]
fn a_point_of_small_order_is_refused_rather_than_returned() {
    let scalar = unhex32("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4");
    let prime = unhex32("edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f");
    let mut above = prime;
    above[0] = above[0].wrapping_add(1);

    for (name, point) in [
        ("zero", [0u8; 32]),
        (
            "one",
            unhex32("0100000000000000000000000000000000000000000000000000000000000000"),
        ),
        ("the prime", prime),
        ("one above the prime", above),
    ] {
        assert_eq!(
            x25519(&scalar, &point),
            Err(EcError::ZeroSharedSecret),
            "peer value {name}"
        );
    }
}

#[test]
fn the_highest_bit_of_a_peer_value_is_ignored() {
    let scalar = unhex32("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4");
    let point = unhex32("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c");
    let mut with_bit = point;
    with_bit[31] |= 0x80;
    assert_eq!(x25519(&scalar, &point), x25519(&scalar, &with_bit));
}

#[test]
fn the_scalar_is_clamped_so_that_neighbours_agree() {
    let point = unhex32("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c");
    let scalar = unhex32("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4");

    // The three bits the clamping clears at the bottom and the two it
    // fixes at the top cannot change the result.
    let mut altered = scalar;
    altered[0] |= 0x07;
    altered[31] &= 0x7F;
    altered[31] |= 0x40;
    assert_eq!(x25519(&scalar, &point), x25519(&altered, &point));

    let mut high = scalar;
    high[31] |= 0x80;
    assert_eq!(x25519(&scalar, &point), x25519(&high, &point));
}

#[test]
fn the_errors_render_a_message() {
    for error in [
        EcError::ZeroSharedSecret,
        EcError::InvalidPoint,
        EcError::InvalidScalar,
        EcError::BadSignature,
    ] {
        assert!(!format!("{error}").is_empty());
        assert!(!format!("{error:?}").is_empty());
    }
}
