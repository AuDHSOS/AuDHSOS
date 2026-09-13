// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Numbers out of text, and integers back into it.
//!
//! Which text is a number decides what `'1x' + 1` answers and whether
//! `'10' < '9'` compares as text or as numbers, so it is read the way
//! `sqlite3AtoF` and `sqlite3Atoi64` in `src/util.c` read it, down to
//! which prefixes count and which digits are dropped. The double itself
//! comes from [`crate::fp::from_digits`], which is the same routine the C
//! library calls.
//!
//! Reading is one pass over the text: O(n) in its length.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "a port of fixed-width C arithmetic; the mantissa is guarded by its own limit, the cursor by the text, and the exponent saturates"
)]

use alloc::vec::Vec;

use crate::fp::from_digits;

/// The most a mantissa may hold before one more digit would not fit.
const MANTISSA_LIMIT: u64 = (u64::MAX - 9) / 10;

/// What SQLite calls whitespace, which is not what Unicode calls it.
const fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// A digit's value, or nothing.
fn digit(byte: u8) -> Option<u64> {
    if byte.is_ascii_digit() {
        Some(u64::from(byte - b'0'))
    } else {
        None
    }
}

/// What reading a real out of text answered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Real {
    /// The number, or zero where there was none.
    pub value: f64,
    /// The bits `sqlite3AtoF` answers with, which the methods below name.
    read: u8,
    /// Whether the whole of the text was the number, trailing spaces
    /// aside.
    whole: bool,
}

impl Real {
    /// Whether any prefix of the text was a number at all.
    #[must_use]
    pub const fn number(self) -> bool {
        self.read & 1 != 0
    }

    /// Whether a point or an exponent was written, which is what makes a
    /// number a real rather than an integer.
    #[must_use]
    pub const fn fractional(self) -> bool {
        self.read & 2 != 0
    }

    /// Whether the value is a zero as written, rather than one that
    /// underflowed to zero.
    #[must_use]
    pub const fn zero(self) -> bool {
        self.read & 4 != 0
    }

    /// Whether more digits were written than a mantissa holds.
    #[must_use]
    pub const fn truncated(self) -> bool {
        self.read & 8 != 0
    }

    /// Whether the whole of the text was the number, trailing spaces
    /// aside.
    #[must_use]
    pub const fn complete(self) -> bool {
        self.whole
    }
}

/// Nothing that is a number.
const NO_REAL: Real = Real {
    value: 0.0,
    read: 0,
    whole: false,
};

/// The real `text` begins with.
///
/// This is `sqlite3AtoF`. A NUL ends the text, as it does in C.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one routine of the C library, kept in one piece so that it can be read against it"
)]
pub fn real(text: &[u8]) -> Real {
    let byte = |at: usize| text.get(at).copied().unwrap_or(0);
    let mut at = 0;
    let mut negative = false;
    let mut mantissa: u64 = 0;
    let mut power: i32 = 0;
    // The bits `sqlite3AtoF` answers with: 1 a digit, 2 a point or an
    // exponent, 4 a hard zero, 8 more digits than are kept.
    let mut state = 0u8;

    // Whitespace, then at most one sign. A sign is not followed by more
    // whitespace: `- 1` is not a number.
    loop {
        if is_space(byte(at)) {
            at += 1;
            continue;
        }
        if byte(at) == b'-' {
            negative = true;
            at += 1;
        } else if byte(at) == b'+' {
            at += 1;
        }
        break;
    }

    if let Some(first) = digit(byte(at)) {
        state = 1;
        mantissa = first;
        at += 1;
        while let Some(value) = digit(byte(at)) {
            mantissa = mantissa * 10 + value;
            at += 1;
            if mantissa >= MANTISSA_LIMIT {
                // Past what the mantissa holds the digits only move the
                // point: the value is rounded, not read further.
                state = 9;
                while digit(byte(at)).is_some() {
                    at += 1;
                    power = power.saturating_add(1);
                }
                break;
            }
        }
    }

    if byte(at) == b'.' {
        at += 1;
        if digit(byte(at)).is_some() {
            state |= 1;
            loop {
                if mantissa < MANTISSA_LIMIT {
                    mantissa = mantissa * 10 + digit(byte(at)).unwrap_or(0);
                    power = power.saturating_sub(1);
                } else {
                    state = 11;
                }
                at += 1;
                if digit(byte(at)).is_none() {
                    break;
                }
            }
        } else if state == 0 {
            return NO_REAL;
        }
        state |= 2;
    } else if state == 0 {
        return NO_REAL;
    }

    if byte(at) == b'e' || byte(at) == b'E' {
        at += 1;
        let sign = if byte(at) == b'-' {
            at += 1;
            -1
        } else {
            if byte(at) == b'+' {
                at += 1;
            }
            1
        };
        if let Some(first) = digit(byte(at)) {
            let mut exponent = i32::try_from(first).unwrap_or(0);
            at += 1;
            state |= 2;
            while let Some(value) = digit(byte(at)) {
                exponent = if exponent < 10_000 {
                    exponent * 10 + i32::try_from(value).unwrap_or(0)
                } else {
                    10_000
                };
                at += 1;
            }
            power = power.saturating_add(sign * exponent);
        } else {
            // An `e` with no digits after it is not part of the number,
            // which leaves text the number did not cover.
            at -= 1;
        }
    }

    let mut value = if mantissa == 0 {
        state |= 4;
        0.0
    } else {
        from_digits(mantissa, power)
    };
    if negative {
        value = -value;
    }
    while is_space(byte(at)) {
        at += 1;
    }
    Real {
        value,
        read: state,
        whole: byte(at) == 0,
    }
}

/// How reading an integer out of text came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The whole text was the integer.
    Exact,
    /// There were no digits.
    Empty,
    /// The integer was followed by something that is not whitespace.
    Trailing,
    /// The digits name a number above what an integer holds.
    Overflow,
    /// The digits are exactly `9223372036854775808`, written without a
    /// sign, which is one past the largest integer and the magnitude of
    /// the smallest.
    Limit,
}

/// What reading an integer out of text answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Integer {
    /// The number, clamped to what an integer holds.
    pub value: i64,
    /// How it came out.
    pub outcome: Outcome,
}

/// The integer `text` is.
///
/// This is `sqlite3Atoi64` over UTF-8. Unlike [`real`] it reads the whole
/// slice, a NUL included, because the C routine is given a length.
#[must_use]
pub fn integer(text: &[u8]) -> Integer {
    let byte = |at: usize| text.get(at).copied().unwrap_or(0);
    let len = text.len();
    let mut at = 0;
    while at < len && is_space(byte(at)) {
        at += 1;
    }
    let mut negative = false;
    if at < len {
        if byte(at) == b'-' {
            negative = true;
            at += 1;
        } else if byte(at) == b'+' {
            at += 1;
        }
    }
    let start = at;
    while at < len && byte(at) == b'0' {
        at += 1;
    }
    let mut unsigned: u64 = 0;
    let mut count = 0;
    while let Some(value) = digit(byte(at + count)) {
        unsigned = unsigned.wrapping_mul(10).wrapping_add(value);
        count += 1;
    }
    let clamp = if negative { i64::MIN } else { i64::MAX };
    let value =
        i64::try_from(unsigned).map_or(clamp, |value| if negative { -value } else { value });

    let mut outcome = Outcome::Exact;
    if count == 0 && start == at {
        outcome = Outcome::Empty;
    } else if at + count < len {
        let mut after = at + count;
        while after < len {
            if !is_space(byte(after)) {
                outcome = Outcome::Trailing;
                break;
            }
            after += 1;
        }
    }
    if count < 19 {
        return Integer { value, outcome };
    }
    // Nineteen digits is where an integer may or may not hold the number,
    // so the digits are compared against 2^63 rather than the value.
    let order = if count > 19 {
        1
    } else {
        compare_to_2pow63(text.get(at..).unwrap_or_default())
    };
    if order < 0 {
        return Integer { value, outcome };
    }
    Integer {
        value: clamp,
        // Exactly 2^63 is the smallest integer where a sign precedes it
        // and one past the largest where none does.
        outcome: if order > 0 {
            Outcome::Overflow
        } else if negative {
            outcome
        } else {
            Outcome::Limit
        },
    }
}

/// Where nineteen digits sit against `9223372036854775808`.
fn compare_to_2pow63(digits: &[u8]) -> i32 {
    let limit = b"922337203685477580";
    let byte = |at: usize| digits.get(at).copied().unwrap_or(0);
    for (at, bound) in limit.iter().enumerate() {
        let order = (i32::from(byte(at)) - i32::from(*bound)) * 10;
        if order != 0 {
            return order;
        }
    }
    i32::from(byte(18)) - i32::from(b'8')
}

/// An integer as the text SQLite writes it as, which is decimal with a
/// sign and no separators.
///
/// This is `sqlite3Int64ToText`.
#[must_use]
pub fn integer_text(value: i64) -> Vec<u8> {
    let mut out = Vec::new();
    if value == 0 {
        out.push(b'0');
        return out;
    }
    let mut rest = value.unsigned_abs();
    let mut digits = Vec::new();
    while rest > 0 {
        digits.push(b'0' + u8::try_from(rest % 10).unwrap_or(0));
        rest /= 10;
    }
    if value < 0 {
        out.push(b'-');
    }
    out.extend(digits.iter().rev());
    out
}
