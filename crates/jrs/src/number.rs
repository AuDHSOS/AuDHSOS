// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! ECMA-262 6.1.6.1.20 radix-10 notation and nearest-even decimal ties.
//! The toolchain provides shortest round-tripping digits. Its tie choice is
//! corrected using exact integer arithmetic, never another floating operation.

use alloc::{format, string::String, string::ToString, vec::Vec};
use core::fmt;

struct Decimal(f64);

impl fmt::Display for Decimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        decimal(self.0, formatter)
    }
}

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

pub(crate) fn decimal_string(value: f64) -> String {
    if value.is_nan() {
        return String::from("NaN");
    }
    if value == f64::INFINITY {
        return String::from("Infinity");
    }
    if value == f64::NEG_INFINITY {
        return String::from("-Infinity");
    }
    if value == 0.0 {
        return String::from("0");
    }
    let decimal = Decimal(value);
    format!("{decimal}")
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

/// `parseInt` of 19.2.5 over the code units of its first argument.
///
/// `radix` is the `ToInt32` of its second one; zero means the clause picks it.
pub(crate) fn parse_integer(units: &[u16], radix: i32) -> f64 {
    if radix != 0 && !(2..=36).contains(&radix) {
        return f64::NAN;
    }
    let mut text = units;
    while text.first().is_some_and(|unit| space(*unit)) {
        text = text.get(1..).unwrap_or(&[]);
    }
    let negative = text.first() == Some(&45);
    if matches!(text.first(), Some(43 | 45)) {
        text = text.get(1..).unwrap_or(&[]);
    }
    let mut radix = u32::try_from(radix).unwrap_or(0);
    if (radix == 0 || radix == 16) && matches!(text.get(..2), Some([48, 88 | 120])) {
        text = text.get(2..).unwrap_or(&[]);
        radix = 16;
    }
    if radix == 0 {
        radix = 10;
    }
    let Some(mut integer) = audhsos_math::RadixInteger::new(radix) else {
        return f64::NAN;
    };
    let mut found = false;
    for unit in text {
        let Some(digit) = char::from_u32(u32::from(*unit)).and_then(|c| c.to_digit(radix)) else {
            break;
        };
        integer.push(digit);
        found = true;
    }
    let value = if found { integer.to_f64() } else { f64::NAN };
    if negative { -value } else { value }
}

/// `parseFloat` of 19.2.4 over the code units of its argument.
pub(crate) fn parse_float(units: &[u16]) -> f64 {
    let mut text = units;
    while text.first().is_some_and(|unit| space(*unit)) {
        text = text.get(1..).unwrap_or(&[]);
    }
    let mut end = usize::from(matches!(text.first(), Some(43 | 45)));
    let infinity = [73, 110, 102, 105, 110, 105, 116, 121];
    if text
        .get(end..)
        .is_some_and(|rest| rest.starts_with(&infinity))
    {
        return if text.first() == Some(&45) {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    let start = end;
    while text.get(end).is_some_and(|unit| digit(*unit)) {
        end = end.saturating_add(1);
    }
    let mut digits = end.saturating_sub(start);
    if text.get(end) == Some(&46) {
        end = end.saturating_add(1);
        let start = end;
        while text.get(end).is_some_and(|unit| digit(*unit)) {
            end = end.saturating_add(1);
        }
        digits = digits.saturating_add(end.saturating_sub(start));
    }
    if digits == 0 {
        return f64::NAN;
    }
    if matches!(text.get(end), Some(69 | 101)) {
        let mark = end;
        end = end.saturating_add(1);
        if matches!(text.get(end), Some(43 | 45)) {
            end = end.saturating_add(1);
        }
        let start = end;
        while text.get(end).is_some_and(|unit| digit(*unit)) {
            end = end.saturating_add(1);
        }
        if start == end {
            end = mark;
        }
    }
    let ascii: String = text
        .get(..end)
        .unwrap_or(&[])
        .iter()
        .map(|unit| char::from(u8::try_from(*unit).unwrap_or(0)))
        .collect();
    ascii.parse().unwrap_or(f64::NAN)
}

/// The white space and line terminators 19.2.4 and 19.2.5 skip.
fn space(unit: u16) -> bool {
    char::from_u32(u32::from(unit))
        .is_some_and(|c| crate::value::whitespace(c) || crate::value::line_terminator(c))
}

fn digit(unit: u16) -> bool {
    (48..=57).contains(&unit)
}

/// The exact decimal expansion of a finite value above zero: its digits with
/// no leading and no trailing zero, and how many of them stand before the
/// decimal point.
///
/// A double is an integer times a power of two, so its decimal expansion ends;
/// the smallest subnormal ends after 1074 fraction digits.
fn exact_decimal(value: f64) -> (Vec<u8>, i32) {
    let text = format!("{value:.1100}");
    let mut digits: Vec<u8> = Vec::with_capacity(text.len());
    let mut point: i32 = 0;
    let mut seen_point = false;
    for byte in text.bytes() {
        if byte == b'.' {
            seen_point = true;
            continue;
        }
        if !seen_point {
            point = point.saturating_add(1);
        }
        digits.push(byte.wrapping_sub(b'0'));
    }
    // A leading zero moves the point, a trailing one is not a digit of the
    // expansion at all.
    let leading = digits.iter().take_while(|digit| **digit == 0).count();
    digits.drain(..leading);
    point = point.saturating_sub(i32::try_from(leading).unwrap_or(0));
    while digits.last() == Some(&0) {
        digits.pop();
    }
    (digits, point)
}

/// Keeps `keep` of the digits and rounds the rest away, with a tie going up,
/// which is the larger integer every clause of 21.1.3 asks for.
///
/// Answers the digits kept and whether the carry ran off the front, which
/// moves the decimal point one place right.
fn round_digits(digits: &[u8], keep: usize) -> (Vec<u8>, bool) {
    let mut kept: Vec<u8> = digits.iter().copied().take(keep).collect();
    while kept.len() < keep {
        kept.push(0);
    }
    if digits.get(keep).copied().unwrap_or(0) < 5 {
        return (kept, false);
    }
    for index in (0..kept.len()).rev() {
        let Some(digit) = kept.get_mut(index) else {
            continue;
        };
        if *digit < 9 {
            *digit = digit.saturating_add(1);
            return (kept, false);
        }
        *digit = 0;
    }
    // Every digit was nine, so the answer is one and as many zeros, which the
    // carry says and the caller writes.
    (kept, true)
}

/// `Number.prototype.toFixed` of 21.1.3.3 for a finite value below 10**21.
pub(crate) fn fixed_notation(value: f64, count: usize) -> String {
    if value == 0.0 {
        let mut text = String::from("0");
        if count > 0 {
            text.push('.');
            text.extend(core::iter::repeat_n('0', count));
        }
        return text;
    }
    let (digits, point) = exact_decimal(value);
    let keep = point.saturating_add(i32::try_from(count).unwrap_or(0));
    // A value below half of the last place kept answers zeros alone.
    let (kept, carried) = if keep < 0 {
        (Vec::new(), false)
    } else {
        round_digits(&digits, usize::try_from(keep).unwrap_or(0))
    };
    let mut text = String::new();
    if carried {
        text.push('1');
    }
    for digit in &kept {
        text.push(char::from(b'0'.wrapping_add(*digit)));
    }
    // 21.1.3.3 step 11 pads the integer to one digit and cuts the fraction
    // off the end.
    while text.len() <= count {
        text.insert(0, '0');
    }
    if count > 0 {
        text.insert(text.len().saturating_sub(count), '.');
    }
    text
}

/// The digits and the decimal exponent 21.1.3.2 and 21.1.3.5 answer: `keep`
/// significant digits of the value, and the power of ten the first of them
/// stands at.
fn significant_digits(value: f64, keep: usize) -> (String, i32) {
    let (digits, point) = exact_decimal(value);
    let (kept, carried) = round_digits(&digits, keep);
    let mut text = String::new();
    if carried {
        // The carry made one digit more, and the clause keeps as many as it
        // asked for: a one and zeros, one decimal place further out.
        text.push('1');
        text.extend(core::iter::repeat_n('0', keep.saturating_sub(1)));
    } else {
        for digit in &kept {
            text.push(char::from(b'0'.wrapping_add(*digit)));
        }
    }
    (
        text,
        point.saturating_sub(1).saturating_add(i32::from(carried)),
    )
}

/// `Number.prototype.toExponential` of 21.1.3.2 for a finite value, with the
/// count of fraction digits the call named.
pub(crate) fn exponential_notation(value: f64, count: usize) -> String {
    let (significand, exponent) = if value == 0.0 {
        (
            core::iter::repeat_n('0', count.saturating_add(1)).collect::<String>(),
            0,
        )
    } else {
        significant_digits(value, count.saturating_add(1))
    };
    with_exponent(&significand, exponent, count)
}

/// Joins the significand and the exponent the way 21.1.3.2 step 12 does.
fn with_exponent(significand: &str, exponent: i32, count: usize) -> String {
    let mut text = String::from(significand.get(..1).unwrap_or("0"));
    if count != 0 {
        text.push('.');
        text.push_str(significand.get(1..).unwrap_or_default());
    }
    text.push('e');
    text.push(if exponent < 0 { '-' } else { '+' });
    text.push_str(&exponent.unsigned_abs().to_string());
    text
}

/// The digits 21.1.3.2 answers where the call named no count: the shortest
/// that name the value again, which 6.1.6.1.20 also uses.
pub(crate) fn shortest_exponential(value: f64) -> String {
    if value == 0.0 {
        return String::from("0e+0");
    }
    // The toolchain writes the shortest digits that name the value again,
    // with the point after the first of them, which is where 21.1.3.2 puts it.
    let text = format!("{value:e}");
    let (significand, exponent) = text.split_once('e').unwrap_or((text.as_str(), "0"));
    let digits: String = significand.chars().filter(|unit| *unit != '.').collect();
    let count = digits.len().saturating_sub(1);
    with_exponent(&digits, exponent.parse::<i32>().unwrap_or(0), count)
}

/// `Number.prototype.toPrecision` of 21.1.3.5 for a finite value, with the
/// count of significant digits the call named.
pub(crate) fn precision_notation(value: f64, count: usize) -> String {
    let (significand, exponent) = if value == 0.0 {
        (core::iter::repeat_n('0', count).collect::<String>(), 0)
    } else {
        significant_digits(value, count)
    };
    // Step 12 takes the exponential notation outside the window of 21.1.3.5.
    if exponent < -6 || exponent >= i32::try_from(count).unwrap_or(i32::MAX) {
        return with_exponent(&significand, exponent, count.saturating_sub(1));
    }
    if exponent == i32::try_from(count).unwrap_or(i32::MAX).saturating_sub(1) {
        return significand;
    }
    let mut text = String::new();
    if exponent >= 0 {
        let head = usize::try_from(exponent).unwrap_or(0).saturating_add(1);
        text.push_str(significand.get(..head).unwrap_or(&significand));
        text.push('.');
        text.push_str(significand.get(head..).unwrap_or_default());
        return text;
    }
    text.push_str("0.");
    text.extend(core::iter::repeat_n(
        '0',
        usize::try_from(exponent.saturating_neg())
            .unwrap_or(0)
            .saturating_sub(1),
    ));
    text.push_str(&significand);
    text
}

#[cfg(test)]
mod tests {
    use super::{decimal_string, format_with_radix};

    #[test]
    fn decimal_string_matches_ecmascript_number_keys() {
        for (number, expected) in [
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (f64::NEG_INFINITY, "-Infinity"),
            (-0.0, "0"),
            (1.5, "1.5"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
        ] {
            assert_eq!(decimal_string(number), expected);
        }
    }

    #[test]
    fn a_radix_reaches_every_representation_of_number_to_string() {
        for (number, radix, expected) in [
            (f64::NAN, 2, "NaN"),
            (0.0, 2, "0"),
            (-0.0, 2, "0"),
            (f64::INFINITY, 2, "Infinity"),
            (f64::NEG_INFINITY, 2, "-Infinity"),
            // Radix ten is the decimal form itself.
            (1.5, 10, "1.5"),
            // An integer below 2^53 is converted with integer arithmetic, and
            // one above it with the double the value already is.
            (255.0, 16, "ff"),
            (-10.0, 2, "-1010"),
            (12_345_678_901_234_567_890.0, 16, "ab54a98ceb1f0800"),
            (1e21, 36, "5v1j4f4ds7c4ks"),
            // A fraction adds digits until it is exhausted, and an integer
            // part of zero still leads with one digit.
            (0.5, 2, "0.1"),
            (10.25, 4, "22.1"),
            (255.5, 16, "ff.8"),
        ] {
            assert_eq!(
                format_with_radix(number, radix),
                expected,
                "{number} {radix}"
            );
        }
        // A fraction that never terminates stops after fifty digits.
        let repeating = format_with_radix(0.1, 3);
        assert_eq!(repeating.len(), 52, "{repeating}");
        assert!(repeating.starts_with("0.0022002200"), "{repeating}");
    }
}
