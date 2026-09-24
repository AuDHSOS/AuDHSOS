// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! CFF and CFF2 cubic outlines, Adobe notes 5176 and 5177.

mod blend;
mod charset;
mod dict;
mod index;
mod type2;
use crate::{
    Fixed, Font, FontError,
    read::{self, add, mul},
};
use blend::{Blend, Store};
use dict::Dict;
pub use index::Index;
pub use type2::{Command, Position};

/// A borrowed CFF/CFF2 font with validated indices and dictionary references.
#[derive(Clone, Copy, Debug)]
pub struct Cff<'a> {
    data: &'a [u8],
    cff2: bool,
    top: Dict,
    chars: Index<'a>,
    global: Index<'a>,
    fds: Index<'a>,
    units: u16,
    store: Option<Store<'a>>,
}

impl<'a> Cff<'a> {
    /// Parse the CFF or CFF2 table of an OpenType face.
    /// # Errors
    /// Rejects missing tables, malformed dictionaries, offsets, and glyph counts.
    pub fn parse(font: &Font<'a>) -> Result<Self, FontError> {
        let metrics = font.metrics()?;
        let (data, cff2) = if let Some(t) = font.table(*b"CFF2") {
            (t.data, true)
        } else {
            (font.required(*b"CFF ")?, false)
        };
        let result = Self::parse_table(data, cff2, metrics.head.units_per_em)?;
        if result.chars.len() != usize::from(metrics.maxp.glyph_count) {
            return Err(FontError::InvalidTable);
        }
        Ok(result)
    }

    /// Parse a stand-alone table; `units` is the face's units-per-em.
    /// # Errors
    /// Returns malformed INDEX, DICT, charset, or `FDSelect` errors.
    pub fn parse_table(data: &'a [u8], cff2: bool, units: u16) -> Result<Self, FontError> {
        if read::u8(data, 0)? != if cff2 { 2 } else { 1 }
            || read::u8(data, 1)? != 0
            || !(16..=16384).contains(&units)
        {
            return Err(FontError::UnsupportedFormat);
        }
        let header = usize::from(read::u8(data, 2)?);
        let (top, global) = if cff2 {
            if header < 5 {
                return Err(FontError::InvalidTable);
            }
            let len = usize::from(read::u16(data, 3)?);
            let top = Dict::parse(read::bytes(data, header, len)?, true)?;
            (
                top,
                Index::parse(read::tail(data, add(header, len)?)?, true)?.0,
            )
        } else {
            if header < 4 || !(1..=4).contains(&read::u8(data, 3)?) {
                return Err(FontError::InvalidTable);
            }
            let (names, n) = Index::parse(read::tail(data, header)?, false)?;
            let at = add(header, n)?;
            let (tops, n) = Index::parse(read::tail(data, at)?, false)?;
            if names.len() != 1 || tops.len() != 1 {
                return Err(FontError::InvalidTable);
            }
            let top = Dict::parse(tops.get(0)?, false)?;
            let at = add(at, n)?;
            let (_, n) = Index::parse(read::tail(data, at)?, false)?;
            (top, Index::parse(read::tail(data, add(at, n)?)?, false)?.0)
        };
        if top.charstrings == 0 {
            return Err(FontError::MissingTable);
        }
        let chars = Index::parse(read::tail(data, top.charstrings)?, cff2)?.0;
        if chars.is_empty() {
            return Err(FontError::InvalidTable);
        }
        let fds = if top.fdarray == 0 {
            Index::EMPTY
        } else {
            Index::parse(read::tail(data, top.fdarray)?, cff2)?.0
        };
        if (cff2 || top.cid) && fds.is_empty() {
            return Err(FontError::MissingTable);
        }
        if fds.len() > 256 {
            return Err(FontError::LimitExceeded);
        }
        let result = Self {
            data,
            cff2,
            top,
            chars,
            global,
            fds,
            units,
            store: if top.vstore == 0 {
                None
            } else {
                Some(Store::parse(read::tail(data, top.vstore)?)?)
            },
        };
        if !cff2 {
            result.validate_charset()?;
        }
        if fds.is_empty() {
            result.local(top, true)?;
        } else {
            for i in 0..fds.len() {
                result.local(Dict::parse(fds.get(i)?, cff2)?, true)?;
            }
            if top.fdselect == 0 {
                if fds.len() != 1 {
                    return Err(FontError::MissingTable);
                }
            } else {
                result.validate_select()?;
            }
        }
        Ok(result)
    }

    /// Number of glyph charstrings.
    #[must_use]
    pub const fn glyph_count(&self) -> usize {
        self.chars.len()
    }

    fn local(&self, d: Dict, validate: bool) -> Result<(Index<'a>, usize), FontError> {
        if d.private.0 == 0 {
            return Ok((Index::EMPTY, 0));
        }
        let private = Dict::parse_with(
            read::bytes(self.data, d.private.1, d.private.0)?,
            self.cff2,
            Blend {
                store: self.store,
                ..Blend::default()
            },
        )?;
        if private.subrs == 0 {
            return Ok((Index::EMPTY, private.vsindex));
        }
        if private.subrs < d.private.0 {
            return Err(FontError::Overlap);
        }
        let data = read::tail(self.data, add(d.private.1, private.subrs)?)?;
        let index = if validate {
            Index::parse(data, self.cff2)?.0
        } else {
            Index::validated(data, self.cff2)?
        };
        Ok((index, private.vsindex))
    }

    fn select(&self, glyph: usize) -> Result<usize, FontError> {
        if self.top.fdselect == 0 {
            return Ok(0);
        }
        let data = read::tail(self.data, self.top.fdselect)?;
        match read::u8(data, 0)? {
            0 => Ok(usize::from(read::u8(data, add(1, glyph)?)?)),
            3 | 4 => {
                let wide = read::u8(data, 0)? == 4;
                let (count, header, stride) = if wide {
                    (read::offset(read::u32(data, 1)?)?, 5, 6)
                } else {
                    (usize::from(read::u16(data, 1)?), 3, 3)
                };
                // `validate_select` proves `first` strictly increasing from 0.
                let at = |i: usize| add(header, mul(i, stride)?);
                let after = read::lower_bound(count, |i| {
                    let at = at(i)?;
                    let first = if wide {
                        read::offset(read::u32(data, at)?)?
                    } else {
                        usize::from(read::u16(data, at)?)
                    };
                    Ok(first <= glyph)
                })?;
                let at = at(read::sub(after, 1)?)?;
                Ok(if wide {
                    usize::from(read::u16(data, add(at, 4)?)?)
                } else {
                    usize::from(read::u8(data, add(at, 2)?)?)
                })
            }
            _ => Err(FontError::UnsupportedFormat),
        }
    }

    fn validate_select(&self) -> Result<(), FontError> {
        let data = read::tail(self.data, self.top.fdselect)?;
        match read::u8(data, 0)? {
            0 => {
                for fd in read::bytes(data, 1, self.chars.len())? {
                    if usize::from(*fd) >= self.fds.len() {
                        return Err(FontError::InvalidTable);
                    }
                }
            }
            format @ (3 | 4) => {
                if format == 4 && !self.cff2 {
                    return Err(FontError::UnsupportedFormat);
                }
                let wide = format == 4;
                let (count, header, stride) = if wide {
                    (read::offset(read::u32(data, 1)?)?, 5, 6)
                } else {
                    (usize::from(read::u16(data, 1)?), 3, 3)
                };
                if count == 0 || count > self.chars.len() {
                    return Err(FontError::InvalidTable);
                }
                let mut previous = 0;
                for i in 0..=count {
                    let at = add(header, mul(i, stride)?)?;
                    let first = if wide {
                        read::offset(read::u32(data, at)?)?
                    } else {
                        usize::from(read::u16(data, at)?)
                    };
                    if (i == 0 && first != 0)
                        || (i > 0 && first <= previous)
                        || first > self.chars.len()
                    {
                        return Err(FontError::InvalidTable);
                    }
                    previous = first;
                    if i == count {
                        if first != self.chars.len() {
                            return Err(FontError::InvalidTable);
                        }
                    } else {
                        let fd = if wide {
                            usize::from(read::u16(data, add(at, 4)?)?)
                        } else {
                            usize::from(read::u8(data, add(at, 2)?)?)
                        };
                        if fd >= self.fds.len() {
                            return Err(FontError::InvalidTable);
                        }
                    }
                }
            }
            _ => return Err(FontError::UnsupportedFormat),
        }
        Ok(())
    }

    fn validate_charset(&self) -> Result<(), FontError> {
        match self.top.charset {
            0..=2 => {
                let count = match self.top.charset {
                    0 => 229,
                    1 => 166,
                    _ => 87,
                };
                if self.chars.len() > count {
                    return Err(FontError::InvalidTable);
                }
            }
            at => {
                let data = read::tail(self.data, at)?;
                let format = read::u8(data, 0)?;
                if format == 0 {
                    read::bytes(data, 1, mul(read::sub(self.chars.len(), 1)?, 2)?)?;
                } else if matches!(format, 1 | 2) {
                    let mut glyph = 1;
                    let mut at = 1;
                    while glyph < self.chars.len() {
                        let first = read::u16(data, at)?;
                        let n = if format == 1 {
                            u16::from(read::u8(data, add(at, 2)?)?)
                        } else {
                            read::u16(data, add(at, 2)?)?
                        };
                        first.checked_add(n).ok_or(FontError::Overflow)?;
                        glyph = add(glyph, add(usize::from(n), 1)?)?;
                        at = add(at, if format == 1 { 3 } else { 4 })?;
                    }
                    if glyph != self.chars.len() {
                        return Err(FontError::InvalidTable);
                    }
                } else {
                    return Err(FontError::UnsupportedFormat);
                }
            }
        }
        Ok(())
    }

    /// Decode a glyph into caller-provided cubic path commands.
    ///
    /// Coordinates use font units. On error the output buffer is unspecified.
    /// # Errors
    /// Rejects invalid operators, stacks, subroutines, arithmetic, or output limits.
    pub fn outline(&self, glyph: u16, commands: &mut [Command]) -> Result<usize, FontError> {
        self.outline_inner(glyph, commands, false, &[])
    }

    /// Decode at explicit normalized CFF2 variation coordinates.
    /// # Errors
    /// Rejects wrong axis counts, coordinates outside [-1, 1], or malformed outlines.
    pub fn outline_instance(
        &self,
        glyph: u16,
        coords: &[Fixed],
        commands: &mut [Command],
    ) -> Result<usize, FontError> {
        if let Some(store) = self.store {
            store.validate_coords(coords)?;
        } else if !coords.is_empty() {
            return Err(FontError::InvalidTable);
        }
        self.outline_inner(glyph, commands, false, coords)
    }

    fn outline_inner(
        &self,
        glyph: u16,
        commands: &mut [Command],
        component: bool,
        coords: &[Fixed],
    ) -> Result<usize, FontError> {
        let id = usize::from(glyph);
        let program = self.chars.get(id)?;
        let fd = if self.fds.is_empty() {
            self.top
        } else {
            Dict::parse(self.fds.get(self.select(id)?)?, self.cff2)?
        };
        let (local, index) = self.local(fd, false)?;
        let (count, seac) = type2::decode(
            program,
            local,
            self.global,
            self.cff2,
            commands,
            Blend {
                store: self.store,
                index,
                coords,
            },
        )?;
        if let Some([dx, dy, base, accent]) = seac {
            if component || self.top.cid || count != 0 {
                return Err(FontError::InvalidTable);
            }
            let base = self.standard_glyph(base)?;
            let accent = self.standard_glyph(accent)?;
            let first = self.outline_inner(base, commands, true, coords)?;
            let tail = commands.get_mut(first..).ok_or(FontError::BufferTooSmall)?;
            let second = self.outline_inner(accent, tail, true, coords)?;
            let origin = self.transform(Position::default(), fd.matrix)?;
            let shift = self.transform(Position { x: dx, y: dy }, fd.matrix)?;
            let dx = shift.x.checked_sub(origin.x)?;
            let dy = shift.y.checked_sub(origin.y)?;
            for command in tail.get_mut(..second).ok_or(FontError::BufferTooSmall)? {
                command.transform(|p| {
                    Ok(Position {
                        x: p.x.checked_add(dx)?,
                        y: p.y.checked_add(dy)?,
                    })
                })?;
            }
            return add(first, second);
        }
        for command in commands.get_mut(..count).ok_or(FontError::BufferTooSmall)? {
            command.transform(|p| self.transform(p, fd.matrix))?;
        }
        Ok(count)
    }

    fn standard_glyph(&self, code: Fixed) -> Result<u16, FontError> {
        let sid = *charset::STANDARD
            .get(dict::offset(code)?)
            .ok_or(FontError::InvalidTable)?;
        if sid == 0 {
            return Err(FontError::InvalidTable);
        }
        let id = self.glyph(sid)?.ok_or(FontError::GlyphIndex)?;
        u16::try_from(id).map_err(|_| FontError::GlyphIndex)
    }

    /// Lowest glyph id above 0 with `sid`; O(R), O(G) for format 0 and
    /// predefined charsets.
    fn glyph(&self, sid: u16) -> Result<Option<usize>, FontError> {
        let count = self.chars.len();
        let table: &[u16] = match self.top.charset {
            0 => return Ok(Some(usize::from(sid)).filter(|id| *id < count)),
            1 => &charset::EXPERT,
            2 => &charset::EXPERT_SUBSET,
            start => {
                let data = read::tail(self.data, start)?;
                let format = read::u8(data, 0)?;
                if format == 0 {
                    for id in 1..count {
                        if read::u16(data, add(1, mul(read::sub(id, 1)?, 2)?)?)? == sid {
                            return Ok(Some(id));
                        }
                    }
                    return Ok(None);
                }
                let mut glyph = 1;
                let mut at = 1;
                while glyph < count {
                    let first = read::u16(data, at)?;
                    let n = if format == 1 {
                        u16::from(read::u8(data, add(at, 2)?)?)
                    } else {
                        read::u16(data, add(at, 2)?)?
                    };
                    if let Some(offset) = sid.checked_sub(first)
                        && offset <= n
                    {
                        return add(glyph, usize::from(offset)).map(Some);
                    }
                    glyph = add(glyph, add(usize::from(n), 1)?)?;
                    at = add(at, if format == 1 { 3 } else { 4 })?;
                }
                return Ok(None);
            }
        };
        Ok(table
            .get(1..count)
            .and_then(|sids| sids.iter().position(|s| *s == sid))
            .map(|i| i.saturating_add(1)))
    }

    fn transform(&self, p: Position, fd: Option<[Fixed; 6]>) -> Result<Position, FontError> {
        let mut p = if self.fds.is_empty() {
            p
        } else {
            apply_matrix(p, fd)?
        };
        if let Some(matrix) = self.top.matrix {
            p = apply_matrix(p, Some(matrix))?;
            p.x = p.x.mul_ratio(i64::from(self.units), 1)?;
            p.y = p.y.mul_ratio(i64::from(self.units), 1)?;
        } else {
            p.x = p.x.mul_ratio(i64::from(self.units), 1000)?;
            p.y = p.y.mul_ratio(i64::from(self.units), 1000)?;
        }
        Ok(p)
    }
}

#[expect(
    clippy::many_single_char_names,
    reason = "standard six-entry affine matrix notation"
)]
fn apply_matrix(p: Position, m: Option<[Fixed; 6]>) -> Result<Position, FontError> {
    let Some([a, b, c, d, e, f]) = m else {
        return Ok(p);
    };
    Ok(Position {
        x: p.x
            .checked_mul(a)?
            .checked_add(p.y.checked_mul(c)?)?
            .checked_add(e)?,
        y: p.x
            .checked_mul(b)?
            .checked_add(p.y.checked_mul(d)?)?
            .checked_add(f)?,
    })
}
