// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{
    FontError,
    read::{self, add, mul},
};

#[derive(Clone, Copy, Debug)]
pub(super) struct Table<'a>(pub &'a [u8]);
impl Table<'_> {
    pub(super) fn u(self, at: usize) -> Result<usize, FontError> {
        Ok(usize::from(read::u16(self.0, at)?))
    }
    pub(super) fn word(self, at: usize) -> Result<u16, FontError> {
        read::u16(self.0, at)
    }
    pub(super) fn signed(self, at: usize) -> Result<i16, FontError> {
        read::i16(self.0, at)
    }
    pub(super) fn long(self, at: usize) -> Result<usize, FontError> {
        read::offset(read::u32(self.0, at)?)
    }
    pub(super) fn tag(self, at: usize) -> Result<[u8; 4], FontError> {
        read::array(self.0, at)
    }
    pub(super) fn at(self, at: usize) -> Result<Self, FontError> {
        Ok(Self(read::tail(self.0, at)?))
    }
    pub(super) fn child(self, at: usize) -> Result<Self, FontError> {
        let n = self.u(at)?;
        if n == 0 {
            return Err(FontError::InvalidTable);
        }
        self.at(n)
    }
    pub(super) fn child32(self, at: usize) -> Result<Self, FontError> {
        let n = self.long(at)?;
        if n == 0 {
            return Err(FontError::InvalidTable);
        }
        self.at(n)
    }
    pub(super) fn array(self, at: usize, n: usize, stride: usize) -> Result<(), FontError> {
        read::bytes(self.0, at, mul(n, stride)?)?;
        Ok(())
    }
    pub(super) fn coverage(self, glyph: u16) -> Result<Option<usize>, FontError> {
        let n = self.u(2)?;
        match self.u(0)? {
            1 => {
                self.array(4, n, 2)?;
                let i = read::lower_bound(n, |i| Ok(self.word(add(4, mul(i, 2)?)?)? < glyph))?;
                if i < n && self.word(add(4, mul(i, 2)?)?)? == glyph {
                    Ok(Some(i))
                } else {
                    Ok(None)
                }
            }
            2 => {
                self.array(4, n, 6)?;
                let i = read::lower_bound(n, |i| Ok(self.word(add(6, mul(i, 6)?)?)? < glyph))?;
                if i == n {
                    return Ok(None);
                }
                let at = add(4, mul(i, 6)?)?;
                let first = self.word(at)?;
                let last = self.word(add(at, 2)?)?;
                if first > last {
                    return Err(FontError::InvalidTable);
                }
                if glyph < first {
                    Ok(None)
                } else {
                    Ok(Some(add(
                        self.u(add(at, 4)?)?,
                        usize::from(glyph.abs_diff(first)),
                    )?))
                }
            }
            _ => Err(FontError::UnsupportedFormat),
        }
    }
    pub(super) fn class(self, glyph: u16) -> Result<u16, FontError> {
        match self.u(0)? {
            1 => {
                let first = self.word(2)?;
                let n = self.u(4)?;
                self.array(6, n, 2)?;
                let i = usize::from(glyph.abs_diff(first));
                if glyph < first || i >= n {
                    Ok(0)
                } else {
                    self.word(add(6, mul(i, 2)?)?)
                }
            }
            2 => {
                let n = self.u(2)?;
                self.array(4, n, 6)?;
                let i = read::lower_bound(n, |i| Ok(self.word(add(6, mul(i, 6)?)?)? < glyph))?;
                if i == n {
                    return Ok(0);
                }
                let at = add(4, mul(i, 6)?)?;
                if self.word(at)? > self.word(add(at, 2)?)? {
                    return Err(FontError::InvalidTable);
                }
                if glyph < self.word(at)? {
                    Ok(0)
                } else {
                    self.word(add(at, 4)?)
                }
            }
            _ => Err(FontError::UnsupportedFormat),
        }
    }
    pub(super) fn tagged(self, tag: [u8; 4]) -> Result<Option<Self>, FontError> {
        self.tagged_at(tag, 0)
    }
    pub(super) fn tagged_at(self, tag: [u8; 4], at: usize) -> Result<Option<Self>, FontError> {
        let n = self.u(at)?;
        let start = add(at, 2)?;
        self.array(start, n, 6)?;
        let i = read::lower_bound(n, |i| Ok(self.tag(add(start, mul(i, 6)?)?)? < tag))?;
        if i < n && self.tag(add(start, mul(i, 6)?)?)? == tag {
            Ok(Some(self.child(add(add(start, 4)?, mul(i, 6)?)?)?))
        } else {
            Ok(None)
        }
    }
}
