// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bounded binary64 trigonometric argument reduction and kernels.

use crate::Wide;

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
// Consecutive 33-bit pieces of pi/2 and the remaining tail.
const PIO2_1: f64 = 1.570_796_326_734_125_6;
const PIO2_2: f64 = 6.077_100_506_303_966e-11;
const PIO2_3: f64 = 2.022_266_248_711_166_5e-21;
const PIO2_3T: f64 = 8.478_427_660_368_9e-32;
const PIO2: Wide = Wide(core::f64::consts::FRAC_PI_2, 6.123_233_995_736_766e-17);
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
        (0, Wide(magnitude, 0.0))
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
fn reduce_cody_waite(value: f64) -> (u8, Wide) {
    let quotient = (value * core::f64::consts::FRAC_2_PI + 0.5) as u32;
    let quotient_float = f64::from(quotient);
    let reduced = Wide(value - quotient_float * PIO2_1, 0.0)
        .add(Wide(-quotient_float * PIO2_2, 0.0))
        .add(Wide(-quotient_float * PIO2_3, 0.0))
        .add(Wide(-quotient_float * PIO2_3T, 0.0));
    ((quotient & 3) as u8, reduced)
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "masked 24-bit limbs and an eleven-bit exponent fit their targets"
)]
fn reduce_payne_hanek(value: f64) -> (u8, Wide) {
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
                return (0, Wide(value, 0.0));
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

    let fraction = fraction_pair(&product, shift, round_up);
    let reduced = fraction.mul(PIO2);
    (quadrant, if round_up { reduced.neg() } else { reduced })
}

#[expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "bounded indices and 53-bit integers fit their operations exactly"
)]
fn fraction_pair(product: &[u32; PRODUCT_LIMBS], shift: usize, round_up: bool) -> Wide {
    let mut magnitude = *product;
    let full_limbs = shift / LIMB_BITS;
    let remaining = shift % LIMB_BITS;
    let limb_mask = u32::try_from(LIMB_MASK).unwrap_or(0);
    for limb in magnitude
        .iter_mut()
        .skip(full_limbs + usize::from(remaining != 0))
    {
        *limb = 0;
    }
    if remaining != 0
        && let Some(limb) = magnitude.get_mut(full_limbs)
    {
        *limb &= (1 << remaining) - 1;
    }
    if round_up {
        let mut carry = 1u32;
        for limb in magnitude.iter_mut().take(full_limbs) {
            let total = (limb_mask ^ *limb) + carry;
            *limb = total & limb_mask;
            carry = total >> LIMB_BITS;
        }
        if remaining != 0 {
            let mask = (1 << remaining) - 1;
            if let Some(limb) = magnitude.get_mut(full_limbs) {
                *limb = ((!*limb) & mask).wrapping_add(carry) & mask;
            }
        }
    }
    let Some(top) = (0..shift).rev().find(|&bit| product_bit(&magnitude, bit)) else {
        return Wide(0.0, 0.0);
    };
    let mut hi = 0u64;
    let mut lo = 0u64;
    for offset in 0..106 {
        let bit = top
            .checked_sub(offset)
            .is_some_and(|bit| product_bit(&magnitude, bit));
        if offset < 53 {
            hi = (hi << 1) | u64::from(bit);
        } else {
            lo = (lo << 1) | u64::from(bit);
        }
    }
    let leading_zeros = shift - top - 1;
    let scale = if leading_zeros <= 1022 {
        f64::from_bits(u64::try_from(1023 - leading_zeros).unwrap_or(0) << 52)
    } else if leading_zeros <= 1074 {
        f64::from_bits(1u64 << (1074 - leading_zeros))
    } else {
        0.0
    };
    let high = (hi as f64 * (1.0 / 9_007_199_254_740_992.0)) * scale;
    let low =
        (lo as f64 * (1.0 / 9_007_199_254_740_992.0)) * (1.0 / 9_007_199_254_740_992.0) * scale;
    Wide(high, low)
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
fn sin_kernel(value: Wide) -> f64 {
    let square = value.0 * value.0;
    let mut term = value.0;
    let mut sum = value.0;
    for index in 1i32..=12 {
        let twice = index * 2;
        term *= -square / f64::from(twice * (twice + 1));
        sum += term;
    }
    if value.1 == 0.0 {
        sum
    } else {
        sum + value.1 * cos_kernel_raw(value.0)
    }
}

fn cos_kernel(value: Wide) -> f64 {
    let result = cos_kernel_raw(value.0);
    if value.1 == 0.0 {
        result
    } else {
        result - value.1 * sin_kernel(Wide(value.0, 0.0))
    }
}

#[expect(
    clippy::arithmetic_side_effects,
    reason = "the fixed loop index is in 1..=12"
)]
fn cos_kernel_raw(value: f64) -> f64 {
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
