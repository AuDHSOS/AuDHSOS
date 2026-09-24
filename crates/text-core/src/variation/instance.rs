// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{Axes, Hvar, Mvar};
use crate::{Fixed, Font, FontError, glyf::Outline, metrics::Metrics, read};

/// An immutable, explicitly normalized variation instance.
#[derive(Clone, Copy, Debug)]
pub struct Instance<'a, 'c> {
    font: Font<'a>,
    metrics: Metrics<'a>,
    coords: &'c [Fixed],
    hvar: Option<Hvar<'a>>,
    mvar: Option<Mvar<'a>>,
}
/// Validated metric tables of one face, independent of its coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tables<'a> {
    font: Font<'a>,
    metrics: Metrics<'a>,
    axes: usize,
    hvar: Option<Hvar<'a>>,
    mvar: Option<Mvar<'a>>,
}
fn axis_count(font: &Font<'_>) -> Result<usize, FontError> {
    Ok(font
        .table(*b"fvar")
        .map(|t| Axes::parse(t.data, font.table(*b"avar").map(|a| a.data)))
        .transpose()?
        .map_or(0, Axes::len))
}
fn check(coords: &[Fixed], axes: usize) -> Result<(), FontError> {
    if coords.len() != axes
        || coords
            .iter()
            .any(|v| *v < Fixed::from_i32(-1) || *v > Fixed::ONE)
    {
        return Err(FontError::InvalidTable);
    }
    Ok(())
}
impl<'a> Tables<'a> {
    /// Validate the face's metric and metric variation tables.
    /// # Errors
    /// Rejects malformed tables.
    pub(crate) fn parse(font: &Font<'a>) -> Result<Self, FontError> {
        Self::with_axes(font, axis_count(font)?)
    }
    fn with_axes(font: &Font<'a>, axes: usize) -> Result<Self, FontError> {
        Ok(Self {
            font: *font,
            metrics: font.metrics()?,
            axes,
            hvar: if axes == 0 {
                None
            } else {
                font.table(*b"HVAR")
                    .map(|t| Hvar::parse(t.data, axes))
                    .transpose()?
            },
            mvar: if axes == 0 {
                None
            } else {
                font.table(*b"MVAR")
                    .map(|t| Mvar::parse(t.data, axes))
                    .transpose()?
            },
        })
    }
    /// The validated metrics of the face.
    pub(crate) const fn metrics(self) -> Metrics<'a> {
        self.metrics
    }
    /// Bind coordinates in O(axes), without parsing a table again.
    /// # Errors
    /// Rejects wrong axis counts or out-of-range coordinates.
    pub(crate) fn instance<'c>(self, coords: &'c [Fixed]) -> Result<Instance<'a, 'c>, FontError> {
        check(coords, self.axes)?;
        Ok(self.bind(coords))
    }
    const fn bind<'c>(self, coords: &'c [Fixed]) -> Instance<'a, 'c> {
        Instance {
            font: self.font,
            metrics: self.metrics,
            coords,
            hvar: self.hvar,
            mvar: self.mvar,
        }
    }
}
impl<'a, 'c> Instance<'a, 'c> {
    /// Validate coordinates and the instance's metric variation tables.
    /// # Errors
    /// Rejects malformed tables, wrong axis counts, or out-of-range coordinates.
    pub fn new(font: &Font<'a>, coords: &'c [Fixed]) -> Result<Self, FontError> {
        let axes = axis_count(font)?;
        check(coords, axes)?;
        Ok(Tables::with_axes(font, axes)?.bind(coords))
    }
    /// Read an instance advance, applying HVAR or gvar phantom deltas once.
    ///
    /// Supply the matching instance outline when nondefault gvar metrics lack HVAR.
    /// # Errors
    /// Returns `MissingOutline` when required, or a glyph/variation/arithmetic error.
    pub fn advance(
        self,
        glyph: u16,
        size: Fixed,
        outline: Option<&Outline>,
    ) -> Result<Fixed, FontError> {
        let base = Fixed::from_i32(i32::from(self.metrics.horizontal(glyph)?.0));
        let value = if let Some(hvar) = self.hvar {
            base.checked_add(hvar.advance(glyph, self.coords)?)?
        } else if self.coords.iter().any(|v| *v != Fixed::ZERO)
            && self.font.table(*b"gvar").is_some()
        {
            let outline = outline.ok_or(FontError::MissingOutline)?;
            outline.phantoms[1].x.checked_sub(outline.phantoms[0].x)?
        } else {
            base
        };
        value.mul_div(size, i64::from(self.metrics.head.units_per_em))
    }
    /// Return scaled ascender, signed descender, and nonnegative line gap.
    /// # Errors
    /// Returns malformed metric variation or arithmetic errors.
    pub fn line_metrics(self, size: Fixed) -> Result<[Fixed; 3], FontError> {
        let line = self.metrics.line;
        let mut values = [
            Fixed::from_i32(i32::from(line.ascender)),
            Fixed::from_i32(i32::from(line.descender)),
            Fixed::from_i32(i32::from(line.gap)),
        ];
        if self
            .metrics
            .os2
            .is_some_and(|os| os.use_typo && os.typo.is_some())
        {
            values[2] = Fixed::from_i32(i32::from(read::i16(self.font.required(*b"OS/2")?, 72)?));
            if let Some(mvar) = self.mvar {
                for (value, tag) in values.iter_mut().zip([*b"hasc", *b"hdsc", *b"hlgp"]) {
                    *value = value.checked_add(mvar.delta(tag, self.coords)?)?;
                }
            }
        }
        values[2] = values[2].max(Fixed::ZERO);
        for value in &mut values {
            *value = value.mul_div(size, i64::from(self.metrics.head.units_per_em))?;
        }
        Ok(values)
    }
}
