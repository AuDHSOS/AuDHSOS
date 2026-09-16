// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Allocation-free binary64 elementary functions for the clauses of 21.3.2.
//!
//! Every function answers the special values its clause names exactly. A
//! finite result is implementation-approximated, which 21.3.2 allows for
//! every one of them but `Math.sqrt`, whose result is the correctly rounded
//! square root.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "binary64 arithmetic answers the infinities and NaN the clauses name"
)]
#![expect(
    clippy::float_cmp,
    reason = "the clauses of 21.3.2 classify their argument by exact equality"
)]
#![expect(
    clippy::as_conversions,
    reason = "the exponent and the significand of a binary64 convert by their width"
)]
#![expect(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "every conversion below stands on a range the caller has bounded"
)]
#![expect(
    clippy::missing_const_for_fn,
    reason = "a const function cannot answer the floating-point arithmetic these need"
)]

/// Bits of the significand of a binary64.
const SIGNIFICAND_BITS: i32 = 52;
/// The significand mask of a binary64.
const SIGNIFICAND_MASK: u64 = (1u64 << 52) - 1;
/// The bias of the exponent of a binary64.
const EXPONENT_BIAS: i32 = 1023;
/// The exponent mask of a binary64, unshifted.
const EXPONENT_MASK: u64 = 0x7ff;

/// `2^exponent`, for an exponent the binary64 format holds.
fn power_of_two(exponent: i32) -> f64 {
    if exponent > 1023 {
        return f64::INFINITY;
    }
    if exponent < -1074 {
        return 0.0;
    }
    if exponent < -1022 {
        // The result is subnormal, so the bit stands below the significand.
        let shift = -1022 - exponent;
        return f64::from_bits(1u64 << (52 - shift));
    }
    f64::from_bits(((exponent + EXPONENT_BIAS) as u64) << 52)
}

/// The significand and the exponent of a finite positive binary64: the value
/// is `significand * 2^exponent` with the significand in `[1, 2)`.
fn decompose(value: f64) -> (f64, i32) {
    let mut bits = value.to_bits();
    let mut exponent = ((bits >> 52) & EXPONENT_MASK) as i32;
    if exponent == 0 {
        // A subnormal is scaled up by 2^64 first, which is exact.
        let scaled = value * 18_446_744_073_709_551_616.0;
        bits = scaled.to_bits();
        exponent = ((bits >> 52) & EXPONENT_MASK) as i32 - 64;
    }
    let significand = f64::from_bits((bits & SIGNIFICAND_MASK) | ((EXPONENT_BIAS as u64) << 52));
    (significand, exponent - EXPONENT_BIAS)
}

/// The integer nearest to a value the caller has bounded well inside `i32`.
fn nearest_integer(value: f64) -> i32 {
    let shifted = if value >= 0.0 {
        value + 0.5
    } else {
        value - 0.5
    };
    shifted as i32
}

/// `floor(sqrt(value))` for a `u128`, by the bit-by-bit method.
const fn integer_square_root(value: u128) -> u128 {
    let mut root: u128 = 0;
    let mut bit: u128 = 1 << 126;
    let mut rest = value;
    while bit > rest {
        bit >>= 2;
    }
    while bit != 0 {
        if rest >= root + bit {
            rest -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    root
}

/// `Math.sqrt` of 21.3.2.32, correctly rounded.
///
/// The significand is squared out to 105 bits, whose integer square root
/// carries the 53 bits of the answer and the remainder that rounds them.
#[must_use]
pub fn sqrt(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 {
        return value;
    }
    if value < 0.0 {
        return f64::NAN;
    }
    if value.is_infinite() {
        return value;
    }
    let (significand, exponent) = decompose(value);
    // The significand is an integer of 53 bits once it is scaled by 2^52.
    let mantissa = (significand * power_of_two(SIGNIFICAND_BITS)) as u128;
    let mut exponent = exponent - SIGNIFICAND_BITS;
    let mut wide = mantissa << 52;
    exponent -= SIGNIFICAND_BITS;
    if exponent % 2 != 0 {
        wide <<= 1;
        exponent -= 1;
    }
    let root = integer_square_root(wide);
    let rounded = if wide - root * root > root {
        root + 1
    } else {
        root
    };
    (rounded as f64) * power_of_two(exponent / 2)
}

/// `Math.cbrt` of 21.3.2.9.
#[must_use]
pub fn cbrt(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 || value.is_infinite() {
        return value;
    }
    let magnitude = value.abs();
    let (significand, exponent) = decompose(magnitude);
    // The exponent is split into a multiple of three and a remainder the
    // significand takes, so the seed starts within one octave of the answer.
    let third = exponent.div_euclid(3);
    let rest = exponent - third * 3;
    let scaled = significand * power_of_two(rest);
    let mut root = scaled * 0.5 + 0.5;
    for _ in 0..8 {
        root = (2.0 * root + scaled / (root * root)) / 3.0;
    }
    let answer = root * power_of_two(third);
    if value.is_sign_negative() {
        -answer
    } else {
        answer
    }
}

/// `Math.exp` of 21.3.2.14.
#[must_use]
pub fn exp(value: f64) -> f64 {
    if value.is_nan() {
        return value;
    }
    if value == 0.0 {
        return 1.0;
    }
    if value == f64::INFINITY {
        return value;
    }
    if value == f64::NEG_INFINITY {
        return 0.0;
    }
    if value > 709.782_712_893_384 {
        return f64::INFINITY;
    }
    if value < -745.133_219_101_941 {
        return 0.0;
    }
    let scale = nearest_integer(value * core::f64::consts::LOG2_E);
    // The two halves of `ln 2` keep the reduced argument accurate.
    let reduced = value
        - f64::from(scale) * core::f64::consts::LN_2
        - f64::from(scale) * 2.319_046_813_846_3e-17;
    let mut term = 1.0;
    let mut sum = 1.0;
    for index in 1i32..=16 {
        term *= reduced / f64::from(index);
        sum += term;
    }
    sum * power_of_two(scale)
}

/// `Math.expm1` of 21.3.2.15.
#[must_use]
pub fn expm1(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 || value == f64::INFINITY {
        return value;
    }
    if value == f64::NEG_INFINITY {
        return -1.0;
    }
    if value.abs() >= 0.5 {
        return exp(value) - 1.0;
    }
    // The series answers the small arguments the subtraction would cancel.
    let mut term = value;
    let mut sum = value;
    for index in 2i32..=18 {
        term *= value / f64::from(index);
        sum += term;
    }
    sum
}

/// The natural logarithm of a significand in `[1, 2)`, by the series of
/// `2 atanh((m - 1) / (m + 1))`.
fn log_significand(significand: f64) -> f64 {
    let ratio = (significand - 1.0) / (significand + 1.0);
    let square = ratio * ratio;
    let mut term = ratio;
    let mut sum = ratio;
    for index in 1i32..=18 {
        term *= square;
        sum += term / f64::from(index * 2 + 1);
    }
    sum * 2.0
}

/// `Math.log` of 21.3.2.20.
#[must_use]
pub fn log(value: f64) -> f64 {
    if value.is_nan() {
        return value;
    }
    if value < 0.0 {
        return f64::NAN;
    }
    if value == 0.0 {
        return f64::NEG_INFINITY;
    }
    if value == f64::INFINITY || value == 1.0 {
        return if value == 1.0 { 0.0 } else { value };
    }
    let (mut significand, mut exponent) = decompose(value);
    if significand > core::f64::consts::SQRT_2 {
        significand *= 0.5;
        exponent += 1;
    }
    f64::from(exponent) * core::f64::consts::LN_2 + log_significand(significand)
}

/// `Math.log1p` of 21.3.2.21.
#[must_use]
pub fn log1p(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 || value == f64::INFINITY {
        return value;
    }
    if value < -1.0 {
        return f64::NAN;
    }
    if value == -1.0 {
        return f64::NEG_INFINITY;
    }
    if value.abs() >= 0.5 {
        return log(1.0 + value);
    }
    // The series answers the small arguments the addition would cancel.
    let ratio = value / (value + 2.0);
    let square = ratio * ratio;
    let mut term = ratio;
    let mut sum = ratio;
    for index in 1i32..=18 {
        term *= square;
        sum += term / f64::from(index * 2 + 1);
    }
    sum * 2.0
}

/// The exponent of a value that is an exact power of the radix, and nothing
/// for every other value.
fn exact_power(value: f64, radix: f64) -> Option<i32> {
    let mut power = 1.0f64;
    let mut exponent = 0i32;
    while power < value && exponent < 400 {
        power *= radix;
        exponent += 1;
    }
    if power == value {
        return Some(exponent);
    }
    let mut power = 1.0f64;
    let mut exponent = 0i32;
    while power > value && exponent > -400 {
        power /= radix;
        exponent -= 1;
    }
    (power == value).then_some(exponent)
}

/// `Math.log2` of 21.3.2.23.
#[must_use]
pub fn log2(value: f64) -> f64 {
    if value.is_nan() || value < 0.0 {
        return if value.is_nan() { value } else { f64::NAN };
    }
    if value == 0.0 {
        return f64::NEG_INFINITY;
    }
    if value == f64::INFINITY {
        return value;
    }
    // A power of two answers its exponent, which no series would land on.
    let (significand, exponent) = decompose(value);
    if significand == 1.0 {
        return f64::from(exponent);
    }
    log(value) / core::f64::consts::LN_2
}

/// `Math.log10` of 21.3.2.22.
#[must_use]
pub fn log10(value: f64) -> f64 {
    if value.is_nan() || value < 0.0 {
        return if value.is_nan() { value } else { f64::NAN };
    }
    if value == 0.0 {
        return f64::NEG_INFINITY;
    }
    if value == f64::INFINITY {
        return value;
    }
    if let Some(exponent) = exact_power(value, 10.0) {
        return f64::from(exponent);
    }
    log(value) / core::f64::consts::LN_10
}

/// The arc tangent of a value in `[0, 1]`, by the series of `atan`.
fn atan_kernel(value: f64) -> f64 {
    // The half-angle identity halves the argument twice, so the series
    // converges on `[0, tan(pi/16)]` whatever it was given.
    let once = value / (1.0 + sqrt(1.0 + value * value));
    let twice = once / (1.0 + sqrt(1.0 + once * once));
    let square = twice * twice;
    let mut term = twice;
    let mut sum = twice;
    for index in 1i32..=20 {
        term *= -square;
        sum += term / f64::from(index * 2 + 1);
    }
    sum * 4.0
}

/// `Math.atan` of 21.3.2.7.
#[must_use]
pub fn atan(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 {
        return value;
    }
    if value == f64::INFINITY {
        return core::f64::consts::FRAC_PI_2;
    }
    if value == f64::NEG_INFINITY {
        return -core::f64::consts::FRAC_PI_2;
    }
    let magnitude = value.abs();
    let answer = if magnitude > 1.0 {
        core::f64::consts::FRAC_PI_2 - atan_kernel(1.0 / magnitude)
    } else {
        atan_kernel(magnitude)
    };
    if value.is_sign_negative() {
        -answer
    } else {
        answer
    }
}

/// `Math.atan2` of 21.3.2.8, whose table names every special value.
#[must_use]
pub fn atan2(y: f64, x: f64) -> f64 {
    if y.is_nan() || x.is_nan() {
        return f64::NAN;
    }
    let quarter = core::f64::consts::FRAC_PI_4;
    if y.is_infinite() {
        let angle = if x == f64::INFINITY {
            quarter
        } else if x == f64::NEG_INFINITY {
            quarter * 3.0
        } else {
            core::f64::consts::FRAC_PI_2
        };
        return if y.is_sign_negative() { -angle } else { angle };
    }
    if x == f64::INFINITY {
        return if y.is_sign_negative() { -0.0 } else { 0.0 };
    }
    if x == f64::NEG_INFINITY {
        return if y.is_sign_negative() {
            -core::f64::consts::PI
        } else {
            core::f64::consts::PI
        };
    }
    if y == 0.0 {
        let angle = if x > 0.0 || (x == 0.0 && !x.is_sign_negative()) {
            0.0
        } else {
            core::f64::consts::PI
        };
        return if y.is_sign_negative() { -angle } else { angle };
    }
    if x == 0.0 {
        return if y.is_sign_negative() {
            -core::f64::consts::FRAC_PI_2
        } else {
            core::f64::consts::FRAC_PI_2
        };
    }
    let angle = atan((y / x).abs());
    let angle = if x < 0.0 {
        core::f64::consts::PI - angle
    } else {
        angle
    };
    if y.is_sign_negative() { -angle } else { angle }
}

/// `Math.asin` of 21.3.2.5.
#[must_use]
pub fn asin(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 {
        return value;
    }
    let magnitude = value.abs();
    if magnitude > 1.0 {
        return f64::NAN;
    }
    let answer = if magnitude == 1.0 {
        core::f64::consts::FRAC_PI_2
    } else {
        atan(magnitude / sqrt(1.0 - magnitude * magnitude))
    };
    if value.is_sign_negative() {
        -answer
    } else {
        answer
    }
}

/// `Math.acos` of 21.3.2.2.
#[must_use]
pub fn acos(value: f64) -> f64 {
    if value.is_nan() {
        return value;
    }
    if value > 1.0 || value < -1.0 {
        return f64::NAN;
    }
    if value == 1.0 {
        return 0.0;
    }
    core::f64::consts::FRAC_PI_2 - asin(value)
}

/// `Math.sinh` of 21.3.2.31.
#[must_use]
pub fn sinh(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 || value.is_infinite() {
        return value;
    }
    if value.abs() < 0.5 {
        let half = expm1(value);
        return f64::midpoint(half, half / (half + 1.0));
    }
    let raised = exp(value);
    (raised - 1.0 / raised) * 0.5
}

/// `Math.cosh` of 21.3.2.13.
#[must_use]
pub fn cosh(value: f64) -> f64 {
    if value.is_nan() {
        return value;
    }
    if value.is_infinite() {
        return f64::INFINITY;
    }
    if value == 0.0 {
        return 1.0;
    }
    let raised = exp(value.abs());
    f64::midpoint(raised, 1.0 / raised)
}

/// `Math.tanh` of 21.3.2.34.
#[must_use]
pub fn tanh(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 {
        return value;
    }
    if value == f64::INFINITY {
        return 1.0;
    }
    if value == f64::NEG_INFINITY {
        return -1.0;
    }
    let twice = expm1(2.0 * value.abs());
    let answer = twice / (twice + 2.0);
    if value.is_sign_negative() {
        -answer
    } else {
        answer
    }
}

/// `Math.asinh` of 21.3.2.6.
#[must_use]
pub fn asinh(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 || value.is_infinite() {
        return value;
    }
    let magnitude = value.abs();
    let answer = log(magnitude + sqrt(magnitude * magnitude + 1.0));
    if value.is_sign_negative() {
        -answer
    } else {
        answer
    }
}

/// `Math.acosh` of 21.3.2.3.
#[must_use]
pub fn acosh(value: f64) -> f64 {
    if value.is_nan() {
        return value;
    }
    if value < 1.0 {
        return f64::NAN;
    }
    if value == 1.0 {
        return 0.0;
    }
    if value == f64::INFINITY {
        return value;
    }
    log(value + sqrt(value * value - 1.0))
}

/// `Math.atanh` of 21.3.2.9.
#[must_use]
pub fn atanh(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 {
        return value;
    }
    let magnitude = value.abs();
    if magnitude > 1.0 {
        return f64::NAN;
    }
    if magnitude == 1.0 {
        return if value.is_sign_negative() {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let answer = log1p(2.0 * magnitude / (1.0 - magnitude)) * 0.5;
    if value.is_sign_negative() {
        -answer
    } else {
        answer
    }
}

/// `Math.f16round` of 21.3.2.17: the nearest binary16, back as a binary64.
#[must_use]
pub fn f16round(value: f64) -> f64 {
    if value.is_nan() || value == 0.0 || value.is_infinite() {
        return value;
    }
    let magnitude = value.abs();
    // 65520 is the midpoint between the largest binary16 and the next one up.
    if magnitude >= 65_520.0 {
        return if value.is_sign_negative() {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let (_, exponent) = decompose(magnitude);
    // A binary16 carries eleven bits of significand and no exponent below
    // -14, so every finite one is a multiple of 2^-24.
    let quantum_exponent = (exponent - 10).max(-24);
    let quantum = power_of_two(quantum_exponent);
    let scaled = magnitude / quantum;
    let floor = (scaled as u64) as f64;
    let rest = scaled - floor;
    let rounded = if rest > 0.5 || (rest == 0.5 && (floor as u64) % 2 == 1) {
        floor + 1.0
    } else {
        floor
    };
    let answer = rounded * quantum;
    if value.is_sign_negative() {
        -answer
    } else {
        answer
    }
}

/// `Math.hypot` of 21.3.2.18.
///
/// An infinite argument answers the infinity whatever else the list holds,
/// and a NaN answers NaN once no argument is infinite. The values are scaled
/// by the largest magnitude, so the sum of the squares neither overflows nor
/// underflows where the answer does not.
#[must_use]
pub fn hypot(values: &[f64]) -> f64 {
    let mut largest = 0.0f64;
    let mut sawnan = false;
    for value in values {
        if value.is_infinite() {
            return f64::INFINITY;
        }
        if value.is_nan() {
            sawnan = true;
        } else if value.abs() > largest {
            largest = value.abs();
        }
    }
    if sawnan {
        return f64::NAN;
    }
    if largest == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0f64;
    for value in values {
        let scaled = value / largest;
        sum += scaled * scaled;
    }
    largest * sqrt(sum)
}
