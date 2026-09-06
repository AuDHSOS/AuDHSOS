// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What `PublicKey::new` accepts, and what it deliberately does not
//! judge.
//!
//! The bounds here are the primitive's own (D-79): the modulus odd with
//! its top bit set and no wider than the arithmetic allows, the exponent
//! odd and at least three. The lower bound on the key size is not among
//! them, and the last test of this file is what says so.

use crypto_bignum::MAX_BYTES;

use crate::error::RsaError;
use crate::key::PublicKey;
use crate::tests::{fixed_modulus, rfc8448_key, unhex};

#[test]
fn an_exponent_of_one_or_two_is_refused_and_three_is_taken() {
    let modulus = fixed_modulus(2048);
    assert_eq!(
        PublicKey::new(&modulus, &[1]),
        Err(RsaError::InvalidExponent)
    );
    assert_eq!(
        PublicKey::new(&modulus, &[2]),
        Err(RsaError::InvalidExponent)
    );
    assert_eq!(
        PublicKey::new(&modulus, &[4]),
        Err(RsaError::InvalidExponent)
    );
    let key = PublicKey::new(&modulus, &[3]).expect("three is the smallest usable exponent");
    assert_eq!(key.exponent(), 3);
}

#[test]
fn an_exponent_that_does_not_fit_in_sixty_four_bits_is_refused() {
    let modulus = fixed_modulus(2048);
    let wide = [0x01u8, 0, 0, 0, 0, 0, 0, 0, 1];
    assert_eq!(
        PublicKey::new(&modulus, &wide),
        Err(RsaError::InvalidExponent)
    );
    let widest = [0xffu8; 8];
    let key = PublicKey::new(&modulus, &widest).expect("the widest odd exponent still fits");
    assert_eq!(key.exponent(), u64::MAX);
}

#[test]
fn a_modulus_without_its_top_bit_set_is_refused() {
    let mut modulus = fixed_modulus(2048);
    if let Some(first) = modulus.first_mut() {
        *first &= 0x7f;
    }
    assert_eq!(
        PublicKey::new(&modulus, &unhex("010001")),
        Err(RsaError::InvalidModulus)
    );
}

#[test]
fn an_even_modulus_is_refused() {
    let mut modulus = fixed_modulus(2048);
    if let Some(last) = modulus.last_mut() {
        *last &= 0xfe;
    }
    assert_eq!(
        PublicKey::new(&modulus, &unhex("010001")),
        Err(RsaError::InvalidModulus)
    );
}

#[test]
fn a_modulus_wider_than_the_arithmetic_is_refused_and_the_widest_one_is_taken() {
    let widest = fixed_modulus(4096);
    assert_eq!(widest.len(), MAX_BYTES);
    let key = PublicKey::new(&widest, &unhex("010001")).expect("four thousand and ninety-six bits");
    assert_eq!(key.size(), MAX_BYTES);
    assert_eq!(key.bits(), 4096);

    let mut wider = vec![0x81u8];
    wider.extend_from_slice(&widest);
    assert_eq!(
        PublicKey::new(&wider, &unhex("010001")),
        Err(RsaError::InvalidModulus)
    );
}

#[test]
fn the_size_of_a_key_is_the_width_of_its_modulus() {
    for bits in [1024usize, 2048, 3072, 4096] {
        let key = PublicKey::new(&fixed_modulus(bits), &unhex("010001"))
            .expect("the fixed modulus is well formed");
        assert_eq!(key.size(), bits / 8);
        assert_eq!(key.bits(), bits);
    }
}

/// The point of D-79: the lower bound on the key size is a certificate
/// rule, so the primitive takes a key that no chain would ever carry.
#[test]
fn the_thousand_and_twenty_four_bit_key_of_rfc_8448_is_accepted_here() {
    let key = rfc8448_key();
    assert_eq!(key.bits(), 1024);
    assert_eq!(key.size(), 128);
    assert_eq!(key.exponent(), 65537);
}
