// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The exchange over the 2048-bit group, the range check RFC 8268,
//! section 4, prescribes, and the group type over a second prime.
//!
//! The known answers here are the ones a fixed generator makes exact.
//! Two raised to an exponent below the width of the prime is that power
//! of two and nothing is reduced, so the answer can be written down; two
//! raised to the width itself is one reduction, and the answer is
//! `2^2048 - p`, which the test computes from the prime by subtraction
//! rather than from a second exponentiation.

use test_support::generators::{bytes, vec};
use test_support::property::{Config, check_with};

use crate::error::DhError;
use crate::group::{GROUP14_PRIME, GROUP14_PRIME_BYTES, GROUP14_SECRET_BYTES, group14};
use crate::modp::ModpGroup;
use crate::tests::{hex, unhex};

/// The prime of the 1536-bit MODP group, RFC 3526, section 2, which SSH
/// and IKE call group 5. It is here to run the same code over a second
/// width and a second prime, and is not part of the product.
const RFC3526_SECTION2_PRIME: &str = "\
    FFFFFFFF FFFFFFFF C90FDAA2 2168C234 C4C6628B 80DC1CD1 \
    29024E08 8A67CC74 020BBEA6 3B139B22 514A0879 8E3404DD \
    EF9519B3 CD3A431B 302B0A6D F25F1437 4FE1356D 6D51C245 \
    E485B576 625E7EC6 F44C42E9 A637ED6B 0BFF5CB6 F406B7ED \
    EE386BFB 5A899FA5 AE9F2411 7C4B1FE6 49286651 ECE45B3D \
    C2007CB8 A163BF05 98DA4836 1C55D39A 69163FA8 FD24CF5F \
    83655D23 DCA3AD96 1C62F356 208552BB 9ED52907 7096966D \
    670C354E 4ABC9804 F1746C08 CA237327 FFFFFFFF FFFFFFFF";

/// The group of RFC 3526, section 3.
fn group() -> ModpGroup {
    group14().expect("the constants of the document form a group")
}

/// `2^exponent` as a big-endian value of the width of group 14, for an
/// exponent below that width so that nothing is reduced.
fn power_of_two(exponent: usize) -> Vec<u8> {
    let mut value = vec![0u8; GROUP14_PRIME_BYTES];
    let from_the_end = GROUP14_PRIME_BYTES
        .saturating_sub(1)
        .saturating_sub(exponent / 8);
    for (index, slot) in value.iter_mut().enumerate() {
        if index == from_the_end {
            *slot = 1u8.wrapping_shl(u32::try_from(exponent % 8).unwrap_or(0));
        }
    }
    value
}

/// The prime with `offset` subtracted, big-endian at the width of the
/// group. Used for `p-1` and `p-2`, which the range check turns on.
fn prime_less(offset: u8) -> Vec<u8> {
    let mut value = GROUP14_PRIME.to_vec();
    for slot in value.iter_mut().rev().take(1) {
        *slot = slot.wrapping_sub(offset);
    }
    value
}

/// `2^2048 - p`, which is `2^2048` reduced once.
fn two_to_the_width_reduced() -> Vec<u8> {
    let mut value: Vec<u8> = GROUP14_PRIME.iter().map(|byte| !byte).collect();
    let mut carry = 1u8;
    for slot in value.iter_mut().rev() {
        let (sum, overflow) = slot.overflowing_add(carry);
        *slot = sum;
        carry = u8::from(overflow);
    }
    value
}

#[test]
fn the_generator_raised_to_a_small_exponent_is_that_power_of_two() {
    let group = group();
    for (exponent, encoded) in [
        (1usize, vec![0x01u8]),
        (8, vec![0x08]),
        (255, vec![0xff]),
        (2047, vec![0x07, 0xff]),
    ] {
        let mut out = vec![0u8; GROUP14_PRIME_BYTES];
        group
            .public_value(&encoded, &mut out)
            .expect("the exponent is a usable one");
        assert_eq!(
            hex(&out),
            hex(&power_of_two(exponent)),
            "two to the {exponent}"
        );
    }
}

#[test]
fn the_generator_raised_to_the_width_of_the_prime_is_reduced_once() {
    let group = group();
    let mut out = vec![0u8; GROUP14_PRIME_BYTES];
    group
        .public_value(&[0x08, 0x00], &mut out)
        .expect("the exponent is a usable one");
    assert_eq!(hex(&out), hex(&two_to_the_width_reduced()));
}

#[test]
fn an_exchange_between_two_sides_reaches_one_secret() {
    let group = group();
    let ours = [0x5au8; GROUP14_SECRET_BYTES];
    let theirs: [u8; GROUP14_SECRET_BYTES] = core::array::from_fn(|index| {
        u8::try_from(index.wrapping_mul(11).wrapping_add(3) % 251).unwrap_or(1)
    });

    let mut our_public = vec![0u8; GROUP14_PRIME_BYTES];
    group
        .public_value(&ours, &mut our_public)
        .expect("our exponent is a usable one");
    let mut their_public = vec![0u8; GROUP14_PRIME_BYTES];
    group
        .public_value(&theirs, &mut their_public)
        .expect("their exponent is a usable one");
    assert_ne!(hex(&our_public), hex(&their_public));

    let mut from_us = vec![0u8; GROUP14_PRIME_BYTES];
    group
        .shared_secret(&ours, &their_public, &mut from_us)
        .expect("their public value is in range");
    let mut from_them = vec![0u8; GROUP14_PRIME_BYTES];
    group
        .shared_secret(&theirs, &our_public, &mut from_them)
        .expect("our public value is in range");
    assert_eq!(hex(&from_us), hex(&from_them));
    assert_ne!(hex(&from_us), hex(&power_of_two(0)));
}

/// The exchange is symmetric for whatever exponents it is given. The
/// secrets are four bytes so that a case is four short ladders; the
/// property is about the algebra and not about the width of the
/// exponent, which the known answers above already exercise.
#[test]
fn property_the_two_sides_of_an_exchange_agree() {
    let group = group();
    let generator = vec(bytes(4..=4), 2..=2);
    let config = Config {
        cases: 12,
        ..Config::default()
    };
    let result = check_with(&config, "dh_group14_exchange", &generator, |parts| {
        let (Some(ours), Some(theirs)) = (parts.first(), parts.get(1)) else {
            return Err("the generator produced fewer than two values".to_owned());
        };
        let mut our_public = vec![0u8; GROUP14_PRIME_BYTES];
        let mut their_public = vec![0u8; GROUP14_PRIME_BYTES];
        let mut from_us = vec![0u8; GROUP14_PRIME_BYTES];
        let mut from_them = vec![0u8; GROUP14_PRIME_BYTES];
        match (
            group.public_value(ours, &mut our_public),
            group.public_value(theirs, &mut their_public),
        ) {
            // An exponent of zero sends one, which must not be sent.
            (Err(_), _) | (_, Err(_)) => return Ok(()),
            (Ok(()), Ok(())) => {}
        }
        group
            .shared_secret(ours, &their_public, &mut from_us)
            .map_err(|error| error.to_string())?;
        group
            .shared_secret(theirs, &our_public, &mut from_them)
            .map_err(|error| error.to_string())?;
        if from_us == from_them {
            Ok(())
        } else {
            Err(format!("{} against {}", hex(&from_us), hex(&from_them)))
        }
    });
    if let Err(failure) = result {
        panic!("{failure}");
    }
}

#[test]
fn the_range_check_accepts_a_value_inside_the_open_interval() {
    let group = group();
    for value in [
        power_of_two(1),
        power_of_two(2047),
        prime_less(2),
        two_to_the_width_reduced(),
    ] {
        assert_eq!(group.check_public(&value), Ok(()), "{}", hex(&value));
    }
}

#[test]
fn the_range_check_refuses_the_ends_of_the_interval_and_everything_beyond() {
    let group = group();
    let mut prime_and_one = GROUP14_PRIME.to_vec();
    let mut carry = 1u8;
    for slot in prime_and_one.iter_mut().rev() {
        let (sum, overflow) = slot.overflowing_add(carry);
        *slot = sum;
        carry = u8::from(overflow);
    }
    assert_eq!(carry, 0);
    for value in [
        Vec::new(),
        vec![0u8; GROUP14_PRIME_BYTES],
        power_of_two(0),
        prime_less(1),
        GROUP14_PRIME.to_vec(),
        prime_and_one,
        vec![0xffu8; 513],
    ] {
        assert_eq!(
            group.check_public(&value),
            Err(DhError::PublicValueOutOfRange),
            "{}",
            hex(&value)
        );
    }
}

/// An SSH `mpint` whose value has its top bit set carries a leading zero
/// byte, so a value of this group arrives 257 bytes long as often as not.
/// Nothing in the check is tied to the width of the group, so a value
/// buried under any number of zeros reads the same — even under more of
/// them than the widest value the arithmetic holds.
#[test]
fn the_range_check_accepts_a_value_under_leading_zero_bytes() {
    let group = group();
    for zeros in [1usize, GROUP14_PRIME_BYTES, 600] {
        let mut padded = vec![0u8; zeros];
        padded.extend_from_slice(&prime_less(2));
        assert_eq!(group.check_public(&padded), Ok(()), "{zeros} leading zeros");
    }
}

/// A buffer wider than the arithmetic is a legal place to write a value
/// of the group, and the value read back out of it is the same one.
#[test]
fn a_public_value_may_be_written_into_a_buffer_wider_than_the_arithmetic() {
    let group = group();
    let mut wide = vec![0u8; 600];
    group
        .public_value(&[0x08], &mut wide)
        .expect("the exponent is a usable one");
    assert_eq!(
        hex(&wide),
        format!("{}{}", hex(&vec![0u8; 344]), hex(&power_of_two(8)))
    );
}

#[test]
fn an_exponent_of_zero_produces_a_public_value_that_must_not_be_sent() {
    let group = group();
    let mut out = vec![0u8; GROUP14_PRIME_BYTES];
    assert_eq!(
        group.public_value(&[0u8; GROUP14_SECRET_BYTES], &mut out),
        Err(DhError::PublicValueOutOfRange)
    );
}

#[test]
fn an_exponent_of_zero_produces_a_shared_secret_that_must_not_be_used() {
    let group = group();
    let mut out = vec![0u8; GROUP14_PRIME_BYTES];
    assert_eq!(
        group.shared_secret(&[0u8; GROUP14_SECRET_BYTES], &power_of_two(1), &mut out),
        Err(DhError::DegenerateSharedSecret)
    );
}

#[test]
fn the_exchange_refuses_a_peer_value_outside_the_interval() {
    let group = group();
    let mut out = vec![0u8; GROUP14_PRIME_BYTES];
    assert_eq!(
        group.shared_secret(&[7u8; 4], &prime_less(1), &mut out),
        Err(DhError::PublicValueOutOfRange)
    );
}

#[test]
fn both_operations_refuse_an_output_narrower_than_the_group() {
    let group = group();
    let mut out = vec![0u8; GROUP14_PRIME_BYTES - 1];
    assert_eq!(
        group.public_value(&[7u8; 4], &mut out),
        Err(DhError::OutputTooShort)
    );
    assert_eq!(
        group.shared_secret(&[7u8; 4], &power_of_two(1), &mut out),
        Err(DhError::OutputTooShort)
    );
}

#[test]
fn a_group_refuses_a_generator_below_two() {
    for generator in [0u8, 1] {
        assert_eq!(
            ModpGroup::new(&GROUP14_PRIME, generator),
            Err(DhError::InvalidGenerator)
        );
    }
}

#[test]
fn a_group_refuses_a_prime_the_arithmetic_cannot_hold() {
    let mut even = GROUP14_PRIME.to_vec();
    for slot in even.iter_mut().rev().take(1) {
        *slot &= 0xfe;
    }
    let mut too_wide = vec![0x81u8];
    too_wide.extend_from_slice(&[0xffu8; 512]);
    for prime in [Vec::new(), vec![0u8; 256], even, too_wide] {
        assert_eq!(ModpGroup::new(&prime, 2), Err(DhError::InvalidPrime));
    }
}

/// The same exchange over the 1536-bit group of the same document, so
/// that nothing in the type is tied to one width or one prime.
#[test]
fn the_group_of_the_other_section_exchanges_at_its_own_width() {
    let prime = unhex(RFC3526_SECTION2_PRIME);
    assert_eq!(prime.len(), 192);
    let group = ModpGroup::new(&prime, 2).expect("the constants of the document form a group");
    assert_eq!(group.public_len(), 192);

    let ours = [0x11u8, 0x22, 0x33, 0x44];
    let theirs = [0x99u8, 0x88, 0x77, 0x66];
    let mut our_public = vec![0u8; 192];
    let mut their_public = vec![0u8; 192];
    group
        .public_value(&ours, &mut our_public)
        .expect("our exponent is a usable one");
    group
        .public_value(&theirs, &mut their_public)
        .expect("their exponent is a usable one");

    let mut from_us = vec![0u8; 192];
    let mut from_them = vec![0u8; 192];
    group
        .shared_secret(&ours, &their_public, &mut from_us)
        .expect("their public value is in range");
    group
        .shared_secret(&theirs, &our_public, &mut from_them)
        .expect("our public value is in range");
    assert_eq!(hex(&from_us), hex(&from_them));
}

#[test]
fn every_refusal_renders_a_sentence_of_its_own() {
    let mut seen = Vec::new();
    for error in [
        DhError::InvalidPrime,
        DhError::InvalidGenerator,
        DhError::PublicValueOutOfRange,
        DhError::DegenerateSharedSecret,
        DhError::OutputTooShort,
    ] {
        let rendered = error.to_string();
        assert!(!rendered.is_empty(), "{error:?}");
        assert!(!seen.contains(&rendered), "{error:?} repeats a sentence");
        seen.push(rendered);
    }
}
