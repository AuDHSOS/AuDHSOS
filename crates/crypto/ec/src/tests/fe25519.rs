// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The field against the reference of [`super::reference`] and against the
//! identities that pin the operations the reference does not have.

use crypto_ct::Choice;
use test_support::generators::{bytes, vec};
use test_support::property::check;

use crate::fe25519::Fe;
use crate::tests::hex;
use crate::tests::reference::Reference;

/// The prime, as the canonical encoding of a value that is not reduced.
const PRIME: &str = "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f";

/// A pair of elements from a generated pair of byte strings.
fn pair(parts: &[Vec<u8>]) -> Option<([u8; 32], [u8; 32])> {
    let mut first = [0u8; 32];
    let mut second = [0u8; 32];
    for (slot, byte) in first.iter_mut().zip(parts.first()?) {
        *slot = *byte;
    }
    for (slot, byte) in second.iter_mut().zip(parts.get(1)?) {
        *slot = *byte;
    }
    Some((first, second))
}

#[test]
fn property_the_reference_and_the_field_agree() {
    check("fe25519_reference", &vec(bytes(32..=32), 2..=2), |parts| {
        let Some((left, right)) = pair(parts) else {
            return Err("the generator produced fewer than two values".to_owned());
        };
        let (a, b) = (Fe::from_bytes(&left), Fe::from_bytes(&right));
        let (x, y) = (Reference::from_bytes(&left), Reference::from_bytes(&right));

        for (name, ours, theirs) in [
            ("sum", a.add(b).to_bytes(), x.add(y).to_bytes()),
            ("difference", a.sub(b).to_bytes(), x.sub(y).to_bytes()),
            ("product", a.mul(b).to_bytes(), x.mul(y).to_bytes()),
            ("square", a.square().to_bytes(), x.mul(x).to_bytes()),
        ] {
            if ours != theirs {
                return Err(format!(
                    "{name}: {ours} against {theirs}",
                    ours = hex(&ours),
                    theirs = hex(&theirs)
                ));
            }
        }
        Ok(())
    });
}

#[test]
fn property_the_round_trip_through_bytes_keeps_the_value() {
    check("fe25519_bytes", &bytes(32..=32), |message| {
        let mut input = [0u8; 32];
        for (slot, byte) in input.iter_mut().zip(message) {
            *slot = *byte;
        }
        let encoded = Fe::from_bytes(&input).to_bytes();
        if Fe::from_bytes(&encoded).to_bytes() != encoded {
            return Err("the encoding is not stable".to_owned());
        }
        // The highest bit is ignored, so an input that only differs there
        // encodes to the same element.
        let mut flipped = input;
        flipped[31] ^= 0x80;
        if Fe::from_bytes(&flipped).to_bytes() != encoded {
            return Err("the highest bit changed the value".to_owned());
        }
        Ok(())
    });
}

#[test]
fn property_inversion_undoes_multiplication() {
    check("fe25519_invert", &bytes(32..=32), |message| {
        let mut input = [0u8; 32];
        for (slot, byte) in input.iter_mut().zip(message) {
            *slot = *byte;
        }
        let a = Fe::from_bytes(&input);
        if a.is_zero().is_true() {
            return Ok(());
        }
        if a.mul(a.invert()).to_bytes() != Fe::ONE.to_bytes() {
            return Err("the inverse is not an inverse".to_owned());
        }
        Ok(())
    });
}

#[test]
fn property_the_root_power_is_the_documented_exponent() {
    // The power is `(p - 5) / 8`. Raising it to the eighth gives
    // `a^(p - 5)`, which by Fermat is `a^-4`, so multiplying by `a^4`
    // must give one.
    check("fe25519_pow22523", &bytes(32..=32), |message| {
        let mut input = [0u8; 32];
        for (slot, byte) in input.iter_mut().zip(message) {
            *slot = *byte;
        }
        let a = Fe::from_bytes(&input);
        if a.is_zero().is_true() {
            return Ok(());
        }
        let eighth = a.pow22523().square_times(3);
        let fourth = a.square().square();
        if eighth.mul(fourth).to_bytes() != Fe::ONE.to_bytes() {
            return Err("the root power is wrong".to_owned());
        }
        Ok(())
    });
}

#[test]
fn the_canonical_encoding_reduces_values_at_or_above_the_prime() {
    let prime = crate::tests::unhex32(PRIME);
    assert_eq!(hex(&Fe::from_bytes(&prime).to_bytes()), hex(&[0u8; 32]));

    let mut above = prime;
    above[0] = above[0].wrapping_add(1);
    assert_eq!(Fe::from_bytes(&above).to_bytes(), Fe::ONE.to_bytes());

    // The largest value the encoding can carry once the highest bit is
    // dropped is `2^255 - 1`, which is eighteen above the prime.
    let full = [0xFFu8; 32];
    assert_eq!(
        Fe::from_bytes(&full).to_bytes(),
        Fe::from_u64(18).to_bytes()
    );
}

#[test]
fn zero_and_one_are_what_they_claim() {
    assert!(Fe::ZERO.is_zero().is_true());
    assert!(!Fe::ONE.is_zero().is_true());
    assert!(Fe::ONE.is_negative().is_true());
    assert!(!Fe::from_u64(2).is_negative().is_true());
    assert_eq!(Fe::ONE.mul(Fe::ONE).to_bytes(), Fe::ONE.to_bytes());
    assert_eq!(Fe::ZERO.negate().to_bytes(), Fe::ZERO.to_bytes());
    assert_eq!(
        Fe::ONE.negate().add(Fe::ONE).to_bytes(),
        Fe::ZERO.to_bytes()
    );
    assert_eq!(
        Fe::from_limbs([1, 0, 0, 0, 0]).to_bytes(),
        Fe::ONE.to_bytes()
    );
    assert!(!format!("{:?}", Fe::default()).is_empty());
}

#[test]
fn selection_and_exchange_follow_the_choice() {
    let a = Fe::from_u64(7);
    let b = Fe::from_u64(9);
    assert_eq!(Fe::select(Choice::YES, a, b).to_bytes(), a.to_bytes());
    assert_eq!(Fe::select(Choice::NO, a, b).to_bytes(), b.to_bytes());

    let (mut left, mut right) = (a, b);
    Fe::swap(Choice::NO, &mut left, &mut right);
    assert_eq!(
        (left.to_bytes(), right.to_bytes()),
        (a.to_bytes(), b.to_bytes())
    );
    Fe::swap(Choice::YES, &mut left, &mut right);
    assert_eq!(
        (left.to_bytes(), right.to_bytes()),
        (b.to_bytes(), a.to_bytes())
    );
}
