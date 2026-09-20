// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The round constants and the initial values against their definition
//! in FIPS 180-4: the fractional parts of the square and cube roots of
//! the first primes.
//!
//! The fractional part of the `d`-th root of `p` in `w` bits is the low
//! `w` bits of `floor(root_d(p * 2^(d*w)))`, so no fraction is needed.
//! The widest radicand here is `409 * 2^192`, 201 bits, so the
//! arithmetic is 256 bits wide; each root costs O(log n) products of
//! that width, and there are 168 roots.

use crate::sha256;
use crate::sha512;

/// A 256-bit unsigned integer, least significant limb first.
type Wide = [u64; 4];

/// Bits every root here fits in: the largest is `cbrt(409) * 2^64`.
const ROOT_BITS: u32 = 68;

/// The low 64 bits of `value`.
fn low(value: u128) -> u64 {
    u64::try_from(value & u128::from(u64::MAX)).unwrap_or(0)
}

/// `value` as a 256-bit integer.
fn wide(value: u128) -> Wide {
    [low(value), low(value.wrapping_shr(64)), 0, 0]
}

/// `a * b` in 256 bits. Every product taken here fits, so the limbs the
/// loop drops are zero.
fn mul(a: Wide, b: Wide) -> Wide {
    let mut out = [0u64; 4];
    for (offset, left) in a.into_iter().enumerate() {
        let mut carry = 0u128;
        for (position, right) in b.into_iter().enumerate() {
            let index = offset.saturating_add(position);
            if index >= out.len() {
                break;
            }
            let sum = u128::from(left)
                .wrapping_mul(u128::from(right))
                .wrapping_add(u128::from(out[index]))
                .wrapping_add(carry);
            out[index] = low(sum);
            carry = sum.wrapping_shr(64);
        }
    }
    out
}

/// `prime * 2^shift` in 256 bits.
fn radicand(prime: u64, shift: u32) -> Wide {
    let limb = usize::try_from(shift.wrapping_div(64)).unwrap_or(0);
    let spread = u128::from(prime).wrapping_shl(shift.wrapping_rem(64));
    let mut out = [0u64; 4];
    for (index, slot) in out.iter_mut().enumerate() {
        if index == limb {
            *slot = low(spread);
        }
        if index == limb.saturating_add(1) {
            *slot = low(spread.wrapping_shr(64));
        }
    }
    out
}

/// Whether `a` is at most `b`.
fn at_most(a: Wide, b: Wide) -> bool {
    for (left, right) in a.into_iter().zip(b).rev() {
        if left != right {
            return left < right;
        }
    }
    true
}

/// `base^degree` in 256 bits.
fn power(base: u128, degree: u32) -> Wide {
    let mut product = wide(1);
    for _ in 0..degree {
        product = mul(product, wide(base));
    }
    product
}

/// The largest `x` with `x^degree` at most `radicand`, one bit of `x` per
/// round from the top of [`ROOT_BITS`].
fn root(radicand: Wide, degree: u32) -> u128 {
    let mut found = 0u128;
    let mut bit = ROOT_BITS;
    while bit != 0 {
        bit = bit.wrapping_sub(1);
        let candidate = found | 1u128.wrapping_shl(bit);
        if at_most(power(candidate, degree), radicand) {
            found = candidate;
        }
    }
    found
}

/// The fractional part of the `degree`-th root of `prime` in `width`
/// bits, which is the constant FIPS 180-4 takes from it.
fn fraction(prime: u64, degree: u32, width: u32) -> u64 {
    let whole = root(radicand(prime, degree.wrapping_mul(width)), degree);
    let mask = u128::from(u64::MAX).wrapping_shr(64u32.wrapping_sub(width));
    low(whole & mask)
}

/// The first `count` primes, by trial division against the primes below
/// the square root of each candidate.
fn primes(count: usize) -> Vec<u64> {
    let mut found: Vec<u64> = Vec::with_capacity(count);
    let mut candidate = 2u64;
    while found.len() < count {
        if found
            .iter()
            .take_while(|divisor| divisor.saturating_mul(**divisor) <= candidate)
            .all(|divisor| candidate.checked_rem(*divisor) != Some(0))
        {
            found.push(candidate);
        }
        candidate = candidate.saturating_add(1);
    }
    found
}

#[test]
fn the_sha256_initial_values_are_the_square_roots_of_the_first_eight_primes() {
    for (constant, prime) in sha256::INITIAL.into_iter().zip(primes(8)) {
        assert_eq!(u64::from(constant), fraction(prime, 2, 32), "prime {prime}");
    }
}

#[test]
fn the_sha256_round_constants_are_the_cube_roots_of_the_first_sixty_four_primes() {
    let primes = primes(64);
    assert_eq!(primes.last(), Some(&311));
    for (constant, prime) in sha256::K.into_iter().zip(primes) {
        assert_eq!(u64::from(constant), fraction(prime, 3, 32), "prime {prime}");
    }
}

#[test]
fn the_sha512_initial_values_are_the_square_roots_of_the_first_eight_primes() {
    for (constant, prime) in sha512::INITIAL_512.into_iter().zip(primes(8)) {
        assert_eq!(constant, fraction(prime, 2, 64), "prime {prime}");
    }
}

#[test]
fn the_sha384_initial_values_are_the_square_roots_of_the_ninth_to_the_sixteenth_prime() {
    let primes = primes(16);
    let ninth_and_up = primes.get(8..).unwrap_or(&[]).to_vec();
    assert_eq!(ninth_and_up.first(), Some(&23));
    assert_eq!(ninth_and_up.last(), Some(&53));
    for (constant, prime) in sha512::INITIAL_384.into_iter().zip(ninth_and_up) {
        assert_eq!(constant, fraction(prime, 2, 64), "prime {prime}");
    }
}

#[test]
fn the_sha512_round_constants_are_the_cube_roots_of_the_first_eighty_primes() {
    let primes = primes(80);
    assert_eq!(primes.last(), Some(&409));
    for (constant, prime) in sha512::K.into_iter().zip(primes) {
        assert_eq!(constant, fraction(prime, 3, 64), "prime {prime}");
    }
}
