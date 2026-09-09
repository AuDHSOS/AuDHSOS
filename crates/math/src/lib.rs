// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors
#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod integer;
pub use integer::RadixInteger;

// Exact comparisons below classify IEEE special values and representable integers.
#[expect(
    clippy::float_cmp,
    reason = "ECMAScript special-case equality and integer parity"
)]
/// Raises a binary64 base to a binary64 exponent with ECMAScript special values.
/// Finite transcendental results are implementation-approximated. Work is bounded
/// independently of exponent magnitude; no host math library is called.
#[must_use]
pub fn pow(base: f64, exponent: f64) -> f64 {
    if exponent.is_nan() {
        return f64::NAN;
    }
    if exponent == 0.0 {
        return 1.0;
    }
    if base.is_nan() {
        return f64::NAN;
    }
    let odd =
        exponent.abs() < 9_007_199_254_740_992.0 && exponent % 2.0 != 0.0 && exponent % 1.0 == 0.0;
    let negative = base.is_sign_negative() && odd;
    let magnitude = base.abs();
    let result = if base.is_infinite() {
        if exponent > 0.0 { f64::INFINITY } else { 0.0 }
    } else if magnitude == 0.0 {
        if exponent > 0.0 { 0.0 } else { f64::INFINITY }
    } else if exponent.is_infinite() {
        if magnitude == 1.0 {
            return f64::NAN;
        }
        if (magnitude > 1.0) == (exponent > 0.0) {
            f64::INFINITY
        } else {
            0.0
        }
    } else {
        if base < 0.0 && exponent % 1.0 != 0.0 {
            return f64::NAN;
        }
        if exponent == 1.0 {
            return base;
        }
        if exponent == 2.0 {
            return base * base;
        }
        if exponent == -1.0 {
            return 1.0 / base;
        }
        if magnitude == 1.0 {
            1.0
        } else if exponent.abs() <= 16.0
            && exponent % 1.0 == 0.0
            && (1.0 / 4_294_967_296.0..=4_294_967_296.0).contains(&magnitude)
        {
            small_integer_pow(magnitude, exponent)
        } else {
            positive_pow(magnitude, exponent)
        }
    };
    if negative { -result } else { result }
}

#[derive(Clone, Copy)]
struct Wide(f64, f64);
impl Wide {
    fn add(self, r: Self) -> Self {
        let s = self.0 + r.0;
        let v = s - self.0;
        let e = (self.0 - (s - v)) + (r.0 - v) + self.1 + r.1;
        Self::normalize(s, e)
    }
    fn normalize(a: f64, b: f64) -> Self {
        let s = a + b;
        Self(s, b - (s - a))
    }
    fn neg(self) -> Self {
        Self(-self.0, -self.1)
    }
    fn mul(self, r: Self) -> Self {
        let p = self.0 * r.0;
        // Bit splitting avoids overflow from Dekker's usual 2^27+1 multiplier.
        let ah = f64::from_bits(self.0.to_bits() & !0x7ff_ffff);
        let al = self.0 - ah;
        let bh = f64::from_bits(r.0.to_bits() & !0x7ff_ffff);
        let bl = r.0 - bh;
        let e = ((ah * bh - p) + ah * bl + al * bh)
            + al * bl
            + self.0 * r.1
            + self.1 * r.0
            + self.1 * r.1;
        Self::normalize(p, e)
    }
    fn div(self, r: Self) -> Self {
        let q = self.0 / r.0;
        let remainder = self.add(r.mul(Self(q, 0.0)).neg());
        let correction = (remainder.0 + remainder.1) / r.0;
        Self::normalize(q, correction)
    }
}
const LN2: Wide = Wide(core::f64::consts::LN_2, 2.319_046_813_846_299_6e-17);

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions,
    reason = "caller bounds the exactly integral absolute exponent to [1,16]"
)]
fn small_integer_pow(base: f64, exponent: f64) -> f64 {
    let mut n = exponent.abs() as u32;
    let mut factor = Wide(base, 0.0);
    let mut result = Wide(1.0, 0.0);
    while n != 0 {
        if n & 1 != 0 {
            result = result.mul(factor);
        }
        n >>= 1;
        if n != 0 {
            factor = factor.mul(factor);
        }
    }
    if exponent.is_sign_negative() {
        result = Wide(1.0, 0.0).div(result);
    }
    result.0 + result.1
}

fn logarithm(mut x: f64) -> Wide {
    let mut adjustment = 0;
    if x < f64::MIN_POSITIVE {
        x *= 18_014_398_509_481_984.0;
        adjustment = -54;
    }
    let bits = x.to_bits();
    let mut exponent = i32::try_from((bits >> 52) & 0x7ff)
        .unwrap_or(0)
        .saturating_sub(1023)
        .saturating_add(adjustment);
    let mut m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    if m > core::f64::consts::SQRT_2 {
        m *= 0.5;
        exponent = exponent.saturating_add(1);
    }
    let numerator = Wide(m - 1.0, 0.0);
    let denominator = Wide(m, 0.0).add(Wide(1.0, 0.0));
    let z = numerator.div(denominator);
    let square = z.mul(z);
    let mut term = z;
    let mut sum = z;
    for k in 1i32..25 {
        term = term.mul(square);
        sum = sum.add(term.div(Wide(f64::from(k.saturating_mul(2).saturating_add(1)), 0.0)));
    }
    sum.mul(Wide(2.0, 0.0))
        .add(LN2.mul(Wide(f64::from(exponent), 0.0)))
}
#[expect(
    clippy::cast_possible_truncation,
    clippy::as_conversions,
    reason = "range-checked exponential reduction is in [-1075,1024]"
)]
fn positive_pow(base: f64, exponent: f64) -> f64 {
    let log = logarithm(base);
    let approximate = log.0 * exponent;
    if approximate > 710.0 {
        return f64::INFINITY;
    }
    if approximate < -746.0 {
        return 0.0;
    }
    let value = log.mul(Wide(exponent, 0.0));
    let units = value.0 * core::f64::consts::LOG2_E;
    let n = (units + if units >= 0.0 { 0.5 } else { -0.5 }) as i32;
    let r = value.add(LN2.mul(Wide(f64::from(n), 0.0)).neg());
    let mut sum = Wide(1.0, 0.0);
    let mut term = sum;
    for k in 1..27 {
        term = term.mul(r).div(Wide(f64::from(k), 0.0));
        sum = sum.add(term);
    }
    let significand = sum.0 + sum.1;
    if n > 1023 {
        (significand * 2.0) * power_of_two(n.saturating_sub(1))
    } else if n < -1022 {
        (significand * power_of_two(n.saturating_add(1022))) * f64::MIN_POSITIVE
    } else {
        significand * power_of_two(n)
    }
}
fn power_of_two(exponent: i32) -> f64 {
    f64::from_bits(u64::try_from(exponent.saturating_add(1023)).unwrap_or(0) << 52)
}

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests;
