// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Buffer, Engine, table::Table};
use crate::{
    FontError,
    read::{add, mul, sub},
};

impl<'a> Engine<'a, '_> {
    pub(super) fn substitute(
        &mut self,
        buffer: &mut Buffer<'_>,
        i: usize,
        lookup: Table<'a>,
        kind: usize,
        table: Table<'a>,
    ) -> Result<Option<usize>, FontError> {
        if kind == 5 || kind == 6 {
            return self.context(buffer, i, lookup, table, kind == 6);
        }
        if !(1..=8).contains(&kind) || kind == 7 {
            return Err(FontError::UnsupportedFormat);
        }
        let mut glyph = buffer.get(i)?;
        let Some(c) = table.child(2)?.coverage(glyph.id)? else {
            return Ok(None);
        };
        match kind {
            1 => {
                glyph.id = match table.u(0)? {
                    1 => glyph.id.wrapping_add(table.word(4)?),
                    2 => {
                        if c >= table.u(4)? {
                            return Err(FontError::InvalidTable);
                        }
                        table.word(add(6, mul(c, 2)?)?)?
                    }
                    _ => return Err(FontError::UnsupportedFormat),
                };
                self.replace(buffer, i, glyph)?;
                Ok(Some(add(i, 1)?))
            }
            2 | 3 => {
                if table.u(0)? != 1 || c >= table.u(4)? {
                    return Err(FontError::InvalidTable);
                }
                let sequence = table.child(add(6, mul(c, 2)?)?)?;
                let n = sequence.u(0)?;
                sequence.array(2, n, 2)?;
                if kind == 3 {
                    let alternate = usize::from(self.feature.value);
                    if alternate == 0 || alternate > n {
                        return Ok(None);
                    }
                    glyph.id = sequence.word(mul(alternate, 2)?)?;
                    self.replace(buffer, i, glyph)?;
                    return Ok(Some(add(i, 1)?));
                }
                if n == 0 {
                    buffer.remove(i)?;
                    return Ok(Some(i));
                }
                glyph.id = sequence.word(2)?;
                glyph.components = 1;
                if glyph.class == 2 {
                    glyph.class = 1;
                }
                self.replace(buffer, i, glyph)?;
                for j in 1..n {
                    self.tick()?;
                    let mut g = glyph;
                    g.id = sequence.word(add(2, mul(j, 2)?)?)?;
                    buffer.insert(add(i, j)?, g)?;
                    self.replace(buffer, add(i, j)?, g)?;
                }
                Ok(Some(add(i, n)?))
            }
            4 => self.ligature(buffer, i, lookup, table, c),
            8 => {
                if table.u(0)? != 1 {
                    return Err(FontError::UnsupportedFormat);
                }
                let mut at = 4;
                for back in [true, false] {
                    let n = table.u(at)?;
                    at = add(at, 2)?;
                    table.array(at, n, 2)?;
                    let mut pos = i;
                    for j in 0..n {
                        let Some(next) = self.next(buffer, pos, lookup, back)? else {
                            return Ok(None);
                        };
                        pos = next;
                        if table
                            .child(add(at, mul(j, 2)?)?)?
                            .coverage(buffer.get(pos)?.id)?
                            .is_none()
                        {
                            return Ok(None);
                        }
                    }
                    at = add(at, mul(n, 2)?)?;
                }
                if c >= table.u(at)? {
                    return Err(FontError::InvalidTable);
                }
                glyph.id = table.word(add(add(at, 2)?, mul(c, 2)?)?)?;
                self.replace(buffer, i, glyph)?;
                Ok(Some(add(i, 1)?))
            }
            _ => Err(FontError::UnsupportedFormat),
        }
    }
    fn ligature(
        &mut self,
        buffer: &mut Buffer<'_>,
        i: usize,
        lookup: Table<'a>,
        table: Table<'a>,
        c: usize,
    ) -> Result<Option<usize>, FontError> {
        if table.u(0)? != 1 || c >= table.u(4)? {
            return Err(FontError::InvalidTable);
        }
        let set = table.child(add(6, mul(c, 2)?)?)?;
        let n = set.u(0)?;
        set.array(2, n, 2)?;
        for r in 0..n {
            self.tick()?;
            let rule = set.child(add(2, mul(r, 2)?)?)?;
            let count = rule.u(2)?;
            if count == 0 {
                return Err(FontError::InvalidTable);
            }
            if count > 64 {
                return Err(FontError::LimitExceeded);
            }
            rule.array(4, sub(count, 1)?, 2)?;
            let mut positions = [0; 64];
            *positions.first_mut().ok_or(FontError::LimitExceeded)? = i;
            let mut pos = i;
            let mut matched = true;
            for j in 1..count {
                let Some(next) = self.next(buffer, pos, lookup, false)? else {
                    matched = false;
                    break;
                };
                pos = next;
                if buffer.get(pos)?.id != rule.word(add(2, mul(j, 2)?)?)? {
                    matched = false;
                    break;
                }
                *positions.get_mut(j).ok_or(FontError::LimitExceeded)? = pos;
            }
            if !matched {
                continue;
            }
            let mut first = buffer.get(i)?;
            let end = buffer.get(pos)?.end;
            let mut component = 0;
            let mut all_marks = true;
            for at in i..=pos {
                let mut g = buffer.get(at)?;
                if positions
                    .get(..count)
                    .ok_or(FontError::LimitExceeded)?
                    .contains(&at)
                {
                    component = add(component, g.components.max(1))?;
                    all_marks &= g.class == 3;
                    first.context |= g.context;
                } else {
                    g.component = sub(component, 1)?;
                }
                g.start = first.start;
                g.end = end;
                buffer.put(at, g)?;
            }
            let mut trailing = add(pos, 1)?;
            while trailing < buffer.len {
                let mut mark = buffer.get(trailing)?;
                if mark.start >= end || (mark.class != 3 && !mark.hidden) {
                    break;
                }
                mark.start = first.start;
                mark.end = end;
                mark.component = sub(component, 1)?;
                buffer.put(trailing, mark)?;
                trailing = add(trailing, 1)?;
            }
            first.id = rule.word(0)?;
            first.end = end;
            first.class = if all_marks { 3 } else { 2 };
            first.components = component;
            self.replace(buffer, i, first)?;
            // Ascending removal keeps the gap moving forward.
            for (removed, at) in positions
                .get(1..count)
                .ok_or(FontError::LimitExceeded)?
                .iter()
                .enumerate()
            {
                buffer.remove(sub(*at, removed)?)?;
            }
            return Ok(Some(add(i, 1)?));
        }
        Ok(None)
    }
}
