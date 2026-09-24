// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Decimal arithmetic of any precision, which is
//! `research/sqlite/ext/misc/decimal.c`.
//!
//! One value is a sign, the digits most significant first with one digit
//! per byte, and how many of the digits stand right of the point. The C
//! library holds a value to `SQLITE_DECIMAL_MAX_DIGIT` digits because an
//! allocation past it fails silently; this crate leaves the count to the
//! allocator, which raises.

use alloc::vec::Vec;
use core::cmp::Ordering;

/// One decimal value, which is `Decimal` of
/// `research/sqlite/ext/misc/decimal.c:39`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decimal {
    /// Whether the value stands below nought.
    sign: bool,
    /// The digits, most significant first, one digit per byte.
    digits: Vec<u8>,
    /// How many of the digits stand right of the point.
    frac: usize,
}

/// How far the exponent of a text is read, which is the bound the loop of
/// `decimalNewFromText` carries.
const EXPONENT: i64 = 1_000_000;

/// How far `decimal_pow2` reads its argument, past which it answers
/// nothing.
const POWERS: i32 = 20_000;

impl Decimal {
    /// The value a text spells, which is `decimalNewFromText` of
    /// `research/sqlite/ext/misc/decimal.c:72`: the space before it, one
    /// sign, the digits with at most one point among them, and an
    /// exponent after `e`. Every other byte is passed over, so a text
    /// that spells no number is nought.
    ///
    /// Reading the text costs O(n) in its bytes and the exponent costs
    /// O(e) in the digits it names.
    #[must_use]
    pub fn of_text(text: &[u8]) -> Self {
        let mut at = 0;
        while text.get(at).is_some_and(u8::is_ascii_whitespace) {
            at = at.saturating_add(1);
        }
        let mut sign = false;
        match text.get(at) {
            Some(&b'-') => {
                sign = true;
                at = at.saturating_add(1);
            }
            Some(&b'+') => at = at.saturating_add(1),
            _ => {}
        }
        while text.get(at) == Some(&b'0') {
            at = at.saturating_add(1);
        }
        let mut digits: Vec<u8> = Vec::new();
        // Where the point stood, counted from one, which is what
        // `p->nFrac` holds until the digits have all been read.
        let mut point = 0_usize;
        let mut exp = 0_i64;
        while let Some(byte) = text.get(at).copied() {
            match byte {
                b'0'..=b'9' => digits.push(byte.wrapping_sub(b'0')),
                b'.' => point = digits.len().saturating_add(1),
                b'e' | b'E' => {
                    exp = exponent_of(text, at.saturating_add(1));
                    break;
                }
                _ => {}
            }
            at = at.saturating_add(1);
        }
        let count = digits.len();
        let mut frac = if point == 0 {
            0
        } else {
            whole(count).saturating_sub(whole(point).saturating_sub(1))
        };
        let mut held = Decimal {
            sign,
            digits,
            frac: 0,
        };
        held.shifted(exp, &mut frac);
        held.frac = usize::try_from(frac).unwrap_or(0);
        // A value of nought carries no sign, which `decimalNewFromText`
        // writes back once the digits are read.
        if held.digits.iter().all(|digit| *digit == 0) {
            held.sign = false;
        }
        held
    }

    /// The digits moved by the exponent `exp`: a positive one writes
    /// zeros after them and a negative one before them, taking as much
    /// of the move as the fraction holds first.
    fn shifted(&mut self, exp: i64, frac: &mut i64) {
        let count = || whole(self.digits.len());
        let mut exp = exp;
        if exp > 0 {
            if *frac > 0 {
                let taken = exp.min(*frac);
                *frac = frac.saturating_sub(taken);
                exp = exp.saturating_sub(taken);
            }
            self.digits.resize(widened(count().saturating_add(exp)), 0);
            return;
        }
        if exp == 0 {
            return;
        }
        exp = exp.saturating_neg();
        // `nExtra` counts the digits left of the point beyond the first,
        // which is below nought for a value of no digit at all.
        let extra = count().saturating_sub(*frac).saturating_sub(1);
        if extra != 0 {
            let taken = extra.min(exp);
            *frac = frac.saturating_add(taken);
            exp = exp.saturating_sub(taken);
        }
        if exp <= 0 {
            return;
        }
        let mut held = alloc::vec![0_u8; widened(exp)];
        held.append(&mut self.digits);
        self.digits = held;
        *frac = frac.saturating_add(exp);
    }

    /// The text of the value, which is `decimal_result` of
    /// `research/sqlite/ext/misc/decimal.c:252`: the sign, the digits
    /// left of the point with the leading zeros of all but the last of
    /// them dropped, and the digits right of it after a point.
    ///
    /// Writing the text costs O(n) in the digits.
    #[must_use]
    pub fn text(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.digits.len().saturating_add(2));
        // `decimal_result` reads the sign off a value of one digit that is
        // nought and off one of no digit at all, and leaves it on every
        // other value, so a sum of two values that cancel writes `-0`.
        if self.sign && !matches!(self.digits.as_slice(), [] | [0]) {
            out.push(b'-');
        }
        let mut count = self.digits.len().saturating_sub(self.frac);
        if count == 0 {
            out.push(b'0');
        }
        let mut at = 0;
        while count > 1 && self.digits.get(at) == Some(&0) {
            at = at.saturating_add(1);
            count = count.saturating_sub(1);
        }
        while count > 0 {
            out.push(digit_of(&self.digits, at));
            at = at.saturating_add(1);
            count = count.saturating_sub(1);
        }
        if self.frac > 0 {
            out.push(b'.');
            loop {
                out.push(digit_of(&self.digits, at));
                at = at.saturating_add(1);
                if at >= self.digits.len() {
                    break;
                }
            }
        }
        out
    }

    /// The text of the value in the notation `%+#e` writes, which is
    /// `decimal_result_sci` of `research/sqlite/ext/misc/decimal.c:345`:
    /// one digit, a point, the digits after it with the trailing zeros
    /// past the `count`th dropped, and the exponent.
    ///
    /// The sign shows for every value the sign is on, because the count
    /// of digits the C library reads it against is one at the least.
    ///
    /// Writing the text costs O(n) in the digits.
    #[must_use]
    pub fn scientific(&self, count: usize) -> Vec<u8> {
        let mut held = self.digits.len();
        while held > count && self.digits.get(held.saturating_sub(1)) == Some(&0) {
            held = held.saturating_sub(1);
        }
        let mut zeros = 0;
        while zeros < held && self.digits.get(zeros) == Some(&0) {
            zeros = zeros.saturating_add(1);
        }
        let mut frac = whole(self.frac)
            .saturating_add(whole(held))
            .saturating_sub(whole(self.digits.len()));
        let mut width = held.saturating_sub(zeros);
        if width == 0 {
            width = 1;
            frac = 0;
        }
        let mut out = Vec::with_capacity(width.saturating_add(8));
        out.push(if self.sign { b'-' } else { b'+' });
        out.push(digit_of(&self.digits, zeros));
        out.push(b'.');
        if width == 1 {
            out.push(b'0');
        } else {
            for at in 1..width {
                out.push(digit_of(&self.digits, zeros.saturating_add(at)));
            }
        }
        out.push(b'e');
        let exp = whole(width).saturating_sub(frac).saturating_sub(1);
        out.extend_from_slice(&exponent_text(exp));
        out
    }

    /// The value rounded to `count` significant digits, which is
    /// `decimal_round` of `research/sqlite/ext/misc/decimal.c:300`: the
    /// digits past the `count`th are read back to nought and the digit
    /// before them counts up where the first of them is five or more.
    ///
    /// Rounding costs O(n) in the digits.
    pub fn round(&mut self, count: usize) {
        if count < 1 || self.digits.len() <= count {
            return;
        }
        let mut zeros = 0;
        while zeros < self.digits.len() && self.digits.get(zeros) == Some(&0) {
            zeros = zeros.saturating_add(1);
        }
        let count = count.saturating_add(zeros);
        if self.digits.len() <= count {
            return;
        }
        if self.digits.get(count).is_some_and(|digit| *digit >= 5) {
            // A value whose leading digits are all nine carries one digit
            // more once it is rounded up, so a nought goes in front of
            // them and the digits move up with it.
            if self.digits.iter().take(count).all(|digit| *digit == 9) {
                self.expand(self.digits.len().saturating_add(1), self.frac);
            }
            for slot in self.digits.iter_mut().skip(count.saturating_sub(1)).take(1) {
                *slot = slot.saturating_add(1);
            }
            for at in (1..count).rev() {
                if digit_at(&self.digits, at) <= 9 {
                    break;
                }
                for slot in self.digits.iter_mut().skip(at).take(1) {
                    *slot = 0;
                }
                for slot in self.digits.iter_mut().skip(at.saturating_sub(1)).take(1) {
                    *slot = slot.saturating_add(1);
                }
            }
        }
        for slot in self.digits.iter_mut().skip(count) {
            *slot = 0;
        }
    }

    /// The value held to `digits` digits of which `frac` stand right of
    /// the point, which is `decimal_expand` of
    /// `research/sqlite/ext/misc/decimal.c:470`: zeros go in front of the
    /// digits and after them.
    ///
    /// Expanding costs O(n) in the digits it answers.
    fn expand(&mut self, digits: usize, frac: usize) {
        let more = frac.saturating_sub(self.frac);
        let front = digits
            .saturating_sub(self.digits.len())
            .saturating_sub(more);
        if front > 0 {
            let mut held = alloc::vec![0_u8; front];
            held.append(&mut self.digits);
            self.digits = held;
        }
        if more > 0 {
            self.digits
                .resize(self.digits.len().saturating_add(more), 0);
            self.frac = self.frac.saturating_add(more);
        }
    }

    /// The value with its sign turned over, which is what
    /// `decimalSubFunc` writes before it adds.
    #[must_use]
    pub const fn negated(mut self) -> Self {
        self.sign = !self.sign;
        self
    }

    /// Nought, which is what `decimalSumStep` begins a group with.
    #[must_use]
    pub fn zero() -> Self {
        Decimal {
            sign: false,
            digits: alloc::vec![0],
            frac: 0,
        }
    }
}

/// The digits of a value with the trailing zeros of its fraction
/// dropped, which is what `decimal_cmp` of
/// `research/sqlite/ext/misc/decimal.c:411` writes before it compares.
fn trimmed(held: &Decimal) -> (bool, Vec<u8>, usize) {
    let mut digits = held.digits.clone();
    let mut frac = held.frac;
    while frac > 0 && digits.last() == Some(&0) {
        digits.pop();
        frac = frac.saturating_sub(1);
    }
    (held.sign, digits, frac)
}

/// How two values compare, which is `decimal_cmp`: the sign first, then
/// how many digits stand left of the point, then the digits.
///
/// Comparing costs O(n) in the digits of the shorter value.
#[must_use]
pub fn order(one: &Decimal, other: &Decimal) -> Ordering {
    let (sign, mut mine, mut frac) = trimmed(one);
    let (theirs_sign, mut theirs, mut theirs_frac) = trimmed(other);
    if sign != theirs_sign {
        return if sign {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    // Two values below nought compare the other way round, which the C
    // library writes by swapping them.
    if sign {
        core::mem::swap(&mut mine, &mut theirs);
        core::mem::swap(&mut frac, &mut theirs_frac);
    }
    let held = mine.len().saturating_sub(frac);
    let beside = theirs.len().saturating_sub(theirs_frac);
    if held != beside {
        return held.cmp(&beside);
    }
    let width = mine.len().min(theirs.len());
    let order = mine.iter().take(width).cmp(theirs.iter().take(width));
    if order != Ordering::Equal {
        return order;
    }
    mine.len().cmp(&theirs.len())
}

/// The sum of two values, which is `decimal_add` of
/// `research/sqlite/ext/misc/decimal.c:499`: both are held to one width
/// and the digits are added where the signs are the same and taken apart
/// where they differ.
///
/// Adding costs O(n) in the digits of the wider value.
#[must_use]
pub fn added(one: Decimal, other: Decimal) -> Decimal {
    let mut mine = one;
    let mut theirs = other;
    let mut sig = mine.digits.len().saturating_sub(mine.frac);
    if sig != 0 && mine.digits.first() == Some(&0) {
        sig = sig.saturating_sub(1);
    }
    sig = sig.max(theirs.digits.len().saturating_sub(theirs.frac));
    let frac = mine.frac.max(theirs.frac);
    let width = sig.saturating_add(frac).saturating_add(1);
    mine.expand(width, frac);
    theirs.expand(width, frac);
    if mine.sign == theirs.sign {
        let mut carry = 0_u8;
        for at in (0..width).rev() {
            let held = digit_at(&mine.digits, at)
                .saturating_add(digit_at(&theirs.digits, at))
                .saturating_add(carry);
            carry = u8::from(held >= 10);
            let held = held % 10;
            for slot in mine.digits.iter_mut().skip(at).take(1) {
                *slot = held;
            }
        }
        return mine;
    }
    // The smaller value is taken from the larger one, and the sign of
    // the answer is the sign of the larger.
    let swapped = mine.digits < theirs.digits;
    let (front, behind) = if swapped {
        (theirs.digits.clone(), mine.digits.clone())
    } else {
        (mine.digits.clone(), theirs.digits.clone())
    };
    if swapped {
        mine.sign = !mine.sign;
    }
    let mut borrow = 0_i16;
    for at in (0..width).rev() {
        let held = i16::from(digit_at(&front, at))
            .saturating_sub(i16::from(digit_at(&behind, at)))
            .saturating_sub(borrow);
        borrow = i16::from(held < 0);
        let digit = u8::try_from(held.rem_euclid(10)).unwrap_or(0);
        for slot in mine.digits.iter_mut().skip(at).take(1) {
            *slot = digit;
        }
    }
    mine
}

/// The product of two values, which is `decimalMul` of
/// `research/sqlite/ext/misc/decimal.c:583`: every digit of one is
/// multiplied by every digit of the other, and the trailing zeros of the
/// fraction are dropped down to the narrower fraction of the two.
///
/// One product costs O(n m) in the digits of the two values.
#[must_use]
pub fn multiplied(one: Decimal, other: &Decimal) -> Decimal {
    let mut mine = one;
    let width = mine
        .digits
        .len()
        .saturating_add(other.digits.len())
        .saturating_add(2);
    let mut held = alloc::vec![0_u8; width];
    let least = mine.frac.min(other.frac);
    for at in (0..mine.digits.len()).rev() {
        let by = digit_at(&mine.digits, at);
        let mut carry = 0_u32;
        let mut into = at.saturating_add(other.digits.len()).saturating_add(2);
        for beside in (0..other.digits.len()).rev() {
            let sum = u32::from(digit_at(&held, into))
                .saturating_add(
                    u32::from(by).saturating_mul(u32::from(digit_at(&other.digits, beside))),
                )
                .saturating_add(carry);
            for slot in held.iter_mut().skip(into).take(1) {
                *slot = u8::try_from(sum % 10).unwrap_or(0);
            }
            carry = sum / 10;
            into = into.saturating_sub(1);
        }
        let sum = u32::from(digit_at(&held, into)).saturating_add(carry);
        for slot in held.iter_mut().skip(into).take(1) {
            *slot = u8::try_from(sum % 10).unwrap_or(0);
        }
        let over = u8::try_from(sum / 10).unwrap_or(0);
        for slot in held.iter_mut().skip(into.saturating_sub(1)).take(1) {
            *slot = slot.saturating_add(over);
        }
    }
    mine.digits = held;
    mine.frac = mine.frac.saturating_add(other.frac);
    mine.sign ^= other.sign;
    while mine.frac > least && mine.digits.last() == Some(&0) {
        mine.digits.pop();
        mine.frac = mine.frac.saturating_sub(1);
    }
    mine
}

/// Two to the power `power`, which is `decimalPow2` of
/// `research/sqlite/ext/misc/decimal.c:632`, and nothing for a power
/// past twenty thousand either way.
///
/// One power costs O(log n) products of O(n) digits each.
#[must_use]
pub fn power_of_two(power: i32) -> Option<Decimal> {
    if !(-POWERS..=POWERS).contains(&power) {
        return None;
    }
    let mut held = Decimal::of_text(b"1.0");
    if power == 0 {
        return Some(held);
    }
    let mut by = if power > 0 {
        Decimal::of_text(b"2.0")
    } else {
        Decimal::of_text(b"0.5")
    };
    let mut left = power.unsigned_abs();
    loop {
        if left & 1 != 0 {
            held = multiplied(held, &by);
        }
        left >>= 1_u32;
        if left == 0 {
            return Some(held);
        }
        by = multiplied(by.clone(), &by);
    }
}

/// The value a binary64 number spells exactly, which is
/// `decimalFromDouble` of `research/sqlite/ext/misc/decimal.c:668`: the
/// significand as an integer multiplied by two to the power of the
/// exponent, and nothing for a number that is not one.
///
/// One value costs what the power of two costs.
#[must_use]
pub fn of_double(number: f64) -> Option<Decimal> {
    let negative = number < 0.0;
    let held = if negative { -number } else { number };
    let bits = held.to_bits().cast_signed();
    let (mut significand, exponent) = if bits == 0 || bits == i64::MIN {
        (0_i64, 0_i64)
    } else {
        let mut exponent = bits >> 52_i32;
        let mut significand = bits & ((1_i64 << 52_i32) - 1);
        if exponent == 0 {
            significand <<= 1_i32;
        } else {
            significand |= 1_i64 << 52_i32;
        }
        // The significand stands above nought wherever this runs, because
        // the bits of the number are neither nought nor the sign alone.
        while exponent < 1075 && significand & 1 == 0 {
            significand >>= 1_i32;
            exponent = exponent.saturating_add(1);
        }
        (significand, exponent.saturating_sub(1075))
    };
    if exponent > 971 {
        return None;
    }
    if negative {
        significand = significand.saturating_neg();
    }
    let text = crate::number::integer_text(significand);
    let power = power_of_two(i32::try_from(exponent).unwrap_or(0))?;
    Some(multiplied(Decimal::of_text(&text), &power))
}

/// How two texts compare under the collation `decimal`, which is
/// `decimalCollFunc` of `research/sqlite/ext/misc/decimal.c:748`.
///
/// Comparing costs what reading the two texts costs.
#[must_use]
pub fn collate(left: &[u8], right: &[u8]) -> Ordering {
    order(&Decimal::of_text(left), &Decimal::of_text(right))
}

/// The exponent a text carries after `e`, read as
/// `decimalNewFromText` reads it: one sign and the digits after it,
/// with every other byte passed over and the value held under a million.
fn exponent_of(text: &[u8], from: usize) -> i64 {
    let mut at = from;
    let mut negative = false;
    match text.get(at) {
        Some(&b'-') => {
            negative = true;
            at = at.saturating_add(1);
        }
        Some(&b'+') => at = at.saturating_add(1),
        _ => {}
    }
    let mut held = 0_i64;
    while held < EXPONENT {
        let Some(byte) = text.get(at).copied() else {
            break;
        };
        if byte.is_ascii_digit() {
            held = held
                .saturating_mul(10)
                .saturating_add(i64::from(byte.wrapping_sub(b'0')));
        }
        at = at.saturating_add(1);
    }
    if negative {
        held.saturating_neg()
    } else {
        held
    }
}

/// The exponent as `e%+03d` writes it: the sign and at least two digits.
fn exponent_text(exp: i64) -> Vec<u8> {
    let digits = crate::number::integer_text(exp.abs());
    let mut out = alloc::vec![if exp < 0 { b'-' } else { b'+' }];
    if digits.len() < 2 {
        out.push(b'0');
    }
    out.extend_from_slice(&digits);
    out
}

/// One digit as the byte of its numeral, and `0` past the digits.
fn digit_of(digits: &[u8], at: usize) -> u8 {
    b'0'.saturating_add(digit_at(digits, at))
}

/// One digit, and nought past the digits.
fn digit_at(digits: &[u8], at: usize) -> u8 {
    digits.get(at).copied().unwrap_or(0)
}

/// A count of digits as a length, held to what a length holds.
fn widened(count: i64) -> usize {
    usize::try_from(count).unwrap_or(0)
}

/// A length as a whole number, held to what one holds.
fn whole(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}
