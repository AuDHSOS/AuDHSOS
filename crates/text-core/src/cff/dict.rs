// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{CFF_STACK, CFF2_STACK, blend::Blend};
use crate::{
    Fixed, FontError,
    read::{self, add},
};

pub(super) struct Cursor<'a> {
    pub data: &'a [u8],
    pub at: usize,
}
impl Cursor<'_> {
    pub(super) fn byte(&mut self) -> Result<u8, FontError> {
        let b = read::u8(self.data, self.at)?;
        self.at = add(self.at, 1)?;
        Ok(b)
    }
    pub(super) fn skip(&mut self, n: usize) -> Result<(), FontError> {
        read::bytes(self.data, self.at, n)?;
        self.at = add(self.at, n)?;
        Ok(())
    }
    pub(super) fn number(&mut self, b: u8, dict: bool) -> Result<Option<Fixed>, FontError> {
        let value = match b {
            28 => {
                let v = read::i16(self.data, self.at)?;
                self.skip(2)?;
                Fixed::from_i32(i32::from(v))
            }
            29 if dict => {
                let v = read::i32(self.data, self.at)?;
                self.skip(4)?;
                Fixed::from_i32(v)
            }
            30 if dict => self.real()?,
            32..=246 => Fixed::from_i32(i32::from(b).checked_sub(139).ok_or(FontError::Overflow)?),
            247..=254 => {
                let next = i32::from(self.byte()?);
                let first = i32::from(b)
                    .checked_sub(if b < 251 { 247 } else { 251 })
                    .ok_or(FontError::Overflow)?;
                let value = first
                    .checked_mul(256)
                    .and_then(|n| n.checked_add(next))
                    .and_then(|n| n.checked_add(108))
                    .ok_or(FontError::Overflow)?;
                Fixed::from_i32(if b < 251 {
                    value
                } else {
                    value.checked_neg().ok_or(FontError::Overflow)?
                })
            }
            255 if !dict => {
                let v = read::i32(self.data, self.at)?;
                self.skip(4)?;
                Fixed::from_16_16(v)
            }
            _ => return Ok(None),
        };
        Ok(Some(value))
    }
    fn real(&mut self) -> Result<Fixed, FontError> {
        let mut digits = 0_i64;
        let mut fractional = 0_u32;
        let mut exponent = 0_u32;
        let mut negative = false;
        let mut decimal = false;
        let mut exp = false;
        let mut exp_digits = false;
        let mut exp_negative = false;
        let mut any = false;
        for _ in 0..32 {
            let b = self.byte()?;
            for nibble in [b >> 4, b & 15] {
                match nibble {
                    0..=9 => {
                        any = true;
                        if exp {
                            exp_digits = true;
                            exponent = exponent
                                .checked_mul(10)
                                .and_then(|n| n.checked_add(u32::from(nibble)))
                                .ok_or(FontError::Overflow)?;
                        } else {
                            digits = digits
                                .checked_mul(10)
                                .and_then(|n| n.checked_add(i64::from(nibble)))
                                .ok_or(FontError::Overflow)?;
                            if decimal {
                                fractional =
                                    fractional.checked_add(1).ok_or(FontError::Overflow)?;
                            }
                        }
                    }
                    10 if !decimal && !exp => decimal = true,
                    11 | 12 if !exp && any => {
                        exp = true;
                        exp_negative = nibble == 12;
                    }
                    14 if !any && !negative && !decimal => negative = true,
                    15 if any && (!exp || exp_digits) => {
                        let power = if exp_negative {
                            i64::from(fractional).checked_add(i64::from(exponent))
                        } else {
                            i64::from(fractional).checked_sub(i64::from(exponent))
                        }
                        .ok_or(FontError::Overflow)?;
                        if negative {
                            digits = digits.checked_neg().ok_or(FontError::Overflow)?;
                        }
                        if power >= 0 {
                            let denominator = 10_i64
                                .checked_pow(u32::try_from(power).map_err(|_| FontError::Overflow)?)
                                .ok_or(FontError::Overflow)?;
                            return Fixed::ONE.mul_ratio(digits, denominator);
                        }
                        let factor = 10_i64
                            .checked_pow(
                                u32::try_from(power.unsigned_abs())
                                    .map_err(|_| FontError::Overflow)?,
                            )
                            .ok_or(FontError::Overflow)?;
                        return Fixed::ONE
                            .mul_ratio(digits.checked_mul(factor).ok_or(FontError::Overflow)?, 1);
                    }
                    _ => return Err(FontError::InvalidTable),
                }
            }
        }
        Err(FontError::LimitExceeded)
    }
}

pub(super) const fn integer(value: Fixed) -> Result<i64, FontError> {
    if value.bits() & 0xffff_ffff != 0 {
        return Err(FontError::InvalidTable);
    }
    Ok(value.bits() >> 32)
}
pub(super) fn offset(value: Fixed) -> Result<usize, FontError> {
    usize::try_from(integer(value)?).map_err(|_| FontError::InvalidTable)
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Dict {
    pub charstrings: usize,
    pub private: (usize, usize),
    pub subrs: usize,
    pub fdarray: usize,
    pub fdselect: usize,
    pub charset: usize,
    pub cid: bool,
    pub matrix: Option<[Fixed; 6]>,
    pub vstore: usize,
    pub vsindex: usize,
}

impl Dict {
    pub(super) fn parse(data: &[u8], cff2: bool) -> Result<Self, FontError> {
        Self::parse_with(data, cff2, Blend::default())
    }
    pub(super) fn parse_with(data: &[u8], cff2: bool, blend: Blend<'_>) -> Result<Self, FontError> {
        if cff2 {
            Self::parse_in::<CFF2_STACK>(data, cff2, blend)
        } else {
            Self::parse_in::<CFF_STACK>(data, cff2, blend)
        }
    }
    /// Holds an `N`-operand stack; `inline(never)` gives each `N` its own
    /// frame, so the CFF caller's frame omits the 513-entry array.
    #[inline(never)]
    #[expect(
        clippy::many_single_char_names,
        reason = "six affine matrix entries follow the CFF notation"
    )]
    fn parse_in<const N: usize>(
        data: &[u8],
        cff2: bool,
        mut blend: Blend<'_>,
    ) -> Result<Self, FontError> {
        let mut c = Cursor { data, at: 0 };
        let mut stack = [Fixed::ZERO; N];
        let mut len = 0;
        let mut d = Self::default();
        while c.at < data.len() {
            let b = c.byte()?;
            if let Some(n) = c.number(b, true)? {
                if len >= N {
                    return Err(FontError::LimitExceeded);
                }
                *stack.get_mut(len).ok_or(FontError::LimitExceeded)? = n;
                len = add(len, 1)?;
                continue;
            }
            let op = if b == 12 {
                0x0c00 | u16::from(c.byte()?)
            } else {
                u16::from(b)
            };
            if cff2 && op == 23 {
                blend.apply(&mut stack, &mut len)?;
                continue;
            }
            let args = stack.get(..len).ok_or(FontError::InvalidTable)?;
            match (op, args) {
                (17, [v]) => d.charstrings = offset(*v)?,
                (18, [n, p]) => d.private = (offset(*n)?, offset(*p)?),
                (19, [v]) => d.subrs = offset(*v)?,
                (15, [v]) => d.charset = offset(*v)?,
                (24, [v]) if cff2 => d.vstore = offset(*v)?,
                (22, [v]) if cff2 => {
                    d.vsindex = offset(*v)?;
                    blend.select(d.vsindex)?;
                }
                (0x0c24, [v]) => d.fdarray = offset(*v)?,
                (0x0c25, [v]) => d.fdselect = offset(*v)?,
                (0x0c1e, [_, _, _]) => d.cid = true,
                (0x0c07, [a, b, c, d0, e, f]) => d.matrix = Some([*a, *b, *c, *d0, *e, *f]),
                (0x0c06, [v]) if *v == Fixed::from_i32(2) => (),
                (17..=19 | 0x0c24 | 0x0c25 | 0x0c1e | 0x0c07 | 0x0c06, _) => {
                    return Err(FontError::InvalidTable);
                }
                (22 | 23, _) if cff2 => return Err(FontError::UnsupportedFormat),
                (0..=21 | 0x0c00..=0x0c26, _) => (),
                _ if cff2 => (),
                _ => return Err(FontError::InvalidTable),
            }
            len = 0;
        }
        if len != 0 {
            return Err(FontError::InvalidTable);
        }
        Ok(d)
    }
}
