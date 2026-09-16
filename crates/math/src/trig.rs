// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bounded binary64 trigonometric argument reduction and kernels.

const LIMB_BITS: usize = 24;
const LIMB_MASK: u64 = (1 << LIMB_BITS) - 1;

// Binary expansion of 2/pi, most-significant 24-bit limb first. The 1,584
// retained bits exceed the 1,076 bits required to reduce every finite binary64
// input and leave more than 500 guard bits at the largest exponent.
const TWO_OVER_PI: [u32; 66] = [
    0xA2_F983, 0x6E_4E44, 0x15_29FC, 0x27_57D1, 0xF5_34DD, 0xC0_DB62, 0x95_993C, 0x43_9041,
    0xFE_5163, 0xAB_DEBB, 0xC5_61B7, 0x24_6E3A, 0x42_4DD2, 0xE0_0649, 0x2E_EA09, 0xD1_921C,
    0xFE_1DEB, 0x1C_B129, 0xA7_3EE8, 0x82_35F5, 0x2E_BB44, 0x84_E99C, 0x70_26B4, 0x5F_7E41,
    0x39_91D6, 0x39_8353, 0x39_F49C, 0x84_5F8B, 0xBD_F928, 0x3B_1FF8, 0x97_FFDE, 0x05_980F,
    0xEF_2F11, 0x8B_5A0A, 0x6D_1F6D, 0x36_7ECF, 0x27_CB09, 0xB7_4F46, 0x3F_669E, 0x5F_EA2D,
    0x75_27BA, 0xC7_EBE5, 0xF1_7B3D, 0x07_39F7, 0x8A_5292, 0xEA_6BFB, 0x5F_B11F, 0x8D_5D08,
    0x56_0330, 0x46_FC7B, 0x6B_ABF0, 0xCF_BC20, 0x9A_F436, 0x1D_A9E3, 0x91_615E, 0xE6_1B08,
    0x65_9985, 0x5F_14A0, 0x68_408D, 0xFF_D880, 0x4D_7327, 0x31_0606, 0x15_56CA, 0x73_A8C9,
    0x60_E27B, 0xC0_8C6B,
];

const PRODUCT_LIMBS: usize = TWO_OVER_PI.len() + 3;
const TWO_OVER_PI_BITS: usize = TWO_OVER_PI.len() * LIMB_BITS;
const PIO2_FIRST: f64 = 1.570_796_326_734_125_6;
const PIO2_FIRST_TAIL: f64 = 6.077_100_506_506_192e-11;
const CODY_WAITE_LIMIT: f64 = 262_144.0;

/// Returns an implementation-approximated sine for every binary64 input.
#[must_use]
pub fn sin(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 {
        return value;
    }
    if value.is_infinite() {
        return f64::NAN;
    }
    let magnitude = value.abs();
    let (quadrant, reduced) = if magnitude <= core::f64::consts::FRAC_PI_4 {
        (0, magnitude)
    } else if magnitude < CODY_WAITE_LIMIT {
        reduce_cody_waite(magnitude)
    } else {
        reduce_payne_hanek(magnitude)
    };
    let result = match quadrant {
        0 => sin_kernel(reduced),
        1 => cos_kernel(reduced),
        2 => -sin_kernel(reduced),
        _ => -cos_kernel(reduced),
    };
    if value.is_sign_negative() {
        -result
    } else {
        result
    }
}

#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bounded nonnegative quotient is below 2^18"
)]
fn reduce_cody_waite(value: f64) -> (u8, f64) {
    let quotient = (value * core::f64::consts::FRAC_2_PI + 0.5) as u32;
    let quotient_float = f64::from(quotient);
    let reduced = (value - quotient_float * PIO2_FIRST) - quotient_float * PIO2_FIRST_TAIL;
    ((quotient & 3) as u8, reduced)
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "masked 24-bit limbs and an eleven-bit exponent fit their targets"
)]
fn reduce_payne_hanek(value: f64) -> (u8, f64) {
    let bits = value.to_bits();
    let exponent = i32::try_from((bits >> 52) & 0x7ff)
        .unwrap_or(0)
        .saturating_sub(1023);
    let significand = (bits & 0x000f_ffff_ffff_ffff) | (1 << 52);
    let significand_limbs = [
        u32::try_from(significand & LIMB_MASK).unwrap_or(0),
        u32::try_from((significand >> LIMB_BITS) & LIMB_MASK).unwrap_or(0),
        u32::try_from(significand >> (LIMB_BITS * 2)).unwrap_or(0),
    ];
    let mut product = [0u32; PRODUCT_LIMBS];
    for (left_index, left) in significand_limbs.into_iter().enumerate() {
        let mut carry = 0u64;
        for (right_index, right) in TWO_OVER_PI.iter().rev().copied().enumerate() {
            let index = left_index + right_index;
            let Some(slot) = product.get_mut(index) else {
                return (0, value);
            };
            let total = u64::from(*slot) + u64::from(left) * u64::from(right) + carry;
            *slot = u32::try_from(total & LIMB_MASK).unwrap_or(0);
            carry = total >> LIMB_BITS;
        }
        for slot in product.iter_mut().skip(left_index + TWO_OVER_PI.len()) {
            let total = u64::from(*slot) + carry;
            *slot = u32::try_from(total & LIMB_MASK).unwrap_or(0);
            carry = total >> LIMB_BITS;
        }
    }

    let shift = usize::try_from(
        i32::try_from(TWO_OVER_PI_BITS + 52)
            .unwrap_or(i32::MAX)
            .saturating_sub(exponent),
    )
    .unwrap_or(usize::MAX);
    let quotient_low = u8::from(product_bit(&product, shift))
        | (u8::from(product_bit(&product, shift.saturating_add(1))) << 1);
    let half_bit = shift.saturating_sub(1);
    let half = product_bit(&product, half_bit);
    let round_up = half && (any_product_bit_below(&product, half_bit) || quotient_low & 1 != 0);
    let quadrant = quotient_low.wrapping_add(u8::from(round_up)) & 3;

    let mut fraction = 0.0;
    let mut weight = 0.5;
    for offset in 0..64 {
        if product_bit(&product, half_bit.saturating_sub(offset)) {
            fraction += weight;
        }
        weight *= 0.5;
    }
    if round_up {
        fraction -= 1.0;
    }
    let reduced = fraction * core::f64::consts::FRAC_PI_2 + fraction * 6.123_233_995_736_766e-17;
    (quadrant, reduced)
}

fn product_bit(product: &[u32; PRODUCT_LIMBS], bit: usize) -> bool {
    product
        .get(bit / LIMB_BITS)
        .is_some_and(|limb| limb & (1 << (bit % LIMB_BITS)) != 0)
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "remaining is bit modulo 24, so the shift and mask stay in u32"
)]
fn any_product_bit_below(product: &[u32; PRODUCT_LIMBS], bit: usize) -> bool {
    let full_limbs = bit / LIMB_BITS;
    if product.iter().take(full_limbs).any(|limb| *limb != 0) {
        return true;
    }
    let remaining = bit % LIMB_BITS;
    remaining != 0
        && product
            .get(full_limbs)
            .is_some_and(|limb| limb & ((1 << remaining) - 1) != 0)
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "the fixed loop index is in 1..=12"
)]
fn sin_kernel(value: f64) -> f64 {
    let square = value * value;
    let mut term = value;
    let mut sum = value;
    for index in 1i32..=12 {
        let twice = index * 2;
        term *= -square / f64::from(twice * (twice + 1));
        sum += term;
    }
    sum
}

/// Returns an implementation-approximated cosine for every binary64 input.
#[must_use]
pub fn cos(value: f64) -> f64 {
    if value.is_nan() {
        return value;
    }
    if value.is_infinite() {
        return f64::NAN;
    }
    if value == 0.0 {
        return 1.0;
    }
    let magnitude = value.abs();
    let (quadrant, reduced) = if magnitude <= core::f64::consts::FRAC_PI_4 {
        (0, magnitude)
    } else if magnitude < CODY_WAITE_LIMIT {
        reduce_cody_waite(magnitude)
    } else {
        reduce_payne_hanek(magnitude)
    };
    // 21.3.2.12 is even, so the sign of the argument does not reach the
    // answer; the quadrant of the reduction decides it.
    match quadrant {
        0 => cos_kernel(reduced),
        1 => -sin_kernel(reduced),
        2 => -cos_kernel(reduced),
        _ => sin_kernel(reduced),
    }
}

/// Returns an implementation-approximated tangent for every binary64 input.
#[must_use]
pub fn tan(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 {
        return value;
    }
    if value.is_infinite() {
        return f64::NAN;
    }
    let sine = sin(value);
    let cosine = cos(value);
    if cosine == 0.0 {
        return if sine.is_sign_negative() == cosine.is_sign_negative() {
            f64::INFINITY
        } else {
            f64::NEG_INFINITY
        };
    }
    sine / cosine
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "the fixed loop index is in 1..=12"
)]
fn cos_kernel(value: f64) -> f64 {
    let square = value * value;
    let mut term = 1.0;
    let mut sum = 1.0;
    for index in 1i32..=12 {
        let twice = index * 2;
        term *= -square / f64::from((twice - 1) * twice);
        sum += term;
    }
    sum
}
