// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![expect(
    clippy::as_conversions,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "limb arithmetic of an arbitrary-precision integer, whose values are exact"
)]

//! The `BigInt` type of 6.1.6.2.
//!
//! A value is a sign and a magnitude of 32-bit limbs, least significant first,
//! normalized so that zero carries an empty magnitude and no sign. The
//! operations are the ones 6.1.6.2 names, on the mathematical value rather
//! than on a width, except for the bitwise ones, which 6.1.6.2.18 to
//! 6.1.6.2.23 define over the two's complement of an infinite width.

use alloc::{string::String, vec::Vec};

/// An arbitrary-precision integer.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BigIntValue {
    /// Whether the value is below zero. Zero never carries it.
    negative: bool,
    /// The magnitude in 32-bit limbs, least significant first, with no
    /// trailing zero limb.
    limbs: Vec<u32>,
}

/// What an operation of 6.1.6.2 refuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BigIntError {
    /// 6.1.6.2.5 and 6.1.6.2.6 refuse a divisor of zero.
    DivisionByZero,
    /// 6.1.6.2.3 refuses a negative exponent.
    NegativeExponent,
    /// 6.1.6.2.9 and 6.1.6.2.10 have no shift of a width this embedding holds.
    TooLarge,
}

impl BigIntValue {
    /// The value zero.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            negative: false,
            limbs: Vec::new(),
        }
    }

    /// Whether the value is zero.
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    /// Whether the value is below zero.
    #[must_use]
    pub const fn is_negative(&self) -> bool {
        self.negative
    }

    /// The pair a registry keys a value by: its sign and its limbs.
    #[must_use]
    pub fn key(&self) -> (bool, Vec<u32>) {
        (self.negative, self.limbs.clone())
    }

    /// The value one of the two i64 ends carries.
    #[must_use]
    pub fn from_i64(value: i64) -> Self {
        let negative = value < 0;
        let magnitude = value.unsigned_abs();
        let mut limbs = alloc::vec![magnitude as u32, (magnitude >> 32) as u32];
        Self::trim(&mut limbs);
        Self { negative, limbs }
    }

    /// The value, when it fits in an i64.
    #[must_use]
    pub fn to_i64(&self) -> Option<i64> {
        if self.limbs.len() > 2 {
            return None;
        }
        let mut magnitude = 0u64;
        for (index, limb) in self.limbs.iter().enumerate() {
            magnitude |= u64::from(*limb) << (index * 32);
        }
        if self.negative {
            (magnitude <= 1u64 << 63).then(|| (magnitude as i64).wrapping_neg())
        } else {
            i64::try_from(magnitude).ok()
        }
    }

    /// The Number 7.1.13 answers for the value, rounded to the nearer
    /// binary64.
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        let mut value = 0.0f64;
        for limb in self.limbs.iter().rev() {
            value = value * 4_294_967_296.0 + f64::from(*limb);
        }
        if self.negative { -value } else { value }
    }

    /// The value of an integral Number, which 7.1.14 takes and no other.
    #[must_use]
    pub fn from_f64(value: f64) -> Option<Self> {
        if !value.is_finite() || value != truncate(value) {
            return None;
        }
        let negative = value < 0.0;
        let mut magnitude = value.abs();
        let mut limbs = Vec::new();
        while magnitude >= 1.0 {
            let rest = magnitude % 4_294_967_296.0;
            limbs.push(rest as u32);
            magnitude = (magnitude - rest) / 4_294_967_296.0;
        }
        Self::trim(&mut limbs);
        Some(Self {
            negative: negative && !limbs.is_empty(),
            limbs,
        })
    }

    /// `StringToBigInt` of 7.1.14, for text with the radix already stripped.
    ///
    /// Answers `None` for a digit the radix has not got.
    #[must_use]
    pub fn from_digits(digits: &[u16], radix: u32, negative: bool) -> Option<Self> {
        if digits.is_empty() {
            return None;
        }
        let mut value = Self::zero();
        for unit in digits {
            let digit = digit_value(*unit, radix)?;
            value = value.mul_small(radix).add_small(digit);
        }
        value.negative = negative && !value.limbs.is_empty();
        Some(value)
    }

    /// `BigInt::toString` of 6.1.6.2.24, in a radix of 2 to 36.
    #[must_use]
    pub fn to_text(&self, radix: u32) -> String {
        if self.is_zero() {
            return String::from("0");
        }
        let mut limbs = self.limbs.clone();
        let mut digits = Vec::new();
        while !limbs.is_empty() {
            let mut rest = 0u64;
            for limb in limbs.iter_mut().rev() {
                let value = (rest << 32) | u64::from(*limb);
                *limb = (value / u64::from(radix)) as u32;
                rest = value % u64::from(radix);
            }
            digits.push(char::from_digit(rest as u32, radix).unwrap_or('0'));
            Self::trim(&mut limbs);
        }
        let mut text = String::new();
        if self.negative {
            text.push('-');
        }
        text.extend(digits.iter().rev());
        text
    }

    /// 6.1.6.2.8, on the mathematical value.
    #[must_use]
    pub fn compare(&self, other: &Self) -> core::cmp::Ordering {
        use core::cmp::Ordering;
        match (self.negative, other.negative) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => compare_magnitude(&self.limbs, &other.limbs),
            (true, true) => compare_magnitude(&other.limbs, &self.limbs),
        }
    }

    /// 6.1.6.2.1: the value with the opposite sign.
    #[must_use]
    pub fn negate(&self) -> Self {
        Self {
            negative: !self.negative && !self.limbs.is_empty(),
            limbs: self.limbs.clone(),
        }
    }

    /// 6.1.6.2.7.
    #[must_use]
    pub fn add(&self, other: &Self) -> Self {
        if self.negative == other.negative {
            return Self::normalized(self.negative, add_magnitude(&self.limbs, &other.limbs));
        }
        match compare_magnitude(&self.limbs, &other.limbs) {
            core::cmp::Ordering::Less => {
                Self::normalized(other.negative, sub_magnitude(&other.limbs, &self.limbs))
            }
            _ => Self::normalized(self.negative, sub_magnitude(&self.limbs, &other.limbs)),
        }
    }

    /// 6.1.6.2.8 answers the difference through the sum of the negation.
    #[must_use]
    pub fn sub(&self, other: &Self) -> Self {
        self.add(&other.negate())
    }

    /// 6.1.6.2.4.
    #[must_use]
    pub fn mul(&self, other: &Self) -> Self {
        Self::normalized(
            self.negative != other.negative,
            mul_magnitude(&self.limbs, &other.limbs),
        )
    }

    /// 6.1.6.2.5, which rounds toward zero, and 6.1.6.2.6, whose sign is the
    /// sign of the dividend.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::DivisionByZero`] for a divisor of zero.
    pub fn divide(&self, other: &Self) -> Result<(Self, Self), BigIntError> {
        if other.is_zero() {
            return Err(BigIntError::DivisionByZero);
        }
        let (quotient, remainder) = divrem_magnitude(&self.limbs, &other.limbs);
        Ok((
            Self::normalized(self.negative != other.negative, quotient),
            Self::normalized(self.negative, remainder),
        ))
    }

    /// 6.1.6.2.3.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::NegativeExponent`] for an exponent below zero
    /// and [`BigIntError::TooLarge`] for one this embedding cannot hold.
    pub fn power(&self, exponent: &Self) -> Result<Self, BigIntError> {
        if exponent.negative {
            return Err(BigIntError::NegativeExponent);
        }
        let Some(mut count) = exponent
            .to_i64()
            .and_then(|value| u64::try_from(value).ok())
        else {
            return Err(BigIntError::TooLarge);
        };
        // A result past this many limbs is past what the embedding holds.
        if count.saturating_mul(self.limbs.len().max(1) as u64) > 1 << 20 {
            return Err(BigIntError::TooLarge);
        }
        let mut result = Self::from_i64(1);
        let mut base = self.clone();
        while count > 0 {
            if count & 1 == 1 {
                result = result.mul(&base);
            }
            count >>= 1;
            if count > 0 {
                base = base.mul(&base);
            }
        }
        Ok(result)
    }

    /// 6.1.6.2.9 and 6.1.6.2.10, which share one operation: a negative shift
    /// is the other direction.
    ///
    /// # Errors
    ///
    /// Returns [`BigIntError::TooLarge`] for a width this embedding cannot
    /// hold.
    pub fn shift(&self, places: &Self) -> Result<Self, BigIntError> {
        let Some(places) = places.to_i64() else {
            return Err(BigIntError::TooLarge);
        };
        if places >= 0 {
            let places = usize::try_from(places).map_err(|_| BigIntError::TooLarge)?;
            if places > 1 << 25 {
                return Err(BigIntError::TooLarge);
            }
            return Ok(Self::normalized(
                self.negative,
                shift_left(&self.limbs, places),
            ));
        }
        let places = usize::try_from(-places).map_err(|_| BigIntError::TooLarge)?;
        // 6.1.6.2.10 rounds toward negative infinity, so a negative value that
        // loses a set bit takes one away.
        let shifted = shift_right(&self.limbs, places);
        if !self.negative {
            return Ok(Self::normalized(false, shifted));
        }
        let lost = shift_left(&shifted, places) != self.limbs;
        let magnitude = if lost {
            add_magnitude(&shifted, &[1])
        } else {
            shifted
        };
        Ok(Self::normalized(true, magnitude))
    }

    /// 6.1.6.2.18, 6.1.6.2.20 and 6.1.6.2.22 over the two's complement of an
    /// infinite width.
    #[must_use]
    pub fn bitwise(&self, other: &Self, op: BitOp) -> Self {
        let width = self.limbs.len().max(other.limbs.len()) + 1;
        let left = self.two_complement(width);
        let right = other.two_complement(width);
        let mut limbs = Vec::with_capacity(width);
        for index in 0..width {
            let a = left.get(index).copied().unwrap_or(0);
            let b = right.get(index).copied().unwrap_or(0);
            limbs.push(match op {
                BitOp::And => a & b,
                BitOp::Or => a | b,
                BitOp::Xor => a ^ b,
            });
        }
        Self::from_two_complement(limbs)
    }

    /// 6.1.6.2.2, which is the complement of the value less one.
    #[must_use]
    pub fn not(&self) -> Self {
        self.negate().sub(&Self::from_i64(1))
    }

    /// `BigInt.asIntN` of 21.2.2.1 and `BigInt.asUintN` of 21.2.2.2.
    #[must_use]
    pub fn as_n(&self, bits: u64, signed: bool) -> Self {
        if bits == 0 {
            return Self::zero();
        }
        let Ok(bits) = usize::try_from(bits) else {
            return self.clone();
        };
        let width = bits.div_ceil(32) + 1;
        let mut limbs = self.two_complement(width);
        // Keep the low `bits` bits and clear every one above them.
        for index in 0..limbs.len() {
            let low = index * 32;
            if low >= bits {
                if let Some(limb) = limbs.get_mut(index) {
                    *limb = 0;
                }
            } else if bits - low < 32
                && let Some(limb) = limbs.get_mut(index)
            {
                *limb &= (1u32 << (bits - low)) - 1;
            }
        }
        let top = bits - 1;
        let set = limbs
            .get(top / 32)
            .is_some_and(|limb| limb & (1u32 << (top % 32)) != 0);
        if signed && set {
            // Sign-extend above the width, which makes the two's complement of
            // a negative value.
            for index in 0..limbs.len() {
                let low = index * 32;
                if low >= bits {
                    if let Some(limb) = limbs.get_mut(index) {
                        *limb = u32::MAX;
                    }
                } else if bits - low < 32
                    && let Some(limb) = limbs.get_mut(index)
                {
                    *limb |= !((1u32 << (bits - low)) - 1);
                }
            }
            return Self::from_two_complement(limbs);
        }
        Self::trim(&mut limbs);
        Self::normalized(false, limbs)
    }

    /// The two's complement of the value in `width` limbs.
    fn two_complement(&self, width: usize) -> Vec<u32> {
        let mut limbs = alloc::vec![0u32; width];
        for (index, limb) in self.limbs.iter().enumerate() {
            if let Some(slot) = limbs.get_mut(index) {
                *slot = *limb;
            }
        }
        if !self.negative {
            return limbs;
        }
        let mut carry = 1u64;
        for limb in &mut limbs {
            let value = u64::from(!*limb) + carry;
            *limb = value as u32;
            carry = value >> 32;
        }
        limbs
    }

    /// The value a two's complement of this width denotes.
    fn from_two_complement(mut limbs: Vec<u32>) -> Self {
        let negative = limbs.last().is_some_and(|limb| limb & 0x8000_0000 != 0);
        if negative {
            let mut carry = 1u64;
            for limb in &mut limbs {
                let value = u64::from(!*limb) + carry;
                *limb = value as u32;
                carry = value >> 32;
            }
        }
        Self::trim(&mut limbs);
        Self::normalized(negative, limbs)
    }

    fn normalized(negative: bool, mut limbs: Vec<u32>) -> Self {
        Self::trim(&mut limbs);
        Self {
            negative: negative && !limbs.is_empty(),
            limbs,
        }
    }

    fn trim(limbs: &mut Vec<u32>) {
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
    }

    fn mul_small(&self, factor: u32) -> Self {
        let mut limbs = Vec::with_capacity(self.limbs.len() + 1);
        let mut carry = 0u64;
        for limb in &self.limbs {
            let value = u64::from(*limb) * u64::from(factor) + carry;
            limbs.push(value as u32);
            carry = value >> 32;
        }
        if carry != 0 {
            limbs.push(carry as u32);
        }
        Self::normalized(self.negative, limbs)
    }

    fn add_small(&self, addend: u32) -> Self {
        Self::normalized(self.negative, add_magnitude(&self.limbs, &[addend]))
    }
}

/// The three operations 6.1.6.2 defines over the two's complement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitOp {
    /// 6.1.6.2.20.
    And,
    /// 6.1.6.2.22.
    Or,
    /// 6.1.6.2.18.
    Xor,
}

/// The value of one digit in this radix.
fn digit_value(unit: u16, radix: u32) -> Option<u32> {
    let value = match unit {
        0x30..=0x39 => u32::from(unit) - 0x30,
        0x41..=0x5A => u32::from(unit) - 0x41 + 10,
        0x61..=0x7A => u32::from(unit) - 0x61 + 10,
        _ => return None,
    };
    (value < radix).then_some(value)
}

/// The value with its fraction removed, which `no_std` has no `trunc` for.
///
/// A magnitude at or above 2^53 carries no fraction, so it answers itself.
fn truncate(value: f64) -> f64 {
    if !value.is_finite() || value.abs() >= 9_007_199_254_740_992.0 {
        return value;
    }
    (value as i64) as f64
}

fn compare_magnitude(left: &[u32], right: &[u32]) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    if left.len() != right.len() {
        return left.len().cmp(&right.len());
    }
    for index in (0..left.len()).rev() {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        if a != b {
            return a.cmp(&b);
        }
    }
    Ordering::Equal
}

fn add_magnitude(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut limbs = Vec::with_capacity(left.len().max(right.len()) + 1);
    let mut carry = 0u64;
    for index in 0..left.len().max(right.len()) {
        let a = u64::from(left.get(index).copied().unwrap_or(0));
        let b = u64::from(right.get(index).copied().unwrap_or(0));
        let value = a + b + carry;
        limbs.push(value as u32);
        carry = value >> 32;
    }
    if carry != 0 {
        limbs.push(carry as u32);
    }
    limbs
}

/// The difference of two magnitudes, where the left one is the larger.
fn sub_magnitude(left: &[u32], right: &[u32]) -> Vec<u32> {
    let mut limbs = Vec::with_capacity(left.len());
    let mut borrow = 0i64;
    for index in 0..left.len() {
        let a = i64::from(left.get(index).copied().unwrap_or(0));
        let b = i64::from(right.get(index).copied().unwrap_or(0));
        let mut value = a - b - borrow;
        if value < 0 {
            value += 1 << 32;
            borrow = 1;
        } else {
            borrow = 0;
        }
        limbs.push(value as u32);
    }
    limbs
}

fn mul_magnitude(left: &[u32], right: &[u32]) -> Vec<u32> {
    if left.is_empty() || right.is_empty() {
        return Vec::new();
    }
    let mut limbs = alloc::vec![0u32; left.len() + right.len()];
    for (i, a) in left.iter().enumerate() {
        let mut carry = 0u64;
        for (j, b) in right.iter().enumerate() {
            let at = i + j;
            let held = u64::from(limbs.get(at).copied().unwrap_or(0));
            let value = held + u64::from(*a) * u64::from(*b) + carry;
            if let Some(slot) = limbs.get_mut(at) {
                *slot = value as u32;
            }
            carry = value >> 32;
        }
        let mut at = i + right.len();
        while carry != 0 {
            let held = u64::from(limbs.get(at).copied().unwrap_or(0));
            let value = held + carry;
            if let Some(slot) = limbs.get_mut(at) {
                *slot = value as u32;
            }
            carry = value >> 32;
            at += 1;
        }
    }
    limbs
}

/// Schoolbook division of two magnitudes, bit by bit.
///
/// The cost is O(n·m) in the bits of the dividend and the limbs of the
/// divisor, which is what the sizes of this embedding call for.
fn divrem_magnitude(left: &[u32], right: &[u32]) -> (Vec<u32>, Vec<u32>) {
    if compare_magnitude(left, right) == core::cmp::Ordering::Less {
        return (Vec::new(), left.to_vec());
    }
    let mut quotient = alloc::vec![0u32; left.len()];
    let mut remainder: Vec<u32> = Vec::new();
    for bit in (0..left.len() * 32).rev() {
        remainder = shift_left(&remainder, 1);
        let set = left
            .get(bit / 32)
            .is_some_and(|limb| limb & (1u32 << (bit % 32)) != 0);
        if set {
            remainder = add_magnitude(&remainder, &[1]);
        }
        if compare_magnitude(&remainder, right) != core::cmp::Ordering::Less {
            remainder = sub_magnitude(&remainder, right);
            while remainder.last() == Some(&0) {
                remainder.pop();
            }
            if let Some(slot) = quotient.get_mut(bit / 32) {
                *slot |= 1u32 << (bit % 32);
            }
        }
    }
    (quotient, remainder)
}

fn shift_left(limbs: &[u32], places: usize) -> Vec<u32> {
    if limbs.is_empty() {
        return Vec::new();
    }
    let whole = places / 32;
    let bits = places % 32;
    let mut out = alloc::vec![0u32; whole];
    let mut carry = 0u32;
    for limb in limbs {
        if bits == 0 {
            out.push(*limb);
        } else {
            out.push((*limb << bits) | carry);
            carry = *limb >> (32 - bits);
        }
    }
    if carry != 0 {
        out.push(carry);
    }
    while out.last() == Some(&0) {
        out.pop();
    }
    out
}

fn shift_right(limbs: &[u32], places: usize) -> Vec<u32> {
    let whole = places / 32;
    if whole >= limbs.len() {
        return Vec::new();
    }
    let bits = places % 32;
    let kept = limbs.get(whole..).unwrap_or_default();
    let mut out = Vec::with_capacity(kept.len());
    for (index, limb) in kept.iter().enumerate() {
        let value = if bits == 0 {
            *limb
        } else {
            let next = kept.get(index + 1).copied().unwrap_or(0);
            (*limb >> bits) | (next << (32 - bits))
        };
        out.push(value);
    }
    while out.last() == Some(&0) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{BigIntValue, BitOp};
    use alloc::{string::String, vec::Vec};

    fn parse(text: &str) -> BigIntValue {
        let negative = text.starts_with('-');
        let digits: Vec<u16> = text.trim_start_matches('-').encode_utf16().collect();
        BigIntValue::from_digits(&digits, 10, negative).expect("decimal digits")
    }

    fn text(value: &BigIntValue) -> String {
        value.to_text(10)
    }

    #[test]
    fn the_text_of_6_1_6_2_24_reads_back_what_7_1_14_parsed() {
        for source in [
            "0",
            "1",
            "-1",
            "4294967296",
            "-4294967297",
            "9007199254740993",
            "123456789012345678901234567890",
            "-99999999999999999999999999999999999999",
        ] {
            assert_eq!(text(&parse(source)), source, "{source}");
        }
        assert_eq!(parse("255").to_text(16), "ff");
        assert_eq!(parse("-255").to_text(2), "-11111111");
    }

    #[test]
    fn the_arithmetic_of_6_1_6_2_carries_across_limbs() {
        let big = parse("123456789012345678901234567890");
        let small = parse("987654321");
        assert_eq!(text(&big.add(&small)), "123456789012345678902222222211");
        assert_eq!(text(&big.sub(&small)), "123456789012345678900246913569");
        assert_eq!(
            text(&big.mul(&small)),
            "121932631124828532112482853211126352690"
        );
        let (quotient, remainder) = big.divide(&small).expect("a divisor that is not zero");
        assert_eq!(text(&quotient), "124999998873437499901");
        assert_eq!(text(&remainder), "574845669");
        assert_eq!(
            text(&quotient.mul(&small).add(&remainder)),
            text(&big),
            "the quotient and the remainder rebuild the dividend"
        );
    }

    #[test]
    fn a_division_of_6_1_6_2_5_rounds_toward_zero() {
        for (left, right, quotient, remainder) in [
            ("7", "2", "3", "1"),
            ("-7", "2", "-3", "-1"),
            ("7", "-2", "-3", "1"),
            ("-7", "-2", "3", "-1"),
        ] {
            let (q, r) = parse(left).divide(&parse(right)).expect("a divisor");
            assert_eq!(text(&q), quotient, "{left}/{right}");
            assert_eq!(text(&r), remainder, "{left}%{right}");
        }
    }

    #[test]
    fn the_bitwise_operations_of_6_1_6_2_run_on_the_two_complement() {
        for (left, right, and, or, xor) in [
            ("12", "10", "8", "14", "6"),
            ("-12", "10", "0", "-2", "-2"),
            ("-12", "-10", "-12", "-10", "2"),
            ("0", "-1", "0", "-1", "-1"),
        ] {
            let a = parse(left);
            let b = parse(right);
            assert_eq!(text(&a.bitwise(&b, BitOp::And)), and, "{left}&{right}");
            assert_eq!(text(&a.bitwise(&b, BitOp::Or)), or, "{left}|{right}");
            assert_eq!(text(&a.bitwise(&b, BitOp::Xor)), xor, "{left}^{right}");
        }
        assert_eq!(text(&parse("5").not()), "-6");
        assert_eq!(text(&parse("-6").not()), "5");
    }

    #[test]
    fn a_shift_of_6_1_6_2_10_rounds_toward_negative_infinity() {
        for (value, places, answer) in [
            ("1", "10", "1024"),
            ("-1", "10", "-1024"),
            ("1024", "-10", "1"),
            ("-7", "-1", "-4"),
            ("7", "-1", "3"),
            ("-1", "-1", "-1"),
        ] {
            let shifted = parse(value).shift(&parse(places)).expect("a width");
            assert_eq!(text(&shifted), answer, "{value}<<{places}");
        }
    }

    #[test]
    fn the_widths_of_21_2_2_take_the_low_bits() {
        assert_eq!(text(&parse("255").as_n(8, false)), "255");
        assert_eq!(text(&parse("255").as_n(8, true)), "-1");
        assert_eq!(text(&parse("256").as_n(8, false)), "0");
        assert_eq!(text(&parse("-1").as_n(64, false)), "18446744073709551615");
        assert_eq!(text(&parse("-1").as_n(64, true)), "-1");
        assert_eq!(text(&parse("12345").as_n(0, true)), "0");
    }

    #[test]
    fn the_power_of_6_1_6_2_3_squares_by_halving_the_exponent() {
        assert_eq!(
            text(&parse("2").power(&parse("64")).expect("an exponent")),
            "18446744073709551616"
        );
        assert_eq!(
            text(&parse("-3").power(&parse("3")).expect("an exponent")),
            "-27"
        );
        assert_eq!(
            text(&parse("5").power(&parse("0")).expect("an exponent")),
            "1"
        );
    }

    #[test]
    fn the_numbers_of_7_1_13_and_7_1_14_agree_where_both_hold_the_value() {
        for source in ["0", "1", "-1", "9007199254740992", "-4294967296"] {
            let value = parse(source);
            let number = value.to_f64();
            assert_eq!(
                text(&BigIntValue::from_f64(number).expect("an integral Number")),
                source
            );
        }
        assert!(BigIntValue::from_f64(1.5).is_none());
        assert!(BigIntValue::from_f64(f64::NAN).is_none());
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one table of cases the mathematical value was taken from"
    )]
    fn the_operations_of_6_1_6_2_agree_with_the_mathematical_value() {
        // Left, right, sum, difference, product, quotient, remainder, and,
        // or, exclusive or.
        for (left, right, sum, difference, product, quotient, remainder, and, or, xor) in [
            (
                "-932652146082171377906",
                "421610615998047273",
                "-932230535466173330633",
                "-933073756698169425179",
                "-393216045821605046229254849827735750338",
                "-2212",
                "-49463494490810030",
                "61313759117845512",
                "-932291849225291176145",
                "-932353162984409021657",
            ),
            (
                "-93620443439254124079632210824806638188",
                "9832976175283",
                "-93620443439254124079632200991830462905",
                "-93620443439254124079632220657782813471",
                "-920567589857615447338826248917436687947869449507204",
                "-9521068877862877725033948",
                "-4678433130904",
                "9558047084688",
                "-93620443439254124079632210549877547593",
                "-93620443439254124079632220107924632281",
            ),
            (
                "-2352542074769714",
                "1141153371300629829",
                "1138800829225860115",
                "-1143505913375399543",
                "-2684611319750037501459218334198906",
                "0",
                "-2352542074769714",
                "1141135152543435844",
                "-2334323317575729",
                "-1143469475861011573",
            ),
            (
                "27615123736804472220734702580405164564",
                "153540110946965095",
                "27615123736804472220888242691352129659",
                "27615123736804472220581162469458199469",
                "4240029162363127986123352547945591410574783839838893580",
                "179856088200581819119",
                "10998093108513259",
                "153452138838921732",
                "27615123736804472220734790552513207927",
                "27615123736804472220581338413674286195",
            ),
            (
                "145819722542393046346360484",
                "-6666186686165173612",
                "145819715876206360181186872",
                "145819729208579732511534096",
                "-972061492992400166406254085586027788196348208",
                "-21874533",
                "1891712312719937288",
                "145819715876220757990548100",
                "-14397809361228",
                "-145819715876235155799909328",
            ),
            (
                "63360158607731480922830283788771253481",
                "72390762004528402",
                "63360158607731480922902674550775781883",
                "63360158607731480922757893026766725079",
                "4586690162341461264351715893819304585954381843805867362",
                "875251991459462576926",
                "44512924494401229",
                "72344581873638400",
                "63360158607731480922830329968902143483",
                "63360158607731480922757985387028505083",
            ),
            (
                "-2764746461320922930301629200740161103",
                "-637812972322523368",
                "-2764746461320922930939442173062684471",
                "-2764746461320922929663816228417637735",
                "1763391158213276240329936769315377458573092967802154904",
                "4334729115422995029",
                "-40027911339823431",
                "-2764746461320922930356084732462866160",
                "-583357440599818311",
                "2764746461320922929772727291863047849",
            ),
            (
                "51002710797876073028924562290430617479",
                "-6182556",
                "51002710797876073028924562290424434923",
                "51002710797876073028924562290436800035",
                "-315327115659673502561415726136075556678496324",
                "-8249453914833294357370084846854",
                "4338655",
                "51002710797876073028924562290426028292",
                "-1593369",
                "-51002710797876073028924562290427621661",
            ),
            (
                "-4591190598193489",
                "-50499935514197305",
                "-55091126112390794",
                "45908744916003816",
                "231854829142160144255644912347145",
                "0",
                "-4591190598193489",
                "-50507088785571193",
                "-4584037326819601",
                "45923051458751592",
            ),
            (
                "-94200429570078577656204051204",
                "-78096721883236306",
                "-94200429570156674378087287510",
                "-94200429570000480934320814898",
                "7356744749415816064382772764748306251655812424",
                "1206202095280",
                "-19517851636815524",
                "-94200429570155178483859015636",
                "-1495894228271874",
                "94200429570153682589630743762",
            ),
            (
                "-87429008808003044271637164098409",
                "-3421073618955849093375286",
                "-87429012229076663227486257473695",
                "-87429005386929425315788070723123",
                "299101075564517780825213531773207245074844030158872519974",
                "25556015",
                "-85863080503601954453119",
                "-87429012210177595044322270963582",
                "-18899068183163986510113",
                "87429012191278526861158284453469",
            ),
            (
                "155238630618278769687763721627839632459",
                "509058210",
                "155238630618278769687763721628348690669",
                "155238630618278769687763721627330574249",
                "79025499425392183778255259034806329466636438390",
                "304952611643919404988603801572",
                "402126339",
                "134616066",
                "155238630618278769687763721628214074603",
                "155238630618278769687763721628079458537",
            ),
            (
                "-4166",
                "-32654222294810556526797",
                "-32654222294810556530963",
                "32654222294810556522631",
                "136037490080180778490636302",
                "0",
                "-4166",
                "-32654222294810556526798",
                "-4165",
                "32654222294810556522633",
            ),
            (
                "6712053079869640233783944",
                "189203426030111",
                "6712053080058843659814055",
                "6712053079680436807753833",
                "1269943438407294195907317885671612337784",
                "35475325266",
                "9881398699418",
                "175992059269640",
                "6712053079882851600544415",
                "6712053079706859541274775",
            ),
        ] {
            let a = parse(left);
            let b = parse(right);
            assert_eq!(text(&a.add(&b)), sum, "{left}+{right}");
            assert_eq!(text(&a.sub(&b)), difference, "{left}-{right}");
            assert_eq!(text(&a.mul(&b)), product, "{left}*{right}");
            let (q, r) = a.divide(&b).expect("a divisor that is not zero");
            assert_eq!(text(&q), quotient, "{left}/{right}");
            assert_eq!(text(&r), remainder, "{left}%{right}");
            assert_eq!(text(&a.bitwise(&b, BitOp::And)), and, "{left}&{right}");
            assert_eq!(text(&a.bitwise(&b, BitOp::Or)), or, "{left}|{right}");
            assert_eq!(text(&a.bitwise(&b, BitOp::Xor)), xor, "{left}^{right}");
        }
    }
}
