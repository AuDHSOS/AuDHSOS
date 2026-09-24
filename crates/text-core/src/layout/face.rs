// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{
    Fixed, Font, TextError,
    cmap::Cmap,
    glyf::Glyf,
    shape::Gdef,
    variation::{Instance, Tables},
};

/// Faces one layout call keeps validated; more faces evict the oldest.
pub(super) const SLOTS: usize = 8;

/// Tables of one role-chain face, validated once per layout call.
#[derive(Clone, Copy, Debug)]
pub(super) struct Face<'f> {
    pub index: usize,
    pub font: Font<'f>,
    pub tables: Tables<'f>,
    /// Scaled ascender, descender and gap at the run coordinates.
    pub line: [Fixed; 3],
    pub units_per_em: i64,
    pub gdef: Gdef<'f>,
    pub cmap: Cmap<'f>,
    /// Parsed on first outline need.
    pub glyf: Option<Glyf<'f>>,
}
impl<'f> Face<'f> {
    /// Bind the run coordinates in O(axes).
    pub(super) fn instance<'c>(&self, coords: &'c [Fixed]) -> Result<Instance<'f, 'c>, TextError> {
        Ok(self.tables.instance(coords)?)
    }
}
/// Round-robin cache of validated faces, keyed by role-chain index.
#[derive(Debug)]
pub(super) struct Faces<'f> {
    slots: [Option<Face<'f>>; SLOTS],
    next: usize,
}
impl Default for Faces<'_> {
    fn default() -> Self {
        Self {
            slots: [None; SLOTS],
            next: 0,
        }
    }
}
impl<'f> Faces<'f> {
    /// Return face `index`; validate it on a miss.
    ///
    /// Every run of one face carries the same coordinates, so `coords` of any
    /// run of the face give its line metrics.
    pub(super) fn get(
        &mut self,
        chain: &[Font<'f>],
        index: usize,
        coords: &[Fixed],
        size: Fixed,
    ) -> Result<Face<'f>, TextError> {
        if let Some(face) = self.slots.iter().flatten().find(|f| f.index == index) {
            return Ok(*face);
        }
        #[cfg(test)]
        super::probe::VALIDATIONS.with(|v| v.set(v.get().saturating_add(1)));
        let font = chain.get(index).ok_or(TextError::NoFont)?;
        let tables = Tables::parse(font)?;
        let line = tables.instance(coords)?.line_metrics(size)?;
        let gdef = font
            .table(*b"GDEF")
            .map(|t| Gdef::parse(t.data, coords.len()))
            .transpose()?
            .unwrap_or_default();
        let face = Face {
            index,
            font: *font,
            tables,
            line,
            units_per_em: i64::from(tables.metrics().head.units_per_em),
            gdef,
            cmap: font.cmap()?,
            glyf: None,
        };
        let slot = self.slots.get_mut(self.next).ok_or(TextError::Overflow)?;
        *slot = Some(face);
        self.next = self
            .next
            .checked_add(1)
            .and_then(|n| n.checked_rem(SLOTS))
            .ok_or(TextError::Overflow)?;
        Ok(face)
    }
    /// Keep an outline table `face` parsed since `get`.
    pub(super) fn put(&mut self, face: &Face<'f>) {
        if let Some(slot) = self
            .slots
            .iter_mut()
            .flatten()
            .find(|f| f.index == face.index)
        {
            slot.glyf = face.glyf;
        }
    }
}
