// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Doubles as decimal text.
//!
//! A double has no decimal spelling of its own, so every engine picks
//! one, and two engines that pick differently disagree about what a query
//! answers. SQLite picks its own: `sqlite3FpDecode` in `src/util.c`
//! extracts eighteen significant digits with a 128-bit multiply against a
//! table of powers of ten, rounds them to seventeen, and tries a shorter
//! rendering where the shorter one reads back as the same double.
//! `sqlite3VdbeMemStringify` then prints those digits as `%!.17g`.
//!
//! This is that, and the tables are the ones `tool/mkfptab.c --round`
//! generated. Every conversion is a fixed number of steps: O(1).

#![expect(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a port of fixed-width C arithmetic, where every index and shift is bounded by the algorithm and the tests cover the bounds"
)]

use alloc::vec::Vec;

/// How many significant digits a double is printed with, which is what
/// `PRAGMA fp_digits` starts at.
pub const DIGITS: i32 = 17;

/// The most digits `decode` will answer with, which is `mxRound` for the
/// `!` flag.
pub const MAX_DIGITS: i32 = 20;

/// The smallest power of ten the table holds.
const FIRST_POWER: i32 = -348;

/// The largest power of ten the table holds.
const LAST_POWER: i32 = 347;

/// The most significant 64 bits of `1.0e+p` for p in 0..=26.
const BASE: [u64; 27] = [
    0x8000_0000_0000_0000,
    0xa000_0000_0000_0000,
    0xc800_0000_0000_0000,
    0xfa00_0000_0000_0000,
    0x9c40_0000_0000_0000,
    0xc350_0000_0000_0000,
    0xf424_0000_0000_0000,
    0x9896_8000_0000_0000,
    0xbebc_2000_0000_0000,
    0xee6b_2800_0000_0000,
    0x9502_f900_0000_0000,
    0xba43_b740_0000_0000,
    0xe8d4_a510_0000_0000,
    0x9184_e72a_0000_0000,
    0xb5e6_20f4_8000_0000,
    0xe35f_a931_a000_0000,
    0x8e1b_c9bf_0400_0000,
    0xb1a2_bc2e_c500_0000,
    0xde0b_6b3a_7640_0000,
    0x8ac7_2304_89e8_0000,
    0xad78_ebc5_ac62_0000,
    0xd8d7_26b7_177a_8000,
    0x8786_7832_6eac_9000,
    0xa968_163f_0a57_b400,
    0xd3c2_1bce_cced_a100,
    0x8459_5161_4014_84a0,
    0xa56f_a5b9_9019_a5c8,
];

/// The most significant 64 bits of `1.0e+(27*g-351)` for g in 0..26.
const SCALE: [u64; 26] = [
    0x8049_a4ac_0c58_11ae,
    0xcf42_894a_5dce_35ea,
    0xa76c_5823_38ed_2621,
    0x873e_4f75_e222_4e68,
    0xda7f_5bf5_9096_6848,
    0xb080_392c_c434_9dec,
    0x8e93_8662_882a_f53e,
    0xe658_29b3_046b_0afa,
    0xba12_1a46_50e4_ddeb,
    0x964e_858c_91ba_2655,
    0xf2d5_6790_ab41_c2a2,
    0xc428_d05a_a475_1e4c,
    0x9e74_d1b7_91e0_7e48,
    0xcccc_cccc_cccc_cccc,
    0xcecb_8f27_f420_0f3a,
    0xa70c_3c40_a64e_6c51,
    0x86f0_ac99_b4e8_dafd,
    0xda01_ee64_1a70_8de9,
    0xb01a_e745_b101_e9e4,
    0x8e41_ade9_fbeb_c27d,
    0xe5d3_ef28_2a24_2e81,
    0xb9a7_4a06_37ce_2ee1,
    0x95f8_3d0a_1fb6_9cd9,
    0xf24a_01a7_3cf2_dccf,
    0xc3b8_3581_09e8_4f07,
    0x9e19_db92_b4e3_1ba9,
];

/// The next 32 bits of each entry of [`SCALE`].
const SCALE_LO: [u32; 26] = [
    0x205b_896d,
    0x5206_4cad,
    0xaf2a_f2b8,
    0x5a77_44a7,
    0xaf39_a475,
    0xbd8d_794e,
    0x547e_b47b,
    0x0cb4_a5a3,
    0x92f3_4d62,
    0x3a6a_07f9,
    0xfae2_7299,
    0xaa97_e14c,
    0x775e_a265,
    0xcccc_cccc,
    0x0000_0000,
    0x9990_90b6,
    0x69a0_28bb,
    0xe80e_6f48,
    0x5ec0_5dd0,
    0x1458_8f14,
    0x8f16_68c9,
    0x6d95_3e2c,
    0x4abd_af10,
    0xbc63_3b39,
    0x0a86_2f81,
    0x6c07_a2c2,
];

/// The high 64 bits of `a * b`, and the low 64.
fn multiply128(a: u64, b: u64) -> (u64, u64) {
    let wide = u128::from(a).wrapping_mul(u128::from(b));
    let high = u64::try_from(wide >> 64).unwrap_or(0);
    let low = u64::try_from(wide & u128::from(u64::MAX)).unwrap_or(0);
    (high, low)
}

/// The upper 96 bits of `((a << 32) + a_lo) * b`, as the high 64 and the
/// middle 32.
fn multiply160(a: u64, a_lo: u32, b: u64) -> (u64, u32) {
    let wide = u128::from(a)
        .wrapping_mul(u128::from(b))
        .wrapping_add(u128::from(a_lo).wrapping_mul(u128::from(b)) >> 32);
    let middle = u32::try_from((wide >> 32) & u128::from(u32::MAX)).unwrap_or(0);
    (u64::try_from(wide >> 64).unwrap_or(0), middle)
}

/// `floor(log2(pow(10, p)))`, from `ln(10)/ln(2) ~ 108853/32768`.
const fn power10_to_2(p: i32) -> i32 {
    p.wrapping_mul(108_853) >> 15
}

/// `floor(log10(pow(2, p)))`, from `ln(2)/ln(10) ~ 78913/262144`.
const fn power2_to_10(p: i32) -> i32 {
    p.wrapping_mul(78_913) >> 18
}

/// The most significant 64 bits of `pow(10, p)`, and the 32 bits after
/// them. `p` must be between [`FIRST_POWER`] and [`LAST_POWER`].
fn power_of_ten(p: i32) -> (u64, u32) {
    let at = |index: i32| usize::try_from(index).unwrap_or(0);
    let (group, rest) = if p == -1 {
        // The one power the table cannot reach by group and remainder.
        return (SCALE[13], SCALE_LO[13]);
    } else if p < 0 {
        let (group, rest) = (p / 27, p % 27);
        if rest == 0 {
            (group, rest)
        } else {
            (group - 1, rest + 27)
        }
    } else if p < 27 {
        return (BASE[at(p)], 0);
    } else {
        (p / 27, p % 27)
    };
    let index = at(group + 13);
    if rest == 0 {
        return (SCALE[index], SCALE_LO[index]);
    }
    let (mut high, mut low) = multiply160(SCALE[index], SCALE_LO[index], BASE[at(rest)]);
    if high & (1 << 63) == 0 {
        high = (high << 1) | u64::from((low >> 31) & 1);
        low = (low << 1) | 1;
    }
    (high, low)
}

/// `d` and `p` such that `m * pow(2, e)` is about `d * pow(10, p)`, with
/// `d` holding at least `n` significant digits. `m` must have its highest
/// bit set and `n` must be between 1 and 18.
fn convert2_to_10(m: u64, e: i32, n: i32) -> (u64, i32) {
    let p = n - 1 - power2_to_10(e + 63);
    let (power, _) = power_of_ten(p);
    let (high, _) = multiply128(m, power);
    let digits = if n == 18 {
        let shifted = high.wrapping_shr(u32::try_from(-(e + power10_to_2(p) + 2)).unwrap_or(0));
        // One digit too many: carry its low bit into the rest.
        shifted.wrapping_add((shifted & 1) << 1) >> 1
    } else {
        high.wrapping_shr(u32::try_from(-(e + power10_to_2(p) + 1)).unwrap_or(0))
    };
    (digits, -p)
}

/// The double nearest `d * pow(10, p)`.
///
/// It is the inverse of the decimal extraction above. Rendering uses it for one
/// thing — asking whether a shorter run of digits reads back as the same
/// double — and reading a number out of text will use it for the answer.
#[must_use]
pub fn from_digits(d: u64, p: i32) -> f64 {
    if p < FIRST_POWER {
        return 0.0;
    }
    if p > LAST_POWER {
        return f64::INFINITY;
    }
    let bits = 64 - i32::try_from(d.leading_zeros()).unwrap_or(0);
    let scale = power10_to_2(p);
    let mut e = 53 - bits - scale;
    if e > 1074 {
        if e >= 1130 {
            return 0.0;
        }
        e = 1074;
    }
    let shift = u32::try_from(-(e - (64 - bits) + scale + 3)).unwrap_or(0);
    let (mut power_high, mut power_low) = power_of_ten(p);
    if power_low != 0 {
        power_high += 1;
        power_low = !power_low;
    }
    let x = d.wrapping_shl(u32::try_from(64 - bits).unwrap_or(0));
    let (mut high, low) = multiply128(x, power_high);
    let middle = u32::try_from(low >> 32).unwrap_or(0);
    let mut sticky = 1;
    if high & (1u64.wrapping_shl(shift).wrapping_sub(1)) == 0 {
        let (product, _) = multiply128(x, u64::from(power_low) << 32);
        let other = u32::try_from(product >> 32).unwrap_or(0);
        sticky = u64::from(middle.wrapping_sub(other) > 1);
        high -= u64::from(middle < other);
    }
    let mut unrounded = high.wrapping_shr(shift) | sticky;
    if unrounded >= (1u64 << 55) - 2 {
        unrounded = (unrounded >> 1) | (unrounded & 1);
        e -= 1;
    }
    let mut mantissa = (unrounded + 1 + ((unrounded >> 2) & 1)) >> 2;
    if e <= -972 {
        return f64::INFINITY;
    }
    if mantissa & (1 << 52) != 0 {
        let exponent = u64::try_from(1075 - e).unwrap_or(0);
        mantissa = (mantissa & !(1u64 << 52)) | (exponent << 52);
    }
    f64::from_bits(mantissa)
}

/// A double taken apart into decimal digits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoded {
    /// Whether the double is below zero.
    pub negative: bool,
    /// Nothing, an infinity, or a NaN.
    pub special: Special,
    /// The significant digits, as ASCII, with no trailing zero.
    pub digits: Vec<u8>,
    /// Where the decimal point goes, counted from the first digit.
    pub point: i32,
}

/// What a double is when it is not a number with digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    /// A number.
    None,
    /// An infinity.
    Infinity,
    /// A NaN.
    NotANumber,
}

/// `r` as decimal digits, `round` of them, at most [`MAX_DIGITS`].
///
/// This is `sqlite3FpDecode` for the rounding `%!g` asks for.
#[must_use]
pub fn decode(r: f64, round: i32) -> Decoded {
    decoded(r, round.clamp(1, MAX_DIGITS))
}

/// `r` as decimal digits, `decimals` of them to the right of the point.
///
/// This is `sqlite3FpDecode` for the rounding `%!f` asks for, which is
/// what `round(X,Y)` is written with.
#[must_use]
pub fn decode_decimals(r: f64, decimals: i32) -> Decoded {
    decoded(r, decimals.clamp(0, 30).wrapping_neg())
}

/// Both of the above: above zero `round` counts significant digits, and
/// at or below it counts digits to the right of the point, negated.
fn decoded(r: f64, round: i32) -> Decoded {
    let mut out = Decoded {
        negative: false,
        special: Special::None,
        digits: Vec::new(),
        point: 0,
    };
    let mut value = r;
    if value < 0.0 {
        out.negative = true;
        value = -value;
    } else if value == 0.0 {
        out.digits.push(b'0');
        out.point = 1;
        return out;
    }
    let bits = value.to_bits();
    let mut exponent = i32::try_from((bits >> 52) & 0x7ff).unwrap_or(0);
    let mut mantissa = bits & 0x000f_ffff_ffff_ffff;
    if exponent == 0x7ff {
        out.special = if mantissa == 0 {
            Special::Infinity
        } else {
            Special::NotANumber
        };
        return out;
    }
    if exponent == 0 {
        let zeros = i32::try_from(mantissa.leading_zeros()).unwrap_or(0);
        mantissa = mantissa.wrapping_shl(u32::try_from(zeros).unwrap_or(0));
        exponent = -1074 - zeros;
    } else {
        mantissa = (mantissa << 11) | (1 << 63);
        exponent -= 1086;
    }
    let width = if round <= 0 || round >= 18 {
        18
    } else {
        round + 1
    };
    let (value10, power) = convert2_to_10(mantissa, exponent, width);
    let mut digits = Vec::new();
    let mut rest = value10;
    while rest > 0 {
        digits.push(b'0' + u8::try_from(rest % 10).unwrap_or(0));
        rest /= 10;
    }
    digits.reverse();
    let mut count = i32::try_from(digits.len()).unwrap_or(0);
    out.point = count + power;
    let mut round = round;
    if round <= 0 {
        // Digits to the right of the point become digits in all: where
        // the first of them rounds up, one more is made room for.
        round = out.point.saturating_sub(round);
        if round == 0 && digits.first().is_some_and(|digit| *digit >= b'5') {
            round = 1;
            digits.insert(0, b'0');
            count += 1;
            out.point += 1;
        }
    }
    let round = round.min(MAX_DIGITS);
    if round > 0 && round < count {
        let mut round = round;
        if round == DIGITS {
            round = shorten(value, &digits, power, count);
        }
        count = round;
        let at = usize::try_from(round).unwrap_or(0);
        if digits.get(at).is_some_and(|digit| *digit >= b'5') {
            let mut j = at;
            loop {
                digits[j - 1] += 1;
                if digits[j - 1] <= b'9' {
                    break;
                }
                digits[j - 1] = b'0';
                if j == 1 {
                    digits.insert(0, b'1');
                    count += 1;
                    out.point += 1;
                    break;
                }
                j -= 1;
            }
        }
    }
    digits.truncate(usize::try_from(count).unwrap_or(0));
    while digits.len() > 1 && digits.last() == Some(&b'0') {
        digits.pop();
    }
    out.digits = digits;
    out
}

/// How many of the seventeen digits are enough, where fewer read back as
/// the same double. Answers seventeen where none are.
///
/// This is what keeps `49.47` from printing as `49.469999999999999`.
#[expect(
    clippy::float_cmp,
    reason = "the question is whether the shorter digits are the same double, which is an exact comparison"
)]
fn shorten(value: f64, digits: &[u8], power: i32, count: i32) -> i32 {
    let digit = |at: usize| digits.get(at).copied().unwrap_or(b'0');
    let number = |len: i32| {
        let mut out: u64 = 0;
        for at in 0..usize::try_from(len).unwrap_or(0) {
            out = out * 10 + u64::from(digit(at) - b'0');
        }
        out
    };
    // How many digits are left once a run at the end is dropped: one past
    // the last digit that is not part of the run.
    let run_before = |limit: usize, skipped: u8| {
        i32::try_from(
            digits
                .get(..limit)
                .unwrap_or_default()
                .iter()
                .rposition(|byte| *byte != skipped)
                .map_or(0, |at| at.saturating_add(1)),
        )
        .unwrap_or(0)
    };
    let point = count + power;
    let (kept, wanted) = if digit(15) == b'9' && digit(14) == b'9' {
        // A run of nines: round it away and see whether what is left
        // reads back as the same double.
        let kept = run_before(14, b'9');
        (kept, number(kept) + 1)
    } else if point >= count || (digit(15) == b'0' && digit(14) == b'0' && digit(13) == b'0') {
        // A run of zeros: drop it.
        let kept = run_before(13, b'0');
        (kept, number(kept))
    } else {
        return DIGITS;
    };
    if from_digits(wanted, power + count - kept) == value {
        kept + 1
    } else {
        DIGITS
    }
}

/// The digits with the point in them, which is the same work for `%g`
/// and for `%f` once the precision and the digits before the point are
/// settled.
fn write_digits(out: &mut Vec<u8>, decoded: &Decoded, precision: i32, before: i32) {
    let mut precision = precision;
    let mut before = before;
    let count = i32::try_from(decoded.digits.len()).unwrap_or(0);
    let mut taken = 0;
    if before < 0 {
        out.push(b'0');
    } else {
        taken = (before + 1).min(count);
        out.extend(
            decoded
                .digits
                .iter()
                .take(usize::try_from(taken).unwrap_or(0)),
        );
        before -= taken;
        if before >= 0 {
            out.extend(core::iter::repeat_n(
                b'0',
                usize::try_from(before + 1).unwrap_or(0),
            ));
            before = -1;
        }
    }
    out.push(b'.');
    // Zeros between the point and the first digit. There is always room
    // for them: a point that far left leaves the precision that wide.
    if before < -1 {
        let zeros = (-1 - before).min(precision);
        out.extend(core::iter::repeat_n(
            b'0',
            usize::try_from(zeros).unwrap_or(0),
        ));
        precision -= zeros;
    }
    if precision > 0 && count > taken {
        out.extend(
            decoded
                .digits
                .iter()
                .skip(usize::try_from(taken).unwrap_or(0)),
        );
    }
    // Nothing before the point can be a zero that has to go: `decode`
    // answers no trailing zero, and what is padded in is always to the
    // left of it. What is left is a point with nothing after it.
    if out.last() == Some(&b'.') {
        out.push(b'0');
    }
}

/// `r` as text, the way `%!.<significant>g` prints it, which is the way
/// SQLite converts a real to a string.
#[must_use]
pub fn text(r: f64, significant: i32) -> Vec<u8> {
    let mut precision = significant.clamp(1, MAX_DIGITS);
    let decoded = decode(r, precision);
    match decoded.special {
        Special::NotANumber => return b"NaN".to_vec(),
        Special::Infinity => {
            return if decoded.negative {
                b"-Inf".to_vec()
            } else {
                b"Inf".to_vec()
            };
        }
        Special::None => {}
    }
    let mut out = Vec::new();
    if decoded.negative {
        out.push(b'-');
    }
    let exponent = decoded.point - 1;
    precision -= 1;
    let scientific = exponent < -4 || exponent > precision;
    if !scientific {
        precision -= exponent;
    }
    let before = if scientific { 0 } else { decoded.point - 1 };
    write_digits(&mut out, &decoded, precision, before);
    if scientific {
        let mut rest = decoded.point - 1;
        out.push(b'e');
        if rest < 0 {
            out.push(b'-');
            rest = -rest;
        } else {
            out.push(b'+');
        }
        if rest >= 100 {
            out.push(b'0' + u8::try_from(rest / 100).unwrap_or(0));
            rest %= 100;
        }
        out.push(b'0' + u8::try_from(rest / 10).unwrap_or(0));
        out.push(b'0' + u8::try_from(rest % 10).unwrap_or(0));
    }
    out
}

/// `r` as text with `decimals` digits after the point, the way
/// `%!.<decimals>f` prints it.
///
/// Trailing zeros go, and a point with nothing after it keeps one, so
/// two decimals of `1.5` is `1.5` rather than `1.50`.
#[must_use]
pub fn fixed(r: f64, decimals: i32) -> Vec<u8> {
    let decoded = decode_decimals(r, decimals);
    match decoded.special {
        Special::NotANumber => return b"NaN".to_vec(),
        Special::Infinity => {
            return if decoded.negative {
                b"-Inf".to_vec()
            } else {
                b"Inf".to_vec()
            };
        }
        Special::None => {}
    }
    let mut out = Vec::new();
    if decoded.negative {
        out.push(b'-');
    }
    write_digits(&mut out, &decoded, decimals.clamp(0, 30), decoded.point - 1);
    out
}
