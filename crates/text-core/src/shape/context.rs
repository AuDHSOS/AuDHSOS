// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Buffer, Engine, table::Table};
use crate::{
    FontError,
    read::{add, mul, sub},
};

#[derive(Clone, Copy)]
enum Matcher<'a> {
    Glyph,
    Class(Table<'a>),
    Coverage(Table<'a>),
}
impl Matcher<'_> {
    fn matches(self, value: u16, glyph: u16) -> Result<bool, FontError> {
        match self {
            Self::Glyph => Ok(value == glyph),
            Self::Class(table) => Ok(table.class(glyph)? == value),
            Self::Coverage(table) => Ok(table.at(usize::from(value))?.coverage(glyph)?.is_some()),
        }
    }
}
impl<'a> Engine<'a, '_> {
    pub(super) fn context(
        &mut self,
        buffer: &mut Buffer<'_>,
        i: usize,
        lookup: Table<'a>,
        table: Table<'a>,
        chain: bool,
    ) -> Result<Option<usize>, FontError> {
        let format = table.u(0)?;
        if format == 3 {
            let matcher = Matcher::Coverage(table);
            return self.rule(buffer, i, lookup, table, 2, [matcher; 3], chain, true);
        }
        if !matches!(format, 1 | 2) {
            return Err(FontError::UnsupportedFormat);
        }
        let Some(coverage) = table.child(2)?.coverage(buffer.get(i)?.id)? else {
            return Ok(None);
        };
        let mut matchers = [Matcher::Glyph; 3];
        let (index, at) = if format == 1 {
            (coverage, 4)
        } else if chain {
            matchers = [
                Matcher::Class(table.child(4)?),
                Matcher::Class(table.child(6)?),
                Matcher::Class(table.child(8)?),
            ];
            (usize::from(table.child(6)?.class(buffer.get(i)?.id)?), 10)
        } else {
            matchers[1] = Matcher::Class(table.child(4)?);
            (usize::from(table.child(4)?.class(buffer.get(i)?.id)?), 6)
        };
        let n = table.u(at)?;
        table.array(add(at, 2)?, n, 2)?;
        if index >= n {
            return Ok(None);
        }
        let offset = add(add(at, 2)?, mul(index, 2)?)?;
        if table.u(offset)? == 0 {
            return Ok(None);
        }
        let set = table.child(offset)?;
        let n = set.u(0)?;
        set.array(2, n, 2)?;
        for j in 0..n {
            self.tick()?;
            let rule = set.child(add(2, mul(j, 2)?)?)?;
            if let Some(end) = self.rule(buffer, i, lookup, rule, 0, matchers, chain, false)? {
                return Ok(Some(end));
            }
        }
        Ok(None)
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "three OpenType context encodings share one matcher"
    )]
    fn rule(
        &mut self,
        buffer: &mut Buffer<'_>,
        i: usize,
        lookup: Table<'a>,
        table: Table<'a>,
        mut at: usize,
        matchers: [Matcher<'a>; 3],
        chain: bool,
        coverage: bool,
    ) -> Result<Option<usize>, FontError> {
        if chain {
            let n = table.u(at)?;
            at = add(at, 2)?;
            table.array(at, n, 2)?;
            let mut pos = i;
            for j in 0..n {
                let Some(next) = self.next(buffer, pos, lookup, true)? else {
                    return Ok(None);
                };
                pos = next;
                if !matchers[0].matches(table.word(add(at, mul(j, 2)?)?)?, buffer.get(pos)?.id)? {
                    return Ok(None);
                }
            }
            at = add(at, mul(n, 2)?)?;
        }
        let count = table.u(at)?;
        at = add(at, 2)?;
        if count == 0 {
            return Err(FontError::InvalidTable);
        }
        if count > 64 {
            return Err(FontError::LimitExceeded);
        }
        let mut actions = 0;
        if !chain {
            actions = table.u(at)?;
            at = add(at, 2)?;
        }
        let first = usize::from(!coverage);
        table.array(at, sub(count, first)?, 2)?;
        let mut positions = [0; 64];
        positions[0] = i;
        let mut pos = i;
        for j in first..count {
            if j != 0 {
                let Some(next) = self.next(buffer, pos, lookup, false)? else {
                    return Ok(None);
                };
                pos = next;
            }
            if !matchers[1].matches(
                table.word(add(at, mul(sub(j, first)?, 2)?)?)?,
                buffer.get(pos)?.id,
            )? {
                return Ok(None);
            }
            *positions.get_mut(j).ok_or(FontError::LimitExceeded)? = pos;
        }
        at = add(at, mul(sub(count, first)?, 2)?)?;
        let last_order = buffer.get(pos)?.source_order;
        if chain {
            let n = table.u(at)?;
            at = add(at, 2)?;
            table.array(at, n, 2)?;
            for j in 0..n {
                let Some(next) = self.next(buffer, pos, lookup, false)? else {
                    return Ok(None);
                };
                pos = next;
                if !matchers[2].matches(table.word(add(at, mul(j, 2)?)?)?, buffer.get(pos)?.id)? {
                    return Ok(None);
                }
            }
            at = add(at, mul(n, 2)?)?;
            actions = table.u(at)?;
            at = add(at, 2)?;
        }
        table.array(at, actions, 4)?;
        let mask = 1_u16
            .checked_shl(u32::try_from(sub(self.depth, 1)?).map_err(|_| FontError::LimitExceeded)?)
            .ok_or(FontError::LimitExceeded)?;
        for glyph in buffer.glyphs_mut() {
            self.tick()?;
            glyph.context &= !mask;
        }
        for position in positions.get(..count).ok_or(FontError::LimitExceeded)? {
            let mut glyph = buffer.get(*position)?;
            glyph.context |= mask;
            buffer.put(*position, glyph)?;
        }
        self.actions(buffer, table, at, actions, mask)?;
        for (j, g) in buffer.glyphs().iter().enumerate().skip(i) {
            self.tick()?;
            if g.source_order > last_order {
                return Ok(Some(j));
            }
        }
        Ok(Some(buffer.len))
    }
    fn actions(
        &mut self,
        buffer: &mut Buffer<'_>,
        table: Table<'a>,
        at: usize,
        count: usize,
        mask: u16,
    ) -> Result<(), FontError> {
        for a in 0..count {
            self.tick()?;
            let record = add(at, mul(a, 4)?)?;
            let mut sequence = table.u(record)?;
            let mut target = None;
            for (j, glyph) in buffer.glyphs().iter().enumerate() {
                self.tick()?;
                if glyph.context & mask != 0 {
                    if sequence == 0 {
                        target = Some(j);
                        break;
                    }
                    sequence = sub(sequence, 1)?;
                }
            }
            self.apply(
                table.u(add(record, 2)?)?,
                buffer,
                target.ok_or(FontError::InvalidTable)?,
            )?;
        }
        for glyph in buffer.glyphs_mut() {
            self.tick()?;
            glyph.context &= !mask;
        }
        Ok(())
    }
}
