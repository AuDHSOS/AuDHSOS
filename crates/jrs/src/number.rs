// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! ECMA-262 6.1.6.1.20 radix-10 notation and nearest-even decimal ties.
//! The toolchain provides shortest round-tripping digits. Its tie choice is
//! corrected using exact integer arithmetic, never another floating operation.

use alloc::{format, string::String, string::ToString};
use core::fmt;

pub(crate) fn decimal(value: f64, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific.split_once('e').ok_or(fmt::Error)?;
    let exponent: i32 = exponent.parse().map_err(|_| fmt::Error)?;
    let mut digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let count = i32::try_from(digits.len()).map_err(|_| fmt::Error)?;
    let scale = count.saturating_sub(1).saturating_sub(exponent);
    if let Some(even) = even_tie(value.abs(), &digits, scale) {
        digits = even.to_string();
    }
    if value.is_sign_negative() {
        f.write_str("-")?;
    }
    let point = exponent.saturating_add(1);
    if (-5..=21).contains(&point) {
        if point <= 0 {
            f.write_str("0.")?;
            for _ in 0..point.unsigned_abs() {
                f.write_str("0")?;
            }
            f.write_str(&digits)
        } else {
            let point = usize::try_from(point).map_err(|_| fmt::Error)?;
            if point >= digits.len() {
                f.write_str(&digits)?;
                for _ in digits.len()..point {
                    f.write_str("0")?;
                }
                Ok(())
            } else {
                write!(
                    f,
                    "{}.{}",
                    digits.get(..point).ok_or(fmt::Error)?,
                    digits.get(point..).ok_or(fmt::Error)?
                )
            }
        }
    } else {
        f.write_str(digits.get(..1).ok_or(fmt::Error)?)?;
        if digits.len() > 1 {
            write!(f, ".{}", digits.get(1..).ok_or(fmt::Error)?)?;
        }
        write!(f, "e{exponent:+}")
    }
}

fn even_tie(value: f64, digits: &str, scale: i32) -> Option<u128> {
    let significand: u128 = digits.parse().ok()?;
    if significand & 1 == 0 {
        return None;
    }
    // A binary rational can be exactly halfway between decimal grid points
    // only when the decimal scaling cancels enough factors of two. Scales
    // outside this range cannot produce an odd <=17-digit shortest tie.
    if scale.unsigned_abs() > 22 {
        return None;
    }
    let bits = value.to_bits();
    let exponent = i32::try_from((bits >> 52) & 0x7ff)
        .ok()?
        .saturating_sub(1075);
    let mantissa = (bits & 0x000f_ffff_ffff_ffff) | (1u64 << 52);
    let power = 5u128.checked_pow(scale.unsigned_abs())?;
    let (mut numerator, mut denominator) = if scale >= 0 {
        (u128::from(mantissa).checked_mul(power)?, 1u128)
    } else {
        (u128::from(mantissa), power)
    };
    let shift = exponent.saturating_add(scale);
    if shift >= 0 {
        numerator = numerator.checked_mul(1u128.checked_shl(shift.unsigned_abs())?)?;
    } else {
        denominator = denominator.checked_mul(1u128.checked_shl(shift.unsigned_abs())?)?;
    }
    let lower = significand.checked_sub(1)?;
    // numerator / denominator == lower + 1/2, with lower even.
    if numerator.checked_mul(2)?
        == lower
            .checked_mul(2)?
            .checked_add(1)?
            .checked_mul(denominator)?
    {
        Some(lower)
    } else {
        None
    }
}

/// Formats an IEEE-754 double according to ECMA-262 `Number::toString` with the specified radix (2..=36).
#[expect(
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]
#[must_use]
pub(crate) fn format_with_radix(value: f64, radix: u32) -> String {
    if value.is_nan() {
        return String::from("NaN");
    }
    if value == 0.0 {
        return String::from("0");
    }
    if value == f64::INFINITY {
        return String::from("Infinity");
    }
    if value == f64::NEG_INFINITY {
        return String::from("-Infinity");
    }
    if radix == 10 {
        return format!("{value}");
    }
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let negative = value.is_sign_negative();
    let abs_val = value.abs();

    let mut result = String::new();
    if negative {
        result.push('-');
    }

    let int_part = float_floor(abs_val);
    let mut frac_part = abs_val - int_part;

    if int_part == 0.0 {
        result.push('0');
    } else if int_part <= 9_007_199_254_740_991.0 {
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let mut n = int_part as u64;
        let mut chars = alloc::vec::Vec::new();
        let r = u64::from(radix);
        while n > 0 {
            let d = usize::try_from(n % r).unwrap_or(0);
            chars.push(digits[d]);
            n /= r;
        }
        chars.reverse();
        for b in chars {
            result.push(b as char);
        }
    } else {
        let r = f64::from(radix);
        let mut chars = alloc::vec::Vec::new();
        let mut n = int_part;
        while n >= 1.0 {
            let rem = n % r;
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let d = usize::try_from(rem as u64).unwrap_or(0);
            chars.push(digits[d.min(digits.len() - 1)]);
            n = (n - rem) / r;
        }
        chars.reverse();
        for b in chars {
            result.push(b as char);
        }
    }

    if frac_part != 0.0 {
        result.push('.');
        let r = f64::from(radix);
        for _ in 0..50 {
            frac_part *= r;
            let d_float = float_floor(frac_part);
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let d = usize::try_from(d_float as u64).unwrap_or(0);
            result.push(digits[d.min(digits.len() - 1)] as char);
            frac_part -= d_float;
            if frac_part == 0.0 {
                break;
            }
        }
    }

    result
}

#[expect(clippy::as_conversions, clippy::arithmetic_side_effects)]
fn float_trunc(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() || n == 0.0 {
        return n;
    }
    let bits = n.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023;
    if exponent < 0 {
        if n.is_sign_negative() { -0.0 } else { 0.0 }
    } else if exponent >= 52 {
        n
    } else {
        let shift = 52 - exponent;
        let mask = !((1u64 << shift) - 1);
        f64::from_bits(bits & mask)
    }
}

#[expect(clippy::float_cmp)]
fn float_floor(n: f64) -> f64 {
    let trunc = float_trunc(n);
    if n < 0.0 && trunc != n {
        trunc - 1.0
    } else {
        trunc
    }
}
