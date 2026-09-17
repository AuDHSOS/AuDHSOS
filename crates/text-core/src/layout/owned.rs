// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{
    ClusterBox, LayoutBuffers, LayoutInfo, LayoutView, Line, MAX_SCALARS, PositionedGlyph,
    Workspace,
};
use crate::{
    Fixed, FontError, FontSet, TextError, TextStyle, bidi, glyf, resolve::Run, segment, shape,
    variation::VariationPoint,
};
use alloc::vec::Vec;

/// Owned output, allocated through fallible vector growth.
#[derive(Debug)]
pub struct Layout {
    info: LayoutInfo,
    runs: Vec<Run>,
    lines: Vec<Line>,
    glyphs: Vec<PositionedGlyph>,
    clusters: Vec<ClusterBox>,
}
impl Layout {
    /// Borrow all numeric output.
    #[must_use]
    pub fn view(&self) -> LayoutView<'_> {
        LayoutView {
            info: self.info,
            runs: &self.runs,
            lines: &self.lines,
            glyphs: &self.glyphs,
            clusters: &self.clusters,
        }
    }
    /// Return the advance box and output counts.
    #[must_use]
    pub const fn info(&self) -> LayoutInfo {
        self.info
    }
}
fn vector<T: Clone>(len: usize, value: T) -> Result<Vec<T>, TextError> {
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|_| TextError::Allocation)?;
    out.resize(len, value);
    Ok(out)
}
fn grow<T: Clone>(out: &mut Vec<T>, value: T) -> Result<(), TextError> {
    if out.len() >= 1_048_576 {
        return Err(TextError::LimitExceeded);
    }
    let target = out
        .len()
        .max(1)
        .checked_mul(2)
        .ok_or(TextError::Overflow)?
        .min(1_048_576);
    let extra = target.checked_sub(out.len()).ok_or(TextError::Overflow)?;
    out.try_reserve_exact(extra)
        .map_err(|_| TextError::Allocation)?;
    out.resize(target, value);
    Ok(())
}
struct Storage {
    bidi: Vec<bidi::Unit>,
    indices: Vec<usize>,
    levels: Vec<u8>,
    ranks: Vec<usize>,
    line_units: Vec<segment::LineUnit>,
    breaks: Vec<segment::LineBoundary>,
    runs: Vec<Run>,
    glyphs: Vec<shape::Glyph>,
    line_glyphs: Vec<PositionedGlyph>,
    line_clusters: Vec<ClusterBox>,
    points: Vec<glyf::Point>,
    contours: Vec<usize>,
    variation: Vec<VariationPoint>,
    carets: Vec<shape::Caret>,
}
impl Storage {
    fn new(n: usize) -> Result<Self, TextError> {
        if n > MAX_SCALARS {
            return Err(TextError::LimitExceeded);
        }
        let glyphs = n.checked_mul(2).ok_or(TextError::Overflow)?.max(16);
        Ok(Self {
            bidi: vector(n, bidi::Unit::default())?,
            indices: vector(n, 0)?,
            levels: vector(n, 0)?,
            ranks: vector(n, 0)?,
            line_units: vector(n, segment::LineUnit::default())?,
            breaks: vector(
                n.checked_add(1).ok_or(TextError::Overflow)?,
                segment::LineBoundary::default(),
            )?,
            runs: vector(n, Run::default())?,
            glyphs: vector(glyphs, shape::Glyph::default())?,
            line_glyphs: vector(glyphs, PositionedGlyph::default())?,
            line_clusters: vector(n, ClusterBox::default())?,
            points: vector(256, glyf::Point::default())?,
            contours: vector(64, 0)?,
            variation: vector(512, VariationPoint::default())?,
            carets: vector(16, shape::Caret::Coordinate(Fixed::ZERO))?,
        })
    }
    fn workspace(&mut self) -> Workspace<'_> {
        Workspace {
            bidi: &mut self.bidi,
            indices: &mut self.indices,
            levels: &mut self.levels,
            ranks: &mut self.ranks,
            line_units: &mut self.line_units,
            breaks: &mut self.breaks,
            runs: &mut self.runs,
            glyphs: &mut self.glyphs,
            line_glyphs: &mut self.line_glyphs,
            line_clusters: &mut self.line_clusters,
            points: &mut self.points,
            contours: &mut self.contours,
            variation: &mut self.variation,
            caret_values: &mut self.carets,
        }
    }
    fn grow(&mut self) -> Result<(), TextError> {
        grow(&mut self.glyphs, shape::Glyph::default())?;
        grow(&mut self.line_glyphs, PositionedGlyph::default())?;
        grow(&mut self.points, glyf::Point::default())?;
        grow(&mut self.contours, 0)?;
        grow(&mut self.variation, VariationPoint::default())?;
        grow(&mut self.carets, shape::Caret::Coordinate(Fixed::ZERO))
    }
}
const fn capacity(error: TextError) -> bool {
    matches!(
        error,
        TextError::BufferTooSmall | TextError::Font(FontError::BufferTooSmall)
    )
}
/// Allocate and lay out text using the same implementation as `layout_into`.
/// # Errors
/// Returns font, settings, allocation, capacity, numeric, and bounded-work errors.
pub fn layout(
    set: &FontSet<'_, '_>,
    style: &TextStyle,
    text: &str,
    max_width: Option<Fixed>,
) -> Result<Layout, TextError> {
    let n = text.chars().count();
    let mut storage = Storage::new(n)?;
    let mut lines = vector(
        n.checked_add(1).ok_or(TextError::Overflow)?,
        Line::default(),
    )?;
    let mut glyphs = vector(
        n.checked_mul(2).ok_or(TextError::Overflow)?.max(16),
        PositionedGlyph::default(),
    )?;
    let mut clusters = vector(n, ClusterBox::default())?;
    loop {
        let result = {
            let mut workspace = storage.workspace();
            let mut buffers = LayoutBuffers {
                lines: &mut lines,
                glyphs: &mut glyphs,
                clusters: &mut clusters,
            };
            super::layout_into(set, style, text, max_width, &mut workspace, &mut buffers)
                .map(|view| view.info)
        };
        match result {
            Ok(info) => {
                storage.runs.truncate(info.runs);
                lines.truncate(info.lines);
                glyphs.truncate(info.glyphs);
                clusters.truncate(info.clusters);
                return Ok(Layout {
                    info,
                    runs: storage.runs,
                    lines,
                    glyphs,
                    clusters,
                });
            }
            Err(error) if capacity(error) => {
                storage.grow()?;
                grow(&mut glyphs, PositionedGlyph::default())?;
            }
            Err(error) => return Err(error),
        }
    }
}
/// Allocate scratch and measure through the complete line/cluster geometry path.
/// # Errors
/// Returns font, settings, allocation, capacity, numeric, and bounded-work errors.
pub fn measure(
    set: &FontSet<'_, '_>,
    style: &TextStyle,
    text: &str,
    max_width: Option<Fixed>,
) -> Result<(Fixed, Fixed), TextError> {
    let mut storage = Storage::new(text.chars().count())?;
    loop {
        match super::measure_into(set, style, text, max_width, &mut storage.workspace()) {
            Ok(size) => return Ok(size),
            Err(error) if capacity(error) => storage.grow()?,
            Err(error) => return Err(error),
        }
    }
}
