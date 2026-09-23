// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Budget, Buffer, Engine, Feature, Gdef, LayoutTable, table::Table};
use crate::{
    Fixed, FontError,
    read::{add, mul},
};

impl<'a> LayoutTable<'a> {
    fn language(self, script: [u8; 4], language: [u8; 4]) -> Result<Option<Table<'a>>, FontError> {
        let list = self.table.child(4)?;
        let script = match list.tagged(script)? {
            Some(s) => Some(s),
            None => list.tagged(*b"DFLT")?,
        };
        let Some(s) = script else {
            return Ok(None);
        };
        if let Some(lookup) = s.tagged_at(language, 2)? {
            return Ok(Some(lookup));
        }
        if s.u(0)? == 0 {
            Ok(None)
        } else {
            Ok(Some(s.child(0)?))
        }
    }
    fn variations(self, coords: &[Fixed]) -> Result<Option<Table<'a>>, FontError> {
        if self.table.u(2)? == 0 || self.table.long(10)? == 0 {
            return Ok(None);
        }
        let v = self.table.child32(10)?;
        if v.u(0)? != 1 || v.u(2)? != 0 {
            return Err(FontError::UnsupportedFormat);
        }
        let n = v.long(4)?;
        if n > 4096 {
            return Err(FontError::LimitExceeded);
        }
        v.array(8, n, 8)?;
        for i in 0..n {
            let at = add(8, mul(i, 8)?)?;
            let mut matched = true;
            if v.long(at)? != 0 {
                let set = v.child32(at)?;
                let count = set.u(0)?;
                if count > 64 {
                    return Err(FontError::LimitExceeded);
                }
                set.array(2, count, 4)?;
                for j in 0..count {
                    let c = set.child32(add(2, mul(j, 4)?)?)?;
                    if c.u(0)? != 1 {
                        matched = false;
                        break;
                    }
                    let Some(coord) = coords.get(c.u(2)?) else {
                        matched = false;
                        break;
                    };
                    let lo = Fixed::from_2_14(c.signed(4)?);
                    let hi = Fixed::from_2_14(c.signed(6)?);
                    if *coord < lo || *coord > hi {
                        matched = false;
                        break;
                    }
                }
            }
            if matched {
                if v.long(add(at, 4)?)? == 0 {
                    return Ok(None);
                }
                let s = v.child32(add(at, 4)?)?;
                if s.u(0)? == 1 && s.u(2)? == 0 {
                    s.array(6, s.u(4)?, 6)?;
                    return Ok(Some(s));
                }
            }
        }
        Ok(None)
    }
    fn feature(
        self,
        index: usize,
        alternate: Option<Table<'a>>,
        engine: &mut Engine<'_, '_>,
    ) -> Result<Table<'a>, FontError> {
        let list = self.table.child(6)?;
        if index >= list.u(0)? {
            return Err(FontError::InvalidTable);
        }
        if let Some(a) = alternate {
            for i in 0..a.u(4)? {
                engine.tick()?;
                let at = add(6, mul(i, 6)?)?;
                match a.u(at)?.cmp(&index) {
                    core::cmp::Ordering::Equal => return a.child32(add(at, 2)?),
                    core::cmp::Ordering::Greater => break,
                    core::cmp::Ordering::Less => {}
                }
            }
        }
        list.child(add(6, mul(index, 6)?)?)
    }
    /// Apply required features, then explicit stages in order; absent stages do nothing.
    /// Operations count against `Budget::PER_APPLICATION` plus `Budget::PER_GLYPH`
    /// per glyph, and against `budget`.
    /// # Errors
    /// Returns invalid references, bounded-work errors, or insufficient glyph capacity.
    #[expect(
        clippy::too_many_arguments,
        reason = "explicit sans-I/O shaping context"
    )]
    pub fn apply(
        self,
        script: [u8; 4],
        language: [u8; 4],
        features: &[Feature],
        gdef: Gdef<'a>,
        coords: &[Fixed],
        rtl: bool,
        buffer: &mut Buffer<'_>,
        budget: &mut Budget,
    ) -> Result<(), FontError> {
        let Some(lang) = self.language(script, language)? else {
            return Ok(());
        };
        if lang.u(0)? != 0 {
            return Err(FontError::InvalidTable);
        }
        let mut engine = Engine::new(self, gdef, coords, rtl, buffer, budget)?;
        self.stages(lang, features, &mut engine, buffer)
    }
    fn stages(
        self,
        lang: Table<'a>,
        features: &[Feature],
        engine: &mut Engine<'a, '_>,
        buffer: &mut Buffer<'_>,
    ) -> Result<(), FontError> {
        let count = lang.u(4)?;
        lang.array(6, count, 2)?;
        let required = lang.u(2)?;
        let alternate = self.variations(engine.coords)?;
        if required != 0xffff {
            let table = self.feature(required, alternate, engine)?;
            engine.stage(table, buffer)?;
        }
        for feature in features {
            if feature.value == 0 {
                continue;
            }
            engine.feature = *feature;
            let list = self.table.child(6)?;
            let mut selected = Selection::new();
            for i in 0..count {
                engine.tick()?;
                let index = lang.u(add(6, mul(i, 2)?)?)?;
                if index >= list.u(0)? {
                    return Err(FontError::InvalidTable);
                }
                if index != required && list.tag(add(2, mul(index, 6)?)?)? == feature.tag {
                    let table = self.feature(index, alternate, engine)?;
                    selected.add(table, self.count, engine)?;
                }
            }
            selected.run(engine, buffer)?;
        }
        Ok(())
    }
}

/// Lookup indices selected for one stage, one bit per index below 4096.
struct Selection([u64; 64]);
impl Selection {
    const fn new() -> Self {
        Self([0; 64])
    }
    fn add(
        &mut self,
        feature: Table<'_>,
        count: usize,
        engine: &mut Engine<'_, '_>,
    ) -> Result<(), FontError> {
        let n = feature.u(2)?;
        feature.array(4, n, 2)?;
        for i in 0..n {
            engine.tick()?;
            let index = feature.u(add(4, mul(i, 2)?)?)?;
            if index >= count {
                return Err(FontError::InvalidTable);
            }
            *self.0.get_mut(index / 64).ok_or(FontError::LimitExceeded)? |= 1 << (index % 64);
        }
        Ok(())
    }
    /// Run the selected lookups in lookup-list order.
    fn run(&self, engine: &mut Engine<'_, '_>, buffer: &mut Buffer<'_>) -> Result<(), FontError> {
        for (word, bits) in self.0.iter().enumerate() {
            let mut bits = *bits;
            while bits != 0 {
                let bit =
                    usize::try_from(bits.trailing_zeros()).map_err(|_| FontError::Overflow)?;
                engine.run(add(mul(word, 64)?, bit)?, buffer)?;
                bits &= bits.wrapping_sub(1);
            }
        }
        Ok(())
    }
}
impl Engine<'_, '_> {
    fn stage(&mut self, feature: Table<'_>, buffer: &mut Buffer<'_>) -> Result<(), FontError> {
        let mut selected = Selection::new();
        selected.add(feature, self.layout.count, self)?;
        selected.run(self, buffer)
    }
}
