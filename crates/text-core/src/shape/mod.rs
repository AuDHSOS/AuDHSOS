// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Bounded OpenType substitution and positioning in caller-owned glyph storage.

mod context;
mod features;
mod gdef;
mod position;
mod script;
mod substitute;
mod table;
use crate::{
    Fixed, FontError,
    read::{add, mul, sub},
};
pub use gdef::{Caret, Gdef};
pub use position::finish;
pub use script::{POSITION_FEATURES, SUBSTITUTION_FEATURES, prepare, script_tag, supported};
use table::Table;

/// A glyph and its original UTF-8 cluster interval; coordinates use font units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Glyph {
    /// Font glyph index.
    pub id: u16,
    /// Inclusive UTF-8 cluster start.
    pub start: usize,
    /// Exclusive UTF-8 cluster end.
    pub end: usize,
    /// Horizontal advance.
    pub advance: Fixed,
    /// Horizontal placement relative to the pen.
    pub x: Fixed,
    /// Vertical placement, positive upward.
    pub y: Fixed,
    /// Inferred GDEF class when the font has no class definition.
    pub class: u16,
    /// Arabic feature mask: 1 isol, 2 init, 4 medi, 8 fina; zero otherwise.
    pub form: u8,
    /// Hangul feature mask: 1 ljmo, 2 vjmo, 4 tjmo; zero otherwise.
    pub jamo: u8,
    pub(crate) component: usize,
    pub(crate) components: usize,
    /// Default-ignorable glyph; contributes no advance or visible ink.
    pub hidden: bool,
    code: u32,
    mirror: bool,
    context: u16,
    source_order: usize,
    origin: Fixed,
    resolved: (Fixed, Fixed),
    state: position::State,
    parent: Option<usize>,
    cursive: bool,
}

impl Glyph {
    #[cfg(test)]
    pub(crate) const fn code(&self) -> u32 {
        self.code
    }
    /// Initialize a glyph with its source cluster and zero positions.
    #[must_use]
    pub fn new(id: u16, start: usize, end: usize) -> Self {
        Self {
            id,
            start,
            end,
            components: 1,
            ..Self::default()
        }
    }
}

/// Mutable glyph prefix with checked expansion into caller capacity.
///
/// The `data.len() - len` free slots form a gap before logical glyph `gap`.
/// `insert` and `remove` move the gap to their index, so a lookup pass copies
/// O(n + work) glyphs. `Engine::run` closes the gap (`gap == len`) before it
/// returns; `glyphs` and `set_advances` require a closed gap.
pub struct Buffer<'a> {
    data: &'a mut [Glyph],
    len: usize,
    gap: usize,
    inserted: usize,
    /// Glyphs copied by gap moves.
    #[cfg(test)]
    pub(crate) moved: usize,
}
impl<'a> Buffer<'a> {
    /// Adopt an initialized prefix.
    /// # Errors
    /// Rejects a length greater than capacity.
    pub const fn new(data: &'a mut [Glyph], len: usize) -> Result<Self, FontError> {
        if len > data.len() {
            return Err(FontError::BufferTooSmall);
        }
        Ok(Self {
            data,
            len,
            gap: len,
            inserted: 0,
            #[cfg(test)]
            moved: 0,
        })
    }
    /// Return the initialized glyphs.
    #[must_use]
    pub fn glyphs(&self) -> &[Glyph] {
        self.data.get(..self.len).unwrap_or(&[])
    }
    /// Modify the initialized glyphs, for example to install instance advances.
    pub fn glyphs_mut(&mut self) -> &mut [Glyph] {
        self.data.get_mut(..self.len).unwrap_or(&mut [])
    }
    /// Override inferred glyph classes with explicit GDEF definitions.
    /// # Errors
    /// Rejects malformed class definitions.
    pub fn set_classes(&mut self, gdef: Gdef<'_>) -> Result<(), FontError> {
        for glyph in self.glyphs_mut() {
            let class = gdef.class(glyph.id)?;
            if class != 0 {
                glyph.class = class;
            }
        }
        Ok(())
    }
    /// Install instance advances and reorder spacing Hangul tone marks.
    /// # Errors
    /// Propagates glyph metric and numeric errors.
    pub fn set_advances(
        &mut self,
        mut advance: impl FnMut(u16) -> Result<Fixed, FontError>,
    ) -> Result<(), FontError> {
        for glyph in self.glyphs_mut() {
            let value = advance(glyph.id)?;
            glyph.advance =
                if glyph.hidden || (glyph.class == 3 && !matches!(glyph.code, 0x302e | 0x302f)) {
                    Fixed::ZERO
                } else {
                    value
                };
        }
        for i in 0..self.len {
            let glyph = self.get(i)?;
            if matches!(glyph.code, 0x302e | 0x302f) && glyph.advance != Fixed::ZERO {
                let mut first = i;
                while first > 0 && self.get(sub(first, 1)?)?.start == glyph.start {
                    first = sub(first, 1)?;
                }
                if first < i {
                    self.data.copy_within(first..i, add(first, 1)?);
                    self.put(first, glyph)?;
                }
            }
        }
        Ok(())
    }
    fn slot(&self, i: usize) -> Result<usize, FontError> {
        if i >= self.len {
            return Err(FontError::InvalidTable);
        }
        if i < self.gap {
            Ok(i)
        } else {
            add(i, sub(self.data.len(), self.len)?)
        }
    }
    fn get(&self, i: usize) -> Result<Glyph, FontError> {
        let at = self.slot(i)?;
        self.data.get(at).copied().ok_or(FontError::InvalidTable)
    }
    fn put(&mut self, i: usize, glyph: Glyph) -> Result<(), FontError> {
        let at = self.slot(i)?;
        *self.data.get_mut(at).ok_or(FontError::InvalidTable)? = glyph;
        Ok(())
    }
    /// Move the gap before logical glyph `i`; copies `|gap - i|` glyphs.
    fn move_gap(&mut self, i: usize) -> Result<(), FontError> {
        if i > self.len {
            return Err(FontError::InvalidTable);
        }
        let width = sub(self.data.len(), self.len)?;
        if i < self.gap {
            self.data.copy_within(i..self.gap, add(i, width)?);
        } else {
            self.data
                .copy_within(add(self.gap, width)?..add(i, width)?, self.gap);
        }
        #[cfg(test)]
        {
            self.moved = add(self.moved, self.gap.abs_diff(i))?;
        }
        self.gap = i;
        Ok(())
    }
    /// Make the glyphs contiguous again.
    fn close(&mut self) -> Result<(), FontError> {
        self.move_gap(self.len)
    }
    fn insert(&mut self, i: usize, glyph: Glyph) -> Result<(), FontError> {
        if self.len == self.data.len() {
            return Err(FontError::BufferTooSmall);
        }
        self.move_gap(i)?;
        *self.data.get_mut(i).ok_or(FontError::BufferTooSmall)? = glyph;
        self.gap = add(i, 1)?;
        self.len = add(self.len, 1)?;
        self.inserted = add(self.inserted, 1)?;
        Ok(())
    }
    fn remove(&mut self, i: usize) -> Result<(), FontError> {
        self.get(i)?;
        self.move_gap(i)?;
        self.len = sub(self.len, 1)?;
        Ok(())
    }
}

/// One ordered feature stage. Alternate substitutions use the one-based value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Feature {
    /// OpenType feature tag.
    pub tag: [u8; 4],
    /// One-based alternate selector; zero disables the stage.
    pub value: u16,
}

/// Operation counter shared by the `LayoutTable::apply` calls of one layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    pub(crate) spent: usize,
}
impl Budget {
    /// Operations all applications drawing on one budget may spend together.
    pub const LIMIT: usize = 64_000_000;
    /// Operations one application may spend, before `PER_GLYPH`.
    pub const PER_APPLICATION: usize = 1_000_000;
    /// Operations one application may spend per glyph of its input.
    pub const PER_GLYPH: usize = 64;
    /// Operations spent so far.
    #[must_use]
    pub const fn spent(self) -> usize {
        self.spent
    }
}

/// A borrowed GSUB or GPOS table.
#[derive(Clone, Copy, Debug)]
pub struct LayoutTable<'a> {
    table: Table<'a>,
    substitution: bool,
    count: usize,
}
impl<'a> LayoutTable<'a> {
    /// Read the header and lookup directory; payloads are checked when selected.
    /// # Errors
    /// Rejects unsupported versions, truncation, or more than 4096 lookups.
    pub fn parse(data: &'a [u8], substitution: bool) -> Result<Self, FontError> {
        let table = Table(data);
        if table.word(0)? != 1 || table.word(2)? > 1 {
            return Err(FontError::UnsupportedFormat);
        }
        table.child(4)?.u(0)?;
        table.child(6)?.u(0)?;
        if table.word(2)? == 1 {
            table.long(10)?;
        }
        let list = table.child(8)?;
        let count = list.u(0)?;
        if count > 4096 {
            return Err(FontError::LimitExceeded);
        }
        list.array(2, count, 2)?;
        Ok(Self {
            table,
            substitution,
            count,
        })
    }
    /// Whether the script explicitly supplies the requested language system.
    /// # Errors
    /// Returns malformed table errors.
    pub fn supports_language(self, script: [u8; 4], language: [u8; 4]) -> Result<bool, FontError> {
        let Some(s) = self.table.child(4)?.tagged(script)? else {
            return Ok(false);
        };
        Ok(s.tagged_at(language, 2)?.is_some())
    }
    /// Apply one lookup, useful for explicit shaping plans and conformance vectors.
    /// # Errors
    /// Returns malformed data, depth/work limits, or glyph-capacity errors.
    pub fn apply_lookup(
        self,
        index: usize,
        gdef: Gdef<'a>,
        coords: &[Fixed],
        rtl: bool,
        buffer: &mut Buffer<'_>,
    ) -> Result<(), FontError> {
        let mut budget = Budget::default();
        Engine::new(self, gdef, coords, rtl, buffer, &mut budget)?.run(index, buffer)
    }
}

struct Engine<'a, 'c> {
    layout: LayoutTable<'a>,
    gdef: Gdef<'a>,
    coords: &'c [Fixed],
    rtl: bool,
    work: usize,
    limit: usize,
    budget: &'c mut Budget,
    active: [usize; 16],
    depth: usize,
    feature: Feature,
    attach: [Option<position::Attach>; 16],
}
impl<'a, 'c> Engine<'a, 'c> {
    fn new(
        layout: LayoutTable<'a>,
        gdef: Gdef<'a>,
        coords: &'c [Fixed],
        rtl: bool,
        buffer: &Buffer<'_>,
        budget: &'c mut Budget,
    ) -> Result<Self, FontError> {
        Ok(Self {
            layout,
            gdef,
            coords,
            rtl,
            work: 0,
            limit: add(Budget::PER_APPLICATION, mul(Budget::PER_GLYPH, buffer.len)?)?,
            budget,
            active: [usize::MAX; 16],
            depth: 0,
            feature: Feature {
                tag: [0; 4],
                value: 1,
            },
            attach: [None; 16],
        })
    }
    fn tick(&mut self) -> Result<(), FontError> {
        self.work = add(self.work, 1)?;
        self.budget.spent = add(self.budget.spent, 1)?;
        if self.work > self.limit || self.budget.spent > Budget::LIMIT {
            Err(FontError::LimitExceeded)
        } else {
            Ok(())
        }
    }
    fn lookup(&self, index: usize) -> Result<Table<'a>, FontError> {
        if index >= self.layout.count {
            return Err(FontError::InvalidTable);
        }
        self.layout.table.child(8)?.child(add(2, mul(index, 2)?)?)
    }
    fn eligible(
        &self,
        buffer: &Buffer<'_>,
        i: usize,
        lookup: Table<'_>,
    ) -> Result<bool, FontError> {
        let g = buffer.get(i)?;
        if g.hidden && (!self.layout.substitution || g.code != 0x200c) {
            return Ok(false);
        }
        let flags = lookup.word(2)?;
        if flags & 0xe0 != 0 {
            return Err(FontError::InvalidTable);
        }
        let set = if flags & 16 != 0 {
            lookup.u(add(6, mul(lookup.u(4)?, 2)?)?)?
        } else {
            0
        };
        Ok(!self.gdef.ignored(g.id, g.class, flags, set)?)
    }
    fn next(
        &mut self,
        buffer: &Buffer<'_>,
        i: usize,
        lookup: Table<'_>,
        back: bool,
    ) -> Result<Option<usize>, FontError> {
        let mut at = i;
        loop {
            self.tick()?;
            if back {
                let Some(n) = at.checked_sub(1) else {
                    return Ok(None);
                };
                at = n;
            } else {
                at = add(at, 1)?;
                if at >= buffer.len {
                    return Ok(None);
                }
            }
            if self.eligible(buffer, at, lookup)? {
                return Ok(Some(at));
            }
        }
    }
    const fn enabled(&self, glyph: Glyph) -> bool {
        match &self.feature.tag {
            b"rtla" => self.rtl,
            b"rtlm" => self.rtl && glyph.mirror,
            b"isol" => glyph.form & 1 != 0,
            b"init" => glyph.form & 2 != 0,
            b"medi" => glyph.form & 4 != 0,
            b"fina" => glyph.form & 8 != 0,
            b"ljmo" => glyph.jamo & 1 != 0,
            b"vjmo" => glyph.jamo & 2 != 0,
            b"tjmo" => glyph.jamo & 4 != 0,
            _ => true,
        }
    }
    fn run(&mut self, index: usize, buffer: &mut Buffer<'_>) -> Result<(), FontError> {
        let result = self.pass(index, buffer);
        let closed = buffer.close();
        result.and(closed)
    }
    fn pass(&mut self, index: usize, buffer: &mut Buffer<'_>) -> Result<(), FontError> {
        for (order, g) in buffer.glyphs_mut().iter_mut().enumerate() {
            g.context = 0;
            g.source_order = order;
        }
        buffer.set_classes(self.gdef)?;
        self.attach = [None; 16];
        let lookup = self.lookup(index)?;
        let kind = lookup.u(0)?;
        let reverse = self.layout.substitution
            && (kind == 8 || (kind == 7 && lookup.u(4)? != 0 && lookup.child(6)?.u(2)? == 8));
        if reverse {
            for i in (0..buffer.len).rev() {
                self.tick()?;
                if self.enabled(buffer.get(i)?) {
                    self.apply(index, buffer, i)?;
                }
            }
        } else {
            let mut i = 0;
            while i < buffer.len {
                self.tick()?;
                let next = if self.enabled(buffer.get(i)?) {
                    self.apply(index, buffer, i)?
                } else {
                    None
                };
                i = next.unwrap_or(add(i, 1)?);
            }
        }
        Ok(())
    }
    fn apply(
        &mut self,
        index: usize,
        buffer: &mut Buffer<'_>,
        i: usize,
    ) -> Result<Option<usize>, FontError> {
        if self.depth >= self.active.len() {
            return Err(FontError::LimitExceeded);
        }
        if self
            .active
            .get(..self.depth)
            .ok_or(FontError::LimitExceeded)?
            .contains(&index)
        {
            return Err(FontError::Cycle);
        }
        let lookup = self.lookup(index)?;
        if !self.eligible(buffer, i, lookup)? {
            return Ok(None);
        }
        *self
            .active
            .get_mut(self.depth)
            .ok_or(FontError::LimitExceeded)? = index;
        self.depth = add(self.depth, 1)?;
        let result = self.subtables(buffer, i, lookup);
        self.depth = sub(self.depth, 1)?;
        result
    }
    fn replace(
        &self,
        buffer: &mut Buffer<'_>,
        index: usize,
        mut glyph: Glyph,
    ) -> Result<(), FontError> {
        let class = self.gdef.class(glyph.id)?;
        if class != 0 {
            glyph.class = class;
        }
        buffer.put(index, glyph)
    }
    fn subtables(
        &mut self,
        buffer: &mut Buffer<'_>,
        i: usize,
        lookup: Table<'a>,
    ) -> Result<Option<usize>, FontError> {
        let kind = lookup.u(0)?;
        let n = lookup.u(4)?;
        lookup.array(6, n, 2)?;
        for j in 0..n {
            self.tick()?;
            let mut table = lookup.child(add(6, mul(j, 2)?)?)?;
            let mut k = kind;
            let extension = if self.layout.substitution { 7 } else { 9 };
            if k == extension {
                if table.u(0)? != 1 {
                    return Err(FontError::UnsupportedFormat);
                }
                k = table.u(2)?;
                if k == extension {
                    return Err(FontError::Cycle);
                }
                table = table.child32(4)?;
            }
            let result = if self.layout.substitution {
                self.substitute(buffer, i, lookup, k, table)?
            } else {
                self.position(buffer, i, lookup, k, table)?
            };
            if result.is_some() {
                return Ok(result);
            }
        }
        Ok(None)
    }
}
