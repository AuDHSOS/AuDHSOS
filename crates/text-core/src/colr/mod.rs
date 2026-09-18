// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Colour glyphs: COLR version 0 and version 1 over CPAL palettes.
//!
//! `Colr::paint` resolves the paint graph of one glyph into a flat stream of
//! [`PaintOp`] in font units; the rasterizer turns that stream into pixels and
//! reads no font table of its own. `docs/microsoft/colr.html:1130`.

mod cpal;
mod paint;
mod trig;

pub use cpal::{Background, Color, ColorSource, Cpal, FOREGROUND, MAX_PALETTES, PaletteUse};
pub use paint::{
    Affine, ColorLine, ColorStop, CompositeMode, Extend, Fill, MAX_DEPTH, MAX_PAINTS, PaintOp,
};

use crate::{
    Fixed, Font, FontError, OutlineKind,
    cff::{Cff, Command},
    glyf::{Glyf, Point},
    read::{self, add, mul},
    variation::{Axes, DeltaMap, ItemStore, VariationPoint},
};
use paint::Walk;

/// Maximum base glyph records in either version's list.
pub const MAX_BASE_GLYPHS: usize = 65_536;
/// Maximum entries in the `LayerList` and in the version 0 layer array.
pub const MAX_LAYERS: usize = 1_048_576;
/// Maximum Clip records in the `ClipList`.
pub const MAX_CLIPS: usize = 65_536;

/// A box in font units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extents {
    /// Smallest x.
    pub x_min: Fixed,
    /// Smallest y.
    pub y_min: Fixed,
    /// Largest x.
    pub x_max: Fixed,
    /// Largest y.
    pub y_max: Fixed,
}

impl Extents {
    const fn of(x: Fixed, y: Fixed) -> Self {
        Self {
            x_min: x,
            y_min: y,
            x_max: x,
            y_max: y,
        }
    }

    fn cover(self, x: Fixed, y: Fixed) -> Self {
        Self {
            x_min: self.x_min.min(x),
            y_min: self.y_min.min(y),
            x_max: self.x_max.max(x),
            y_max: self.y_max.max(y),
        }
    }

    fn union(self, other: Self) -> Self {
        self.cover(other.x_min, other.y_min)
            .cover(other.x_max, other.y_max)
    }

    /// The box of the four transformed corners.
    fn under(self, transform: Affine) -> Result<Self, FontError> {
        let (x, y) = transform.apply(self.x_min, self.y_min)?;
        let mut result = Self::of(x, y);
        for (x, y) in [
            (self.x_min, self.y_max),
            (self.x_max, self.y_min),
            (self.x_max, self.y_max),
        ] {
            let (x, y) = transform.apply(x, y)?;
            result = result.cover(x, y);
        }
        Ok(result)
    }
}

/// Caller-owned scratch for computing ink extents from glyph outlines.
pub struct Scratch<'a> {
    /// TrueType control points.
    pub points: &'a mut [Point],
    /// TrueType contour ends.
    pub contours: &'a mut [usize],
    /// gvar decoding scratch.
    pub variation: &'a mut [VariationPoint],
    /// CFF path commands.
    pub commands: &'a mut [Command],
}

/// What one resolved colour glyph wrote and what the font precomputed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Painted {
    /// Operations written to the caller's slice.
    pub ops: usize,
    /// Colour stops written to the caller's slice.
    pub stops: usize,
    /// The `ClipList` box of this glyph, when the font has one.
    pub clip: Option<Extents>,
    /// Whether the paint graph paints inside a finite box, by the rules of
    /// each format. An unbounded glyph must not be drawn, and `clip` is then
    /// the only box there is.
    pub bounded: bool,
}

/// A validated COLR table and the CPAL table it references.
#[derive(Clone, Copy, Debug)]
pub struct Colr<'a> {
    data: &'a [u8],
    cpal: Cpal<'a>,
    base_list: Option<usize>,
    layer_list: Option<usize>,
    clip_list: Option<usize>,
    bases: &'a [u8],
    layers: &'a [u8],
    store: Option<ItemStore<'a>>,
    map: Option<DeltaMap<'a>>,
    glyphs: u16,
}

impl<'a> Colr<'a> {
    /// Validate the COLR and CPAL tables of a face.
    /// # Errors
    /// Returns `MissingTable` without both tables, and the validation errors
    /// of [`Colr::parse_tables`].
    pub fn parse(font: &Font<'a>) -> Result<Self, FontError> {
        let axes = match font.table(*b"fvar") {
            Some(fvar) => Axes::parse(fvar.data, font.table(*b"avar").map(|t| t.data))?.len(),
            None => 0,
        };
        Self::parse_tables(
            font.required(*b"COLR")?,
            font.required(*b"CPAL")?,
            font.metrics()?.maxp.glyph_count,
            axes,
        )
    }

    /// Validate a borrowed COLR and CPAL pair against a face's glyph count.
    ///
    /// Every record array, clip range and palette range is checked here; each
    /// paint table is checked when [`Colr::paint`] reaches it, under the
    /// depth and visit limits of that traversal.
    /// # Errors
    /// Rejects versions above one, unsorted or duplicate base glyph records,
    /// glyph identifiers outside the face, overlapping clip ranges, counts
    /// above the limits of this module, and ranges outside either table.
    pub fn parse_tables(
        colr: &'a [u8],
        cpal: &'a [u8],
        glyphs: u16,
        axes: usize,
    ) -> Result<Self, FontError> {
        let version = read::u16(colr, 0)?;
        if version > 1 {
            return Err(FontError::UnsupportedFormat);
        }
        let mut result = Self {
            data: colr,
            cpal: Cpal::parse(cpal)?,
            base_list: None,
            layer_list: None,
            clip_list: None,
            bases: records(colr, 2, 4, 6)?,
            layers: records(colr, 12, 8, 4)?,
            store: None,
            map: None,
            glyphs,
        };
        result.check_version_zero()?;
        if version == 1 {
            result.base_list = result.base_glyph_list()?;
            result.layer_list = optional(colr, 18)?;
            result.clip_list = optional(colr, 22)?;
            if let Some(at) = optional(colr, 30)? {
                result.store = Some(ItemStore::parse(read::tail(colr, at)?, axes)?);
                result.map = optional(colr, 26)?
                    .map(|at| DeltaMap::parse(read::tail(colr, at)?))
                    .transpose()?;
            }
            result.check_layer_list()?;
            result.check_clip_list()?;
        }
        Ok(result)
    }

    /// The palettes this table's colours come from.
    #[must_use]
    pub const fn palettes(&self) -> Cpal<'a> {
        self.cpal
    }

    /// Whether this face defines a colour glyph for `glyph`.
    /// # Errors
    /// Rejects a malformed record array.
    pub fn covers(&self, glyph: u16) -> Result<bool, FontError> {
        Ok(self.root(glyph)?.is_some() || self.version_zero_record(glyph)?.is_some())
    }

    /// Resolve the colour glyph of `glyph` into operations and colour stops.
    ///
    /// Version 1 definitions take precedence over version 0 ones, as the
    /// specification requires; a version 0 definition resolves to the same
    /// stream a clipped solid fill per layer would produce. `coords` are
    /// normalized variation coordinates, empty for a face without axes.
    /// # Errors
    /// Returns `MissingTable` for a glyph this face gives no colour glyph,
    /// `Cycle` for a paint table that is its own ancestor, `LimitExceeded`
    /// past [`MAX_DEPTH`] or [`MAX_PAINTS`], `BufferTooSmall` for caller
    /// storage, and the table errors of any malformed paint table.
    pub fn paint(
        &self,
        glyph: u16,
        palette: u16,
        coords: &[Fixed],
        ops: &mut [PaintOp],
        stops: &mut [ColorStop],
    ) -> Result<Painted, FontError> {
        let mut walk = Walk {
            colr: self,
            palette,
            coords,
            ops,
            stops,
            op_count: 0,
            stop_count: 0,
            visits: 0,
            path: [0; MAX_DEPTH],
        };
        let bounded = if let Some(root) = self.root(glyph)? {
            walk.paint(root, 0, Affine::IDENTITY)?
        } else {
            self.version_zero(glyph, &mut walk)?;
            true
        };
        Ok(Painted {
            ops: walk.op_count,
            stops: walk.stop_count,
            clip: self.clip_box(glyph, coords)?,
            bounded,
        })
    }

    /// The precomputed clip box of `glyph`, when the `ClipList` carries one.
    /// # Errors
    /// Rejects a malformed `ClipList` or `ClipBox`.
    pub fn clip_box(&self, glyph: u16, coords: &[Fixed]) -> Result<Option<Extents>, FontError> {
        let Some(list) = self.clip_list else {
            return Ok(None);
        };
        let data = read::tail(self.data, list)?;
        let count = read::offset(read::u32(data, 1)?)?;
        let index = read::lower_bound(count, |index| {
            Ok(read::u16(data, add(5, mul(index, 7)?)?)? <= glyph)
        })?;
        let Some(record) = index.checked_sub(1) else {
            return Ok(None);
        };
        let record = read::bytes(data, add(5, mul(record, 7)?)?, 7)?;
        if read::u16(record, 2)? < glyph {
            return Ok(None);
        }
        let at = add(list, read::offset(read::u24(record, 4)?)?)?;
        let clip = read::tail(self.data, at)?;
        let variable = match read::u8(clip, 0)? {
            1 => false,
            2 => true,
            _ => return Err(FontError::UnsupportedFormat),
        };
        let walk = Walk {
            colr: self,
            palette: 0,
            coords,
            ops: &mut [],
            stops: &mut [],
            op_count: 0,
            stop_count: 0,
            visits: 0,
            path: [0; MAX_DEPTH],
        };
        Ok(Some(walk.clip_extents(clip, variable)?))
    }

    /// Ink extents in font units: the union of the outermost clipped outlines.
    ///
    /// `None` reports a stream no outline clips at all. `Painted::bounded`
    /// carries the specification's verdict; the box of a stream that verdict
    /// calls unbounded covers its clipped part only. Curved segments
    /// contribute their control points, so a box can exceed the ink.
    /// # Errors
    /// Rejects an unmatched clip, malformed outlines, and small scratch.
    pub fn extents(
        &self,
        font: &Font<'_>,
        coords: &[Fixed],
        ops: &[PaintOp],
        scratch: &mut Scratch<'_>,
    ) -> Result<Option<Extents>, FontError> {
        let mut depth = 0usize;
        let mut result = None;
        for op in ops {
            match *op {
                PaintOp::Clip { glyph, transform } => {
                    if depth == 0 {
                        let box_ = self
                            .outline(font, glyph, coords, scratch)?
                            .under(transform)?;
                        result = Some(result.map_or(box_, |have: Extents| have.union(box_)));
                    }
                    depth = add(depth, 1)?;
                }
                PaintOp::Unclip => depth = depth.checked_sub(1).ok_or(FontError::InvalidTable)?,
                _ => (),
            }
        }
        Ok(result)
    }

    #[expect(clippy::unused_self, reason = "an outline reader of this table")]
    fn outline(
        &self,
        font: &Font<'_>,
        glyph: u16,
        coords: &[Fixed],
        scratch: &mut Scratch<'_>,
    ) -> Result<Extents, FontError> {
        match font.outline_kind() {
            OutlineKind::TrueType => {
                let outline = Glyf::parse(font)?.outline_instance(
                    glyph,
                    coords,
                    scratch.points,
                    scratch.contours,
                    scratch.variation,
                )?;
                let points = scratch
                    .points
                    .get(..outline.points)
                    .ok_or(FontError::BufferTooSmall)?;
                let Some(first) = points.first() else {
                    return Ok(Extents::default());
                };
                Ok(points
                    .iter()
                    .fold(Extents::of(first.x, first.y), |box_, point| {
                        box_.cover(point.x, point.y)
                    }))
            }
            OutlineKind::PostScript => {
                let count = Cff::parse(font)?.outline_instance(glyph, coords, scratch.commands)?;
                let commands = scratch
                    .commands
                    .get(..count)
                    .ok_or(FontError::BufferTooSmall)?;
                let mut box_ = None;
                for command in commands {
                    for point in match *command {
                        Command::Move(p) | Command::Line(p) => [Some(p), None, None],
                        Command::Curve(a, b, c) => [Some(a), Some(b), Some(c)],
                        Command::Close => [None; 3],
                    }
                    .into_iter()
                    .flatten()
                    {
                        box_ = Some(box_.map_or_else(
                            || Extents::of(point.x, point.y),
                            |have: Extents| have.cover(point.x, point.y),
                        ));
                    }
                }
                Ok(box_.unwrap_or_default())
            }
        }
    }

    const fn glyph(&self, id: u16) -> Result<u16, FontError> {
        if id < self.glyphs {
            Ok(id)
        } else {
            Err(FontError::GlyphIndex)
        }
    }

    /// The absolute offset of the version 1 root paint table of `glyph`.
    fn root(&self, glyph: u16) -> Result<Option<usize>, FontError> {
        let Some(list) = self.base_list else {
            return Ok(None);
        };
        let data = read::tail(self.data, list)?;
        let count = read::offset(read::u32(data, 0)?)?;
        let index = read::lower_bound(count, |index| {
            Ok(read::u16(data, add(4, mul(index, 6)?)?)? < glyph)
        })?;
        if index >= count {
            return Ok(None);
        }
        let record = read::bytes(data, add(4, mul(index, 6)?)?, 6)?;
        if read::u16(record, 0)? != glyph {
            return Ok(None);
        }
        Ok(Some(add(list, read::offset(read::u32(record, 2)?)?)?))
    }

    fn version_zero_record(&self, glyph: u16) -> Result<Option<&'a [u8]>, FontError> {
        let count = self.bases.len().checked_div(6).ok_or(FontError::Overflow)?;
        let index = read::lower_bound(count, |index| {
            Ok(read::u16(self.bases, mul(index, 6)?)? < glyph)
        })?;
        if index >= count {
            return Ok(None);
        }
        let record = read::bytes(self.bases, mul(index, 6)?, 6)?;
        if read::u16(record, 0)? != glyph {
            return Ok(None);
        }
        Ok(Some(record))
    }

    /// Version 0 layers are the same stream: one clipped solid fill each.
    fn version_zero(&self, glyph: u16, walk: &mut Walk<'_, '_>) -> Result<(), FontError> {
        let record = self
            .version_zero_record(glyph)?
            .ok_or(FontError::MissingTable)?;
        let first = usize::from(read::u16(record, 2)?);
        for index in 0..usize::from(read::u16(record, 4)?) {
            let layer = read::bytes(self.layers, mul(add(first, index)?, 4)?, 4)?;
            walk.solid_layer(read::u16(layer, 0)?, read::u16(layer, 2)?)?;
        }
        Ok(())
    }

    fn check_version_zero(&self) -> Result<(), FontError> {
        let layers = self
            .layers
            .len()
            .checked_div(4)
            .ok_or(FontError::Overflow)?;
        let mut previous = None;
        for record in self.bases.as_chunks::<6>().0 {
            let glyph = self.glyph(read::u16(record, 0)?)?;
            if previous.is_some_and(|last| last >= glyph) {
                return Err(FontError::TableOrder);
            }
            previous = Some(glyph);
            let first = usize::from(read::u16(record, 2)?);
            if add(first, usize::from(read::u16(record, 4)?))? > layers {
                return Err(FontError::InvalidTable);
            }
        }
        for record in self.layers.as_chunks::<4>().0 {
            self.glyph(read::u16(record, 0)?)?;
            let entry = read::u16(record, 2)?;
            if entry != FOREGROUND && entry >= self.cpal.entry_count() {
                return Err(FontError::InvalidTable);
            }
        }
        Ok(())
    }

    fn base_glyph_list(&self) -> Result<Option<usize>, FontError> {
        let Some(at) = optional(self.data, 14)? else {
            return Ok(None);
        };
        let data = read::tail(self.data, at)?;
        let count = read::offset(read::u32(data, 0)?)?;
        if count > MAX_BASE_GLYPHS {
            return Err(FontError::LimitExceeded);
        }
        let records = read::bytes(data, 4, mul(count, 6)?)?;
        let mut previous = None;
        for record in records.as_chunks::<6>().0 {
            let glyph = self.glyph(read::u16(record, 0)?)?;
            if previous.is_some_and(|last| last >= glyph) {
                return Err(FontError::TableOrder);
            }
            previous = Some(glyph);
        }
        Ok(Some(at))
    }

    fn check_layer_list(&self) -> Result<(), FontError> {
        let Some(at) = self.layer_list else {
            return Ok(());
        };
        let data = read::tail(self.data, at)?;
        let count = read::offset(read::u32(data, 0)?)?;
        if count > MAX_LAYERS {
            return Err(FontError::LimitExceeded);
        }
        read::bytes(data, 4, mul(count, 4)?)?;
        Ok(())
    }

    fn check_clip_list(&self) -> Result<(), FontError> {
        let Some(at) = self.clip_list else {
            return Ok(());
        };
        let data = read::tail(self.data, at)?;
        if read::u8(data, 0)? != 1 {
            return Err(FontError::UnsupportedFormat);
        }
        let count = read::offset(read::u32(data, 1)?)?;
        if count > MAX_CLIPS {
            return Err(FontError::LimitExceeded);
        }
        let records = read::bytes(data, 5, mul(count, 7)?)?;
        let mut previous = None;
        for record in records.as_chunks::<7>().0 {
            let start = read::u16(record, 0)?;
            let end = self.glyph(read::u16(record, 2)?)?;
            if start > end || previous.is_some_and(|last| last >= start) {
                return Err(FontError::TableOrder);
            }
            previous = Some(end);
        }
        Ok(())
    }
}

/// One optional `Offset32` of the COLR header; zero means absent.
fn optional(data: &[u8], at: usize) -> Result<Option<usize>, FontError> {
    match read::offset(read::u32(data, at)?)? {
        0 => Ok(None),
        offset => Ok(Some(offset)),
    }
}

/// One version 0 record array: a count field, an offset field, and a stride.
fn records(
    data: &[u8],
    count_at: usize,
    offset_at: usize,
    stride: usize,
) -> Result<&[u8], FontError> {
    let count = usize::from(read::u16(data, count_at)?);
    let at = read::offset(read::u32(data, offset_at)?)?;
    if count == 0 {
        return Ok(&[]);
    }
    if at == 0 {
        return Err(FontError::InvalidTable);
    }
    if count > MAX_LAYERS {
        return Err(FontError::LimitExceeded);
    }
    read::bytes(data, at, mul(count, stride)?)
}
