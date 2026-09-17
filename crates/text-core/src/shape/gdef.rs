// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::table::Table;
use crate::{
    Fixed, FontError,
    read::{add, mul},
    variation::ItemStore,
};

/// Borrowed glyph classes, mark filtering, carets, and variation deltas.
#[derive(Clone, Copy, Debug, Default)]
pub struct Gdef<'a> {
    table: Option<Table<'a>>,
    store: Option<ItemStore<'a>>,
}

/// A ligature caret in design units or an unhinted outline point index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caret {
    /// Unhinted design coordinate.
    Coordinate(Fixed),
    /// Contour point index in the instance outline.
    Point(u16),
}

impl<'a> Gdef<'a> {
    /// Read GDEF 1.0, 1.2, or 1.3.
    /// # Errors
    /// Rejects unsupported headers and malformed variation stores.
    pub fn parse(data: &'a [u8], axes: usize) -> Result<Self, FontError> {
        let table = Table(data);
        if table.word(0)? != 1 || !matches!(table.word(2)?, 0 | 2 | 3) {
            return Err(FontError::UnsupportedFormat);
        }
        table.array(0, 1, 12)?;
        if table.word(2)? >= 2 {
            table.array(0, 1, 14)?;
        }
        let store = if table.word(2)? >= 3 && table.long(14)? != 0 {
            Some(ItemStore::parse(table.child32(14)?.0, axes)?)
        } else {
            None
        };
        Ok(Self {
            table: Some(table),
            store,
        })
    }
    /// Return a GDEF glyph class; absent definitions are class zero.
    /// # Errors
    /// Rejects malformed class data.
    pub fn class(self, glyph: u16) -> Result<u16, FontError> {
        let c = self.class_at(4, glyph)?;
        if c > 4 {
            return Err(FontError::InvalidTable);
        }
        Ok(c)
    }
    fn class_at(self, offset: usize, glyph: u16) -> Result<u16, FontError> {
        let Some(table) = self.table else {
            return Ok(0);
        };
        if table.u(offset)? == 0 {
            Ok(0)
        } else {
            table.child(offset)?.class(glyph)
        }
    }
    pub(super) fn ignored(
        self,
        glyph: u16,
        inferred: u16,
        flags: u16,
        set: usize,
    ) -> Result<bool, FontError> {
        let class = match self.class(glyph)? {
            0 => inferred,
            c => c,
        };
        if (class == 1 && flags & 2 != 0) || (class == 2 && flags & 4 != 0) {
            return Ok(true);
        }
        if class != 3 {
            return Ok(false);
        }
        if flags & 8 != 0 {
            return Ok(true);
        }
        if flags & 16 != 0 {
            let table = self.table.ok_or(FontError::InvalidTable)?;
            if table.word(2)? < 2 {
                return Err(FontError::InvalidTable);
            }
            let sets = table.child(12)?;
            if sets.u(0)? != 1 || set >= sets.u(2)? {
                return Err(FontError::InvalidTable);
            }
            return Ok(sets
                .child32(add(4, mul(set, 4)?)?)?
                .coverage(glyph)?
                .is_none());
        }
        Ok(flags >> 8 != 0 && self.class_at(10, glyph)? != flags >> 8)
    }
    pub(super) fn device(self, table: Table<'_>, coords: &[Fixed]) -> Result<Fixed, FontError> {
        let format = table.word(4)?;
        if format == 0x8000 {
            return self.store.ok_or(FontError::InvalidTable)?.delta(
                table.word(0)?,
                table.word(2)?,
                coords,
            );
        }
        if !(1..=3).contains(&format) || table.word(0)? > table.word(2)? {
            return Err(FontError::InvalidTable);
        }
        let n = add(usize::from(table.word(2)?.abs_diff(table.word(0)?)), 1)?;
        let bits = 1_usize
            .checked_shl(u32::from(format))
            .ok_or(FontError::Overflow)?;
        table.array(6, add(mul(n, bits)?, 15)? / 16, 2)?;
        Ok(Fixed::ZERO)
    }
    /// Copy outline attachment point indices from `AttachList`.
    /// # Errors
    /// Rejects malformed counts and insufficient output capacity.
    pub fn attachment_points(self, glyph: u16, out: &mut [u16]) -> Result<usize, FontError> {
        let Some(table) = self.table else {
            return Ok(0);
        };
        if table.u(6)? == 0 {
            return Ok(0);
        }
        let list = table.child(6)?;
        let Some(index) = list.child(0)?.coverage(glyph)? else {
            return Ok(0);
        };
        if index >= list.u(2)? {
            return Err(FontError::InvalidTable);
        }
        let points = list.child(add(4, mul(index, 2)?)?)?;
        let n = points.u(0)?;
        points.array(2, n, 2)?;
        if n > out.len() {
            return Err(FontError::BufferTooSmall);
        }
        for (i, slot) in out.iter_mut().take(n).enumerate() {
            *slot = points.word(add(2, mul(i, 2)?)?)?;
        }
        Ok(n)
    }
    /// Copy carets in stored order; point carets are resolved against the instance outline.
    /// # Errors
    /// Rejects malformed data, variation indices, or insufficient output.
    pub fn carets(
        self,
        glyph: u16,
        coords: &[Fixed],
        out: &mut [Caret],
    ) -> Result<usize, FontError> {
        let Some(table) = self.table else {
            return Ok(0);
        };
        if table.u(8)? == 0 {
            return Ok(0);
        }
        let list = table.child(8)?;
        let Some(i) = list.child(0)?.coverage(glyph)? else {
            return Ok(0);
        };
        if i >= list.u(2)? {
            return Err(FontError::InvalidTable);
        }
        let lig = list.child(add(4, mul(i, 2)?)?)?;
        let n = lig.u(0)?;
        if n > out.len() {
            return Err(FontError::BufferTooSmall);
        }
        for (i, slot) in out.iter_mut().take(n).enumerate() {
            let c = lig.child(add(2, mul(i, 2)?)?)?;
            *slot = match c.u(0)? {
                1 => Caret::Coordinate(Fixed::from_i32(i32::from(c.signed(2)?))),
                2 => Caret::Point(c.word(2)?),
                3 => {
                    let x = Fixed::from_i32(i32::from(c.signed(2)?));
                    Caret::Coordinate(if c.u(4)? == 0 {
                        x
                    } else {
                        x.checked_add(self.device(c.child(4)?, coords)?)?
                    })
                }
                _ => return Err(FontError::UnsupportedFormat),
            };
        }
        Ok(n)
    }
}
