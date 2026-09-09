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
