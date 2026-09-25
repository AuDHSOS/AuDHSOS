// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Buffer, Engine, table::Table};
use crate::{
    Fixed, FontError,
    read::{add, mul, sub},
};

/// Last attachment search of one lookup in a pass.
#[derive(Clone, Copy)]
pub(super) struct Attach {
    lookup: usize,
    marks: bool,
    from: usize,
    found: Option<usize>,
}

/// Resolution progress of one glyph in `finish`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum State {
    #[default]
    Unresolved,
    Progress,
    Done,
}

fn size(format: u16) -> Result<usize, FontError> {
    if format & 0xff00 != 0 {
        return Err(FontError::InvalidTable);
    }
    mul(
        usize::try_from(format.count_ones()).map_err(|_| FontError::Overflow)?,
        2,
    )
}
impl<'a> Engine<'a, '_> {
    fn value(
        &self,
        buffer: &mut Buffer<'_>,
        i: usize,
        base: Table<'_>,
        record: Table<'_>,
        format: u16,
    ) -> Result<(), FontError> {
        size(format)?;
        let mut values = [Fixed::ZERO; 4];
        let mut at = 0;
        for bit in 0..8 {
            if format & (1 << bit) == 0 {
                continue;
            }
            let value = if bit < 4 {
                Fixed::from_i32(i32::from(record.signed(at)?))
            } else if record.u(at)? == 0 {
                Fixed::ZERO
            } else {
                self.gdef.device(base.at(record.u(at)?)?, self.coords)?
            };
            let slot = values.get_mut(bit % 4).ok_or(FontError::InvalidTable)?;
            *slot = slot.checked_add(value)?;
            at = add(at, 2)?;
        }
        let mut glyph = buffer.get(i)?;
        glyph.x = glyph.x.checked_add(values[0])?;
        glyph.y = glyph.y.checked_add(values[1])?;
        glyph.advance = glyph.advance.checked_add(values[2])?;
        buffer.put(i, glyph)
    }
    fn anchor(&self, table: Table<'_>) -> Result<(Fixed, Fixed), FontError> {
        let mut x = Fixed::from_i32(i32::from(table.signed(2)?));
        let mut y = Fixed::from_i32(i32::from(table.signed(4)?));
        match table.u(0)? {
            1 => {}
            2 => {
                table.word(6)?;
            }
            3 => {
                if table.u(6)? != 0 {
                    x = x.checked_add(self.gdef.device(table.child(6)?, self.coords)?)?;
                }
                if table.u(8)? != 0 {
                    y = y.checked_add(self.gdef.device(table.child(8)?, self.coords)?)?;
                }
            }
            _ => return Err(FontError::UnsupportedFormat),
        }
        Ok((x, y))
    }
    pub(super) fn position(
        &mut self,
        buffer: &mut Buffer<'_>,
        i: usize,
        lookup: Table<'a>,
        kind: usize,
        table: Table<'a>,
    ) -> Result<Option<usize>, FontError> {
        if kind == 7 || kind == 8 {
            return self.context(buffer, i, lookup, table, kind == 8);
        }
        if !(1..=6).contains(&kind) {
            return Err(FontError::UnsupportedFormat);
        }
        let Some(coverage_index) = table.child(2)?.coverage(buffer.get(i)?.id)? else {
            return Ok(None);
        };
        match kind {
            1 => {
                let format = table.word(4)?;
                let at = match table.u(0)? {
                    1 => 6,
                    2 => {
                        if coverage_index >= table.u(6)? {
                            return Err(FontError::InvalidTable);
                        }
                        add(8, mul(coverage_index, size(format)?)?)?
                    }
                    _ => return Err(FontError::UnsupportedFormat),
                };
                self.value(buffer, i, table, table.at(at)?, format)?;
                Ok(Some(add(i, 1)?))
            }
            2 => self.pair(buffer, i, lookup, table, coverage_index),
            3 => self.cursive(buffer, i, lookup, table, coverage_index),
            4..=6 => self.mark(buffer, i, lookup, table, coverage_index, kind),
            _ => Err(FontError::UnsupportedFormat),
        }
    }
    fn pair(
        &mut self,
        buffer: &mut Buffer<'_>,
        i: usize,
        lookup: Table<'a>,
        table: Table<'a>,
        coverage_index: usize,
    ) -> Result<Option<usize>, FontError> {
        let Some(j) = self.next(buffer, i, lookup, false)? else {
            return Ok(None);
        };
        let f1 = table.word(4)?;
        let f2 = table.word(6)?;
        let s1 = size(f1)?;
        let stride = add(s1, size(f2)?)?;
        let (base, record) = match table.u(0)? {
            1 => {
                if coverage_index >= table.u(8)? {
                    return Err(FontError::InvalidTable);
                }
                let set = table.child(add(10, mul(coverage_index, 2)?)?)?;
                let n = set.u(0)?;
                let width = add(2, stride)?;
                set.array(2, n, width)?;
                let index = crate::read::lower_bound(n, |r| {
                    Ok(set.word(add(2, mul(r, width)?)?)? < buffer.get(j)?.id)
                })?;
                if index == n || set.word(add(2, mul(index, width)?)?)? != buffer.get(j)?.id {
                    return Ok(None);
                }
                (set, set.at(add(4, mul(index, width)?)?)?)
            }
            2 => {
                let c1 = usize::from(table.child(8)?.class(buffer.get(i)?.id)?);
                let c2 = usize::from(table.child(10)?.class(buffer.get(j)?.id)?);
                let n1 = table.u(12)?;
                let n2 = table.u(14)?;
                if c1 >= n1 || c2 >= n2 {
                    return Err(FontError::InvalidTable);
                }
                table.array(16, mul(n1, n2)?, stride)?;
                (
                    table,
                    table.at(add(16, mul(add(mul(c1, n2)?, c2)?, stride)?)?)?,
                )
            }
            _ => return Err(FontError::UnsupportedFormat),
        };
        self.value(buffer, i, base, record, f1)?;
        self.value(buffer, j, base, record.at(s1)?, f2)?;
        Ok(Some(if f2 == 0 { j } else { add(j, 1)? }))
    }
    fn cursive(
        &mut self,
        buffer: &mut Buffer<'_>,
        i: usize,
        lookup: Table<'a>,
        table: Table<'a>,
        coverage_index: usize,
    ) -> Result<Option<usize>, FontError> {
        if table.u(0)? != 1 || coverage_index >= table.u(4)? {
            return Err(FontError::InvalidTable);
        }
        let Some(j) = self.next(buffer, i, lookup, false)? else {
            return Ok(None);
        };
        let Some(d) = table.child(2)?.coverage(buffer.get(j)?.id)? else {
            return Ok(None);
        };
        if d >= table.u(4)? {
            return Err(FontError::InvalidTable);
        }
        let exit = add(8, mul(coverage_index, 4)?)?;
        let entry = add(6, mul(d, 4)?)?;
        if table.u(exit)? == 0 || table.u(entry)? == 0 {
            return Ok(None);
        }
        let (ex, ey) = self.anchor(table.child(exit)?)?;
        let (enx, eny) = self.anchor(table.child(entry)?)?;
        let mut a = buffer.get(i)?;
        let mut second = buffer.get(j)?;
        if self.rtl {
            let delta = ex.checked_add(a.x)?;
            a.advance = a.advance.checked_sub(delta)?;
            a.x = a.x.checked_sub(delta)?;
            second.advance = enx.checked_add(second.x)?;
        } else {
            a.advance = ex.checked_add(a.x)?;
            let delta = enx.checked_add(second.x)?;
            second.advance = second.advance.checked_sub(delta)?;
            second.x = second.x.checked_sub(delta)?;
        }
        if lookup.word(2)? & 1 != 0 {
            a.parent = Some(j);
            a.cursive = true;
            a.y = eny.checked_sub(ey)?;
        } else {
            second.parent = Some(i);
            second.cursive = true;
            second.y = ey.checked_sub(eny)?;
        }
        buffer.put(i, a)?;
        buffer.put(j, second)?;
        Ok(Some(j))
    }
    /// Nearest preceding eligible glyph of `i`, skipping marks when `marks`.
    /// Reuses the lookup's previous search of the pass once it reaches that
    /// search's start, so a run of n marks costs O(n).
    fn attachment(
        &mut self,
        buffer: &Buffer<'_>,
        i: usize,
        lookup: Table<'a>,
        marks: bool,
    ) -> Result<Option<usize>, FontError> {
        let index = *self
            .active
            .get(sub(self.depth, 1)?)
            .ok_or(FontError::InvalidTable)?;
        // Slot per lookup, so a context alternating lookups keeps each search.
        let slot = index & 15;
        let mut at = i;
        let found = loop {
            if let Some(last) = self.attach.get(slot).copied().flatten()
                && last.from == at
                && last.marks == marks
                && last.lookup == index
            {
                break last.found;
            }
            self.tick()?;
            let Some(n) = at.checked_sub(1) else {
                break None;
            };
            at = n;
            if !self.eligible(buffer, at, lookup)? {
                continue;
            }
            let glyph = buffer.get(at)?;
            let class = match self.gdef.class(glyph.id)? {
                0 => glyph.class,
                class => class,
            };
            if !marks || class != 3 {
                break Some(at);
            }
        };
        *self.attach.get_mut(slot).ok_or(FontError::InvalidTable)? = Some(Attach {
            lookup: index,
            marks,
            from: i,
            found,
        });
        Ok(found)
    }
    fn mark(
        &mut self,
        buffer: &mut Buffer<'_>,
        i: usize,
        lookup: Table<'a>,
        table: Table<'a>,
        coverage_index: usize,
        kind: usize,
    ) -> Result<Option<usize>, FontError> {
        if table.u(0)? != 1 {
            return Err(FontError::UnsupportedFormat);
        }
        let Some(parent) = self.attachment(buffer, i, lookup, kind != 6)? else {
            return Ok(None);
        };
        let Some(d) = table.child(4)?.coverage(buffer.get(parent)?.id)? else {
            return Ok(None);
        };
        let marks = table.child(8)?;
        if coverage_index >= marks.u(0)? {
            return Err(FontError::InvalidTable);
        }
        let record = add(2, mul(coverage_index, 4)?)?;
        let class = marks.u(record)?;
        let classes = table.u(6)?;
        if class >= classes {
            return Err(FontError::InvalidTable);
        }
        let mark = self.anchor(marks.child(add(record, 2)?)?)?;
        let bases = table.child(10)?;
        if d >= bases.u(0)? {
            return Err(FontError::InvalidTable);
        }
        let (base, at) = if kind == 5 {
            let lig = bases.child(add(2, mul(d, 2)?)?)?;
            let n = lig.u(0)?;
            if n == 0 {
                return Err(FontError::InvalidTable);
            }
            let mark = buffer.get(i)?;
            let component = if mark.start == buffer.get(parent)?.start {
                mark.component.min(sub(n, 1)?)
            } else {
                sub(n, 1)?
            };
            (lig, add(2, mul(add(mul(component, classes)?, class)?, 2)?)?)
        } else {
            (bases, add(2, mul(add(mul(d, classes)?, class)?, 2)?)?)
        };
        if base.u(at)? == 0 {
            return Ok(None);
        }
        let anchor = self.anchor(base.child(at)?)?;
        let mut glyph = buffer.get(i)?;
        glyph.parent = Some(parent);
        glyph.cursive = false;
        glyph.x = anchor.0.checked_sub(mark.0)?;
        glyph.y = anchor.1.checked_sub(mark.1)?;
        buffer.put(i, glyph)?;
        Ok(Some(add(i, 1)?))
    }
}

/// Resolve mark/cursive attachment chains after all positioning stages in O(n).
/// # Errors
/// Rejects cyclic attachments, invalid parents, and overflow; the buffer's
/// attachments are then unspecified.
pub fn finish(buffer: &mut Buffer<'_>, rtl: bool) -> Result<(), FontError> {
    let mut total = Fixed::ZERO;
    if rtl {
        for g in buffer.glyphs() {
            total = total.checked_add(g.advance)?;
        }
    }
    for g in buffer.glyphs_mut() {
        g.state = State::Unresolved;
        if rtl {
            total = total.checked_sub(g.advance)?;
            g.origin = total;
        } else {
            g.origin = total;
            total = total.checked_add(g.advance)?;
        }
    }
    for i in 0..buffer.len {
        // Ascend to a root or a resolved glyph; `parent` then names the child.
        let mut child = None;
        let mut at = i;
        let mut above = loop {
            let mut g = buffer.get(at)?;
            match g.state {
                State::Done => break Some(at),
                State::Progress => return Err(FontError::Cycle),
                State::Unresolved => {}
            }
            let up = g.parent;
            g.state = State::Progress;
            g.parent = child;
            buffer.put(at, g)?;
            child = Some(at);
            match up {
                Some(p) => at = p,
                None => break None,
            }
        };
        // Descend, resolving each glyph from its resolved parent.
        while let Some(c) = child {
            let mut g = buffer.get(c)?;
            let mut x = g.x;
            let mut y = g.y;
            if let Some(p) = above {
                let p = buffer.get(p)?;
                y = y.checked_add(p.resolved.1)?;
                if !g.cursive {
                    x = x
                        .checked_add(p.origin.checked_sub(g.origin)?)?
                        .checked_add(p.resolved.0)?;
                }
            }
            g.resolved = (x, y);
            g.state = State::Done;
            child = g.parent;
            g.parent = None;
            buffer.put(c, g)?;
            above = Some(c);
        }
    }
    for g in buffer.glyphs_mut() {
        g.x = g.resolved.0;
        g.y = g.resolved.1;
        g.state = State::Unresolved;
    }
    Ok(())
}
