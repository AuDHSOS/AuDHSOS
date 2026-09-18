// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(clippy::arithmetic_side_effects, reason = "bounded fixture arithmetic")]

//! R8: the square root, the inverse tangent and the power, against the bounds
//! they state. No test uses a floating-point reference, because D-177 leaves
//! none in the crate; each bound is checked by an identity or by a round trip.

use text_core::Fixed;

use crate::{
    RasterError,
    math::{atan2_turns, pow, sqrt},
};

/// The bound `atan2_turns` states, in Q32.32 units.
const ATAN_BOUND: i64 = 1 << 4;
/// The bound `pow` states, in Q32.32 units.
const POW_BOUND: i64 = 1 << 8;

fn bits(value: i64) -> Fixed {
    Fixed::from_bits(value)
}

fn close(a: Fixed, b: Fixed, bound: i64) -> bool {
    (a.bits() - b.bits()).abs() <= bound
}

#[test]
fn sqrt_every_result_is_the_nearest_root() {
    let mut value: i64 = 1;
    let mut checked = 0;
    while value < i64::MAX / 3 {
        let root = i128::from(sqrt(bits(value)).unwrap().bits());
        let wide = i128::from(value) << 32;
        // The nearest integer root `r` satisfies `(r - 1)^2 < wide` and
        // `wide < (r + 1)^2`, which brackets it without a division.
        assert!((root - 1) * (root - 1) < wide, "{value} under");
        assert!(wide < (root + 1) * (root + 1), "{value} over");
        value = value * 3 + 1;
        checked += 1;
    }
    assert!(checked > 30);
    for power in 0..31_u32 {
        let square = Fixed::from_i32(1 << power).checked_mul(Fixed::from_i32(1 << power));
        if let Ok(square) = square {
            assert_eq!(sqrt(square).unwrap(), Fixed::from_i32(1 << power));
        }
    }
}

#[test]
fn sqrt_exact_squares_round_trip() {
    for value in 1..=64_i32 {
        let exact = Fixed::from_i32(value);
        assert_eq!(sqrt(exact.checked_mul(exact).unwrap()).unwrap(), exact);
    }
    assert_eq!(sqrt(Fixed::ZERO).unwrap(), Fixed::ZERO);
    assert_eq!(sqrt(Fixed::ONE).unwrap(), Fixed::ONE);
}

#[test]
fn sqrt_negative_is_refused() {
    assert_eq!(sqrt(bits(-1)).unwrap_err(), RasterError::Domain);
    assert_eq!(sqrt(Fixed::from_i32(-4)).unwrap_err(), RasterError::Domain);
}

#[test]
fn atan2_axis_directions_are_exact_quarters() {
    let one = Fixed::ONE;
    let zero = Fixed::ZERO;
    for (y, x, expect) in [
        (zero, one, 0_i64),
        (one, zero, 1 << 31),
        (zero, Fixed::from_i32(-1), 1 << 32),
        (Fixed::from_i32(-1), zero, -(1 << 31)),
        (one, one, 1 << 30),
        (one, Fixed::from_i32(-1), 3 << 30),
        (Fixed::from_i32(-1), Fixed::from_i32(-1), -(3 << 30)),
        (Fixed::from_i32(-1), one, -(1 << 30)),
    ] {
        assert!(
            close(atan2_turns(y, x).unwrap(), bits(expect), ATAN_BOUND),
            "atan2({y:?}, {x:?}) gave {:?}, wanted {expect}",
            atan2_turns(y, x).unwrap()
        );
    }
    assert_eq!(atan2_turns(zero, zero).unwrap(), zero);
}

#[test]
fn atan2_complements_the_transposed_direction() {
    // `atan(z) + atan(1/z)` is a quarter turn, which in half-turns is `1/2`.
    // Transposing the arguments reflects the direction in the diagonal, so the
    // two angles must add to that quarter turn for every direction of the
    // first quadrant.
    let half = bits(1 << 31);
    for yi in 1..=64_i32 {
        for xi in 1..=64_i32 {
            let (y, x) = (Fixed::from_i32(yi), Fixed::from_i32(xi));
            let sum = atan2_turns(y, x)
                .unwrap()
                .checked_add(atan2_turns(x, y).unwrap())
                .unwrap();
            assert!(
                close(sum, half, ATAN_BOUND * 2),
                "({yi}, {xi}) and its transpose add to {sum:?}"
            );
        }
    }
}

#[test]
fn atan2_obeys_the_tangent_addition_formula() {
    // `atan(a) + atan(b) = atan((a + b) / (1 - a*b))` while `a*b < 1`, which
    // relates three angles the reduction reaches by different branches.
    for ai in 1..=24_i64 {
        for bi in 1..=24_i64 {
            let a = Fixed::ONE.mul_ratio(ai, 32).unwrap();
            let b = Fixed::ONE.mul_ratio(bi, 32).unwrap();
            let product = a.checked_mul(b).unwrap();
            let numerator = a.checked_add(b).unwrap();
            let denominator = Fixed::ONE.checked_sub(product).unwrap();
            let sum = atan2_turns(a, Fixed::ONE)
                .unwrap()
                .checked_add(atan2_turns(b, Fixed::ONE).unwrap())
                .unwrap();
            let combined = atan2_turns(numerator, denominator).unwrap();
            assert!(
                close(sum, combined, ATAN_BOUND * 4),
                "{ai}/32 and {bi}/32 gave {sum:?} against {combined:?}"
            );
        }
    }
}

#[test]
fn atan2_reflects_through_every_quadrant() {
    for yi in -32..=32_i32 {
        for xi in -32..=32_i32 {
            if xi == 0 && yi == 0 {
                continue;
            }
            let (y, x) = (Fixed::from_i32(yi), Fixed::from_i32(xi));
            let angle = atan2_turns(y, x).unwrap();
            assert!(angle > bits(-(1 << 32)) && angle <= bits(1 << 32));
            // Negating y reflects the angle, except on the negative x axis,
            // where both directions are the half turn itself.
            if yi != 0 {
                let mirrored = atan2_turns(y.checked_neg().unwrap(), x).unwrap();
                assert!(
                    close(mirrored, angle.checked_neg().unwrap(), ATAN_BOUND),
                    "({yi}, {xi}) does not mirror"
                );
            }
            // Scaling the direction leaves the angle alone.
            let scaled = atan2_turns(
                y.checked_mul(Fixed::from_i32(7)).unwrap(),
                x.checked_mul(Fixed::from_i32(7)).unwrap(),
            )
            .unwrap();
            assert!(close(scaled, angle, ATAN_BOUND), "({yi}, {xi}) scales");
        }
    }
}

#[test]
fn atan2_octant_boundaries_agree_with_their_neighbours() {
    // Either side of `tan(pi/8)`, where the reduction switches branch, the
    // result must be continuous: a step would show as a seam in a sweep.
    let boundary = bits(1_779_033_704);
    let below = atan2_turns(boundary.checked_sub(bits(1 << 12)).unwrap(), Fixed::ONE).unwrap();
    let at = atan2_turns(boundary, Fixed::ONE).unwrap();
    let above = atan2_turns(boundary.checked_add(bits(1 << 12)).unwrap(), Fixed::ONE).unwrap();
    assert!(below < at && at < above);
    assert!((at.bits() - below.bits()) < (1 << 13));
    assert!((above.bits() - at.bits()) < (1 << 13));
    assert!(close(at, bits(1 << 29), ATAN_BOUND));
}

#[test]
fn atan2_is_monotone_across_a_quadrant() {
    let mut previous = atan2_turns(Fixed::ZERO, Fixed::ONE).unwrap();
    for step in 1..=512_i32 {
        let angle = atan2_turns(Fixed::from_i32(step), Fixed::from_i32(512)).unwrap();
        assert!(angle > previous, "not increasing at {step}");
        previous = angle;
    }
    assert!(close(previous, bits(1 << 30), ATAN_BOUND));
}

#[test]
fn pow_of_one_is_the_base() {
    for numerator in 1..=64_i64 {
        let base = Fixed::ONE.mul_ratio(numerator, 64).unwrap();
        assert!(close(pow(base, Fixed::ONE).unwrap(), base, POW_BOUND));
    }
}

#[test]
fn pow_of_two_is_the_square() {
    for numerator in 1..=64_i64 {
        let base = Fixed::ONE.mul_ratio(numerator, 64).unwrap();
        let square = base.checked_mul(base).unwrap();
        assert!(
            close(pow(base, Fixed::from_i32(2)).unwrap(), square, POW_BOUND),
            "{numerator}/64 squared"
        );
    }
}

#[test]
fn pow_round_trips_through_its_inverse_exponent() {
    let gamma = Fixed::ONE.mul_ratio(22, 10).unwrap();
    let inverse = Fixed::ONE.checked_div(gamma).unwrap();
    for numerator in 1..=255_i64 {
        let base = Fixed::ONE.mul_ratio(numerator, 255).unwrap();
        let there = pow(base, gamma).unwrap();
        let back = pow(there, inverse).unwrap();
        assert!(
            close(back, base, POW_BOUND * 16),
            "{numerator}/255 went to {there:?} and back to {back:?}"
        );
    }
}

#[test]
fn pow_is_monotone_and_bounded() {
    let gamma = Fixed::ONE.mul_ratio(22, 10).unwrap();
    let mut previous = Fixed::ZERO;
    for numerator in 1..=255_i64 {
        let value = pow(Fixed::ONE.mul_ratio(numerator, 255).unwrap(), gamma).unwrap();
        assert!(value > previous, "not increasing at {numerator}");
        assert!(value <= Fixed::ONE);
        previous = value;
    }
    assert_eq!(pow(Fixed::ZERO, gamma).unwrap(), Fixed::ZERO);
    assert_eq!(pow(Fixed::ONE, gamma).unwrap(), Fixed::ONE);
}

#[test]
fn pow_outside_its_domain_is_refused() {
    let two = Fixed::from_i32(2);
    for (base, exponent) in [
        (bits(-1), Fixed::ONE),
        (two, Fixed::ONE),
        (Fixed::ONE, Fixed::ZERO),
        (Fixed::ONE, bits(-1)),
        (Fixed::ONE, Fixed::from_i32(17)),
    ] {
        assert_eq!(pow(base, exponent).unwrap_err(), RasterError::Domain);
    }
}
