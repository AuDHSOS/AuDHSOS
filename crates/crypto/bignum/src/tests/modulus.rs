// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The run-time modulus: what it refuses, what it derives, and the
//! exponentiation over it.
//!
//! The key of the round trip is the one RFC 8448, section 2 prints, which
//! is a vector rather than a fixture: the traces that document carries
//! were made with it. It is a thousand and twenty-four bits, below what a
//! certificate may carry, so it exercises the arithmetic and no chain.

use test_support::generators::{bytes, vec};

use crate::error::BignumError;
use crate::modulus::{MAX_BYTES, Modulus};
use crate::tests::limbs::{be_of, limbs_of};
use crate::tests::reference::Big;
use crate::tests::{WIDTHS, check_cases, hex, modulus_bytes, unhex};

/// The modulus of the RSA key of RFC 8448, section 2.
const RFC8448_MODULUS: &str = "\
    b4bb498f8279303d980836399b36c6988c0c68de55e1bdb826d3901a2461eafd\
    2de49a91d015abbc9a95137ace6c1af19eaa6af98c7ced43120998e187a80ee0\
    ccb0524b1b018c3e0b63264d449a6d38e22a5fda430846748030530ef0461c8c\
    a9d9efbfae8ea6d1d03e2bd193eff0ab9a8002c47428a6d35a8d88d79f7f1e3f";

/// The private exponent of that key.
const RFC8448_PRIVATE: &str = "\
    04dea705d43a6ea7209dd8072111a83c81e322a59278b33480641eaf7c0a6985\
    b8e31c44f6de62e1b4c2309f6126e77b7c41e923314bbfa3881305dc1217f16c\
    819ce538e922f369828d0e57195d8c8488460207b2faa726bcf708bbd7db7f67\
    9f893492fc2a622e08970aac441ce4e0c3088df25ae679233df8a3bda2ff9941";

/// A modulus of `bits` bits with a fixed body, for the tests that want
/// one value rather than a generated family.
fn fixed(bits: usize) -> Vec<u8> {
    let source: Vec<u8> = (0..bits / 8)
        .map(|index| u8::try_from(index.wrapping_mul(37).wrapping_add(11) % 251).unwrap_or(1))
        .collect();
    modulus_bytes(bits, &source)
}

#[test]
fn the_modulus_refuses_a_value_that_is_even() {
    let mut bytes = fixed(1024);
    if let Some(last) = bytes.last_mut() {
        *last &= 0xfe;
    }
    assert_eq!(Modulus::new(&bytes), Err(BignumError::Even));
}

#[test]
fn the_modulus_refuses_zero() {
    assert_eq!(Modulus::new(&[]), Err(BignumError::NotNormalized));
    assert_eq!(Modulus::new(&[0u8; 16]), Err(BignumError::NotNormalized));
}

#[test]
fn the_modulus_refuses_a_value_whose_top_limb_is_zero() {
    let mut bytes = vec![0u8; 9];
    for slot in bytes.iter_mut().skip(1) {
        *slot = 0xff;
    }
    assert_eq!(Modulus::new(&bytes), Err(BignumError::NotNormalized));
}

#[test]
fn the_modulus_refuses_a_value_wider_than_the_arithmetic() {
    let bytes = fixed(4096);
    assert!(Modulus::new(&bytes).is_ok());
    let mut wider = vec![0x81u8];
    wider.extend_from_slice(&bytes);
    assert_eq!(wider.len(), MAX_BYTES + 1);
    assert_eq!(Modulus::new(&wider), Err(BignumError::TooWide));
}

#[test]
fn an_accepted_modulus_is_zero_above_the_limbs_it_uses() {
    for bits in WIDTHS {
        let modulus = Modulus::new(&fixed(bits)).expect("the fixed modulus is well formed");
        assert_eq!(modulus.used, bits / 64);
        assert_eq!(modulus.bits(), bits);
        for (index, limb) in modulus.limbs.iter().enumerate().skip(modulus.used) {
            assert_eq!(*limb, 0, "modulus limb {index} above the used count");
        }
        for (index, limb) in modulus.r2.iter().enumerate().skip(modulus.used) {
            assert_eq!(*limb, 0, "conversion limb {index} above the used count");
        }
    }
}

#[test]
fn the_derived_inverse_is_the_negative_inverse_of_the_low_limb() {
    for bits in WIDTHS {
        let modulus = Modulus::new(&fixed(bits)).expect("the fixed modulus is well formed");
        let low = modulus.limbs.first().copied().unwrap_or(0);
        assert_eq!(
            low.wrapping_mul(modulus.n0inv).wrapping_add(1),
            0,
            "at {bits} bits the derived inverse is not the negative inverse"
        );
    }
}

#[test]
fn the_derived_conversion_constant_is_two_to_the_double_width() {
    for bits in WIDTHS {
        let encoded = fixed(bits);
        let modulus = Modulus::new(&encoded).expect("the fixed modulus is well formed");
        let reference = Big::from_be_bytes(&encoded);
        let expected = Big::from_u64(1)
            .shl(modulus.used.saturating_mul(128))
            .rem(&reference);
        let ours = Big::from_be_bytes(&be_of(&modulus.r2[..modulus.used]));
        assert_eq!(
            ours,
            expected,
            "at {bits} bits: {} against {}",
            hex(&ours.to_be_bytes(bits / 8)),
            hex(&expected.to_be_bytes(bits / 8))
        );
    }
}

#[test]
fn property_exponentiation_agrees_with_the_reference() {
    for bits in WIDTHS {
        let width = bits / 8;
        let name = format!("bignum_pow_{bits}");
        let generator = vec(bytes(width..=width), 2..=2);
        let cases = if bits <= 2048 { 12 } else { 4 };
        check_cases(cases, &name, &generator, |parts| {
            let first = parts.first().ok_or_else(|| "no modulus".to_owned())?;
            let second = parts.get(1).ok_or_else(|| "no base".to_owned())?;
            let encoded = modulus_bytes(bits, first);
            let modulus = Modulus::new(&encoded).map_err(|error| error.to_string())?;
            let reference = Big::from_be_bytes(&encoded);
            let base = Big::from_be_bytes(second).rem(&reference);
            let base_bytes = base.to_be_bytes(width);

            for exponent in [1u64, 3, 65537] {
                let mut ours = vec![0u8; width];
                modulus
                    .pow(&base_bytes, exponent, &mut ours)
                    .map_err(|error| error.to_string())?;
                let theirs = base
                    .pow_mod(&exponent.to_be_bytes(), &reference)
                    .to_be_bytes(width);
                if ours != theirs {
                    return Err(format!(
                        "exponent {exponent}: {} against {}",
                        hex(&ours),
                        hex(&theirs)
                    ));
                }
            }
            Ok(())
        });
    }
}

#[test]
fn exponentiation_takes_an_exponent_with_its_top_and_bottom_bits_set() {
    let encoded = fixed(1024);
    let modulus = Modulus::new(&encoded).expect("the fixed modulus is well formed");
    let reference = Big::from_be_bytes(&encoded);
    let base = Big::from_be_bytes(&fixed(1016)).rem(&reference);
    let base_bytes = base.to_be_bytes(128);
    let exponent = 0x8000_0000_0000_0001u64;

    let mut ours = vec![0u8; 128];
    modulus
        .pow(&base_bytes, exponent, &mut ours)
        .expect("the base is below the modulus");
    let theirs = base
        .pow_mod(&exponent.to_be_bytes(), &reference)
        .to_be_bytes(128);
    assert_eq!(hex(&ours), hex(&theirs));
}

#[test]
fn an_exponent_of_zero_gives_one() {
    let modulus = Modulus::new(&fixed(1024)).expect("the fixed modulus is well formed");
    let base = fixed(1016);
    let mut out = vec![0u8; 128];
    modulus
        .pow(&base, 0, &mut out)
        .expect("the base is below the modulus");
    let mut expected = vec![0u8; 128];
    if let Some(last) = expected.last_mut() {
        *last = 1;
    }
    assert_eq!(out, expected);
}

#[test]
fn exponentiation_refuses_a_base_that_is_not_below_the_modulus() {
    let encoded = fixed(1024);
    let modulus = Modulus::new(&encoded).expect("the fixed modulus is well formed");
    let mut out = vec![0u8; 128];
    assert_eq!(
        modulus.pow(&encoded, 3, &mut out),
        Err(BignumError::OutOfRange)
    );
}

#[test]
fn exponentiation_refuses_a_base_wider_than_the_arithmetic() {
    let modulus = Modulus::new(&fixed(1024)).expect("the fixed modulus is well formed");
    let mut out = vec![0u8; 128];
    assert_eq!(
        modulus.pow(&[1u8; MAX_BYTES + 1], 3, &mut out),
        Err(BignumError::TooWide)
    );
}

#[test]
fn exponentiation_refuses_an_output_that_is_too_short() {
    let modulus = Modulus::new(&fixed(1024)).expect("the fixed modulus is well formed");
    let base = fixed(1016);
    let mut out = vec![0u8; 8];
    assert_eq!(
        modulus.pow(&base, 3, &mut out),
        Err(BignumError::OutputTooShort)
    );
}

#[test]
fn exponentiation_pads_a_longer_output_with_leading_zeros() {
    let modulus = Modulus::new(&fixed(1024)).expect("the fixed modulus is well formed");
    let base = fixed(1016);
    let mut narrow = vec![0u8; 128];
    let mut wide = vec![0u8; 160];
    modulus
        .pow(&base, 3, &mut narrow)
        .expect("the base is below the modulus");
    modulus
        .pow(&base, 3, &mut wide)
        .expect("the base is below the modulus");
    assert_eq!(&wide[..32], &[0u8; 32]);
    assert_eq!(&wide[32..], &narrow[..]);
}

#[test]
fn the_limbs_of_a_modulus_read_back_as_the_bytes_that_made_it() {
    for bits in WIDTHS {
        let encoded = fixed(bits);
        let modulus = Modulus::new(&encoded).expect("the fixed modulus is well formed");
        let reference = Big::from_be_bytes(&encoded);
        assert_eq!(limbs_of(&reference, modulus.used), modulus.limbs);
    }
}

#[test]
fn the_rfc_8448_key_signs_with_the_wide_exponent_and_verifies_with_the_small_one() {
    let n = unhex(RFC8448_MODULUS);
    let d = unhex(RFC8448_PRIVATE);
    assert_eq!(n.len(), 128);
    assert_eq!(d.len(), 128);
    let modulus = Modulus::new(&n).expect("the published modulus is odd and normalized");
    assert_eq!(modulus.bits(), 1024);

    let mut message = vec![0u8; 128];
    for (index, slot) in message.iter_mut().enumerate() {
        *slot = u8::try_from(index.wrapping_mul(7) % 251).unwrap_or(1);
    }

    let mut signature = vec![0u8; 128];
    modulus
        .pow_wide(&message, &d, &mut signature)
        .expect("the message is below the modulus");
    let mut recovered = vec![0u8; 128];
    modulus
        .pow(&signature, 65537, &mut recovered)
        .expect("the signature is below the modulus");
    assert_eq!(hex(&recovered), hex(&message));
}
