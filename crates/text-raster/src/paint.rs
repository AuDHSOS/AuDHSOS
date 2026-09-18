// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R10, R11 and R12: the clip stack, the group stack, and the paint stream
//! that drives them.
//!
//! `Colr::paint` of `text-core` resolves a colour glyph's graph into a flat
//! list of operations (D-173), so this module meets no cycle and walks no
//! graph. It consumes the list in order.
//!
//! Invariants: an operation writes only inside the glyph's box, because every
//! plane of the two stacks is that box and the box is clipped to the
//! destination before anything is drawn; the stacks never exceed the caller's
//! slices, because a push that would is a typed error.

use text_core::{
    Fixed, Font,
    colr::{ColorStop, Fill, PaintOp, Painted},
};

use crate::{
    composite::composite,
    error::RasterError,
    fill::{Cell, fill},
    flatten::Edge,
    gamma::{Gamma, LINEAR_ONE},
    gradient::Gradient,
    outline::{OutlineScratch, outline_edges},
    pixel::{Paint, Pixel, scale},
    surface::{Format, Surface},
    transform::Affine,
};

/// A box of device pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bounds {
    /// Leftmost column.
    pub x: i32,
    /// Topmost row.
    pub y: i32,
    /// Columns.
    pub width: u32,
    /// Rows.
    pub height: u32,
}

impl Bounds {
    /// How many pixels this box holds.
    fn pixels(self) -> Result<usize, RasterError> {
        usize::try_from(self.width)
            .map_err(|_| RasterError::Overflow)?
            .checked_mul(usize::try_from(self.height).map_err(|_| RasterError::Overflow)?)
            .ok_or(RasterError::Overflow)
    }
}

/// Caller-owned storage one colour glyph is drawn through.
#[derive(Debug)]
pub struct PaintScratch<'a> {
    /// Edges of one clipping outline.
    pub edges: &'a mut [Edge],
    /// One row of accumulation cells, at least the box's width plus one.
    pub cells: &'a mut [Cell],
    /// One `A8` plane of the box per clip stack level.
    pub clips: &'a mut [u8],
    /// One `Rgba16` plane of the box per group stack level.
    pub groups: &'a mut [u8],
    /// What the outline decoders of `text-core` write into.
    pub outline: OutlineScratch<'a>,
}

/// Bytes one pixel of a group plane occupies.
const GROUP_PIXEL: usize = 8;

/// Draw one resolved colour glyph.
///
/// `bounds` is where the glyph's box lands in `destination`, `transform` maps
/// font units to those device pixels, and `painted` is what `Colr::paint`
/// reported. An unbounded glyph draws nothing, which `Painted::bounded`
/// decides and D-173 requires.
/// # Errors
/// Returns `Stream` for brackets that do not match, `BufferTooSmall` when a
/// stack outgrows its slice, and the errors of the outline decoders.
#[expect(
    clippy::too_many_arguments,
    reason = "every argument is a distinct input the caller owns; grouping \
              them would hide which of them the crate may keep"
)]
pub fn draw_color_glyph(
    destination: &mut Surface<'_>,
    bounds: Bounds,
    transform: Affine,
    font: &Font<'_>,
    coordinates: &[Fixed],
    ops: &[PaintOp],
    stops: &[ColorStop],
    painted: Painted,
    foreground: Paint,
    gamma: &Gamma,
    scratch: &mut PaintScratch<'_>,
) -> Result<(), RasterError> {
    if !painted.bounded {
        return Ok(());
    }
    let plane = bounds.pixels()?;
    if plane == 0 {
        return Ok(());
    }
    let mut state = State {
        bounds,
        plane,
        clip_depth: 0,
        group_depth: 0,
    };
    for op in ops {
        match *op {
            PaintOp::Clip {
                glyph,
                transform: own,
            } => {
                state.push_clip(font, coordinates, glyph, transform.compose(own)?, scratch)?;
            }
            PaintOp::Unclip => state.pop_clip()?,
            PaintOp::Group => state.push_group(scratch)?,
            PaintOp::Compose(mode) => {
                state.compose(mode, destination, gamma, scratch)?;
            }
            PaintOp::Fill {
                transform: own,
                fill,
            } => {
                state.paint(
                    destination,
                    transform.compose(own)?,
                    fill,
                    stops,
                    foreground,
                    gamma,
                    scratch,
                )?;
            }
        }
    }
    if state.clip_depth != 0 || state.group_depth != 0 {
        return Err(RasterError::Stream);
    }
    Ok(())
}

/// How deep the two stacks are and where the glyph's box lies.
struct State {
    bounds: Bounds,
    plane: usize,
    clip_depth: usize,
    group_depth: usize,
}

impl State {
    /// Intersect the clip region with one glyph's outline.
    fn push_clip(
        &mut self,
        font: &Font<'_>,
        coordinates: &[Fixed],
        glyph: u16,
        transform: Affine,
        scratch: &mut PaintScratch<'_>,
    ) -> Result<(), RasterError> {
        let start = self
            .clip_depth
            .checked_mul(self.plane)
            .ok_or(RasterError::Overflow)?;
        let end = start.checked_add(self.plane).ok_or(RasterError::Overflow)?;
        if scratch.clips.get(start..end).is_none() {
            return Err(RasterError::BufferTooSmall);
        }
        // The outline is stated against the glyph's origin and the plane
        // against the box, so the transform moves by the box's corner.
        let placed = Affine {
            dx: transform.dx.checked_sub(Fixed::from_i32(self.bounds.x))?,
            dy: transform.dy.checked_sub(Fixed::from_i32(self.bounds.y))?,
            ..transform
        };
        let count = outline_edges(
            font,
            glyph,
            coordinates,
            placed,
            &mut scratch.outline,
            scratch.edges,
        )?;
        let edges = scratch
            .edges
            .get_mut(..count)
            .ok_or(RasterError::BufferTooSmall)?;
        let bytes = scratch
            .clips
            .get_mut(start..end)
            .ok_or(RasterError::BufferTooSmall)?;
        let mut mask = Surface::new(
            bytes,
            self.bounds.width,
            self.bounds.height,
            self.bounds.width,
            Format::A8,
        )?;
        fill(edges, &mut mask, scratch.cells)?;
        // Narrow by the level above, so a nested clip is the product of the
        // two coverages.
        if self.clip_depth > 0 {
            let parent = start.checked_sub(self.plane).ok_or(RasterError::Overflow)?;
            for index in 0..self.plane {
                let above = *scratch
                    .clips
                    .get(parent.checked_add(index).ok_or(RasterError::Overflow)?)
                    .ok_or(RasterError::Overflow)?;
                let here = scratch
                    .clips
                    .get_mut(start.checked_add(index).ok_or(RasterError::Overflow)?)
                    .ok_or(RasterError::Overflow)?;
                *here = narrow(scale(expand(*here), expand(above)));
            }
        }
        self.clip_depth = self
            .clip_depth
            .checked_add(1)
            .ok_or(RasterError::Overflow)?;
        Ok(())
    }

    /// Restore the clip region the matching `Clip` narrowed.
    fn pop_clip(&mut self) -> Result<(), RasterError> {
        self.clip_depth = self.clip_depth.checked_sub(1).ok_or(RasterError::Stream)?;
        Ok(())
    }

    /// The clip weight at one pixel of the box, as a linear level.
    fn weight(&self, scratch: &PaintScratch<'_>, index: usize) -> u32 {
        if self.clip_depth == 0 {
            return LINEAR_ONE;
        }
        let start = self.clip_depth.saturating_sub(1).saturating_mul(self.plane);
        scratch
            .clips
            .get(start.saturating_add(index))
            .map_or(0, |value| expand(*value))
    }

    /// Open an offscreen group.
    fn push_group(&mut self, scratch: &mut PaintScratch<'_>) -> Result<(), RasterError> {
        let start = self
            .group_depth
            .checked_mul(self.plane)
            .and_then(|offset| offset.checked_mul(GROUP_PIXEL))
            .ok_or(RasterError::Overflow)?;
        let end = start
            .checked_add(
                self.plane
                    .checked_mul(GROUP_PIXEL)
                    .ok_or(RasterError::Overflow)?,
            )
            .ok_or(RasterError::Overflow)?;
        scratch
            .groups
            .get_mut(start..end)
            .ok_or(RasterError::BufferTooSmall)?
            .fill(0);
        self.group_depth = self
            .group_depth
            .checked_add(1)
            .ok_or(RasterError::Overflow)?;
        Ok(())
    }

    /// The byte range of one group level.
    fn group_range(&self, level: usize) -> Result<(usize, usize), RasterError> {
        let span = self
            .plane
            .checked_mul(GROUP_PIXEL)
            .ok_or(RasterError::Overflow)?;
        let start = level.checked_mul(span).ok_or(RasterError::Overflow)?;
        Ok((start, start.checked_add(span).ok_or(RasterError::Overflow)?))
    }

    /// Combine the two groups above with `mode` and draw the result onto the
    /// surface below them with source-over, which is the rendering algorithm
    /// of `docs/microsoft/colr.html:3332`.
    fn compose(
        &mut self,
        mode: text_core::colr::CompositeMode,
        destination: &mut Surface<'_>,
        gamma: &Gamma,
        scratch: &mut PaintScratch<'_>,
    ) -> Result<(), RasterError> {
        if self.group_depth < 2 {
            return Err(RasterError::Stream);
        }
        let source_level = self.group_depth.checked_sub(1).ok_or(RasterError::Stream)?;
        let backdrop_level = self.group_depth.checked_sub(2).ok_or(RasterError::Stream)?;
        for index in 0..self.plane {
            let source = self.group_pixel(scratch, source_level, index)?;
            let backdrop = self.group_pixel(scratch, backdrop_level, index)?;
            self.set_group_pixel(
                scratch,
                backdrop_level,
                index,
                composite(mode, source, backdrop),
            )?;
        }
        self.group_depth = source_level;
        self.merge(destination, gamma, scratch)
    }

    /// Draw the top group onto what is under it, source-over, and drop it.
    fn merge(
        &mut self,
        destination: &mut Surface<'_>,
        gamma: &Gamma,
        scratch: &mut PaintScratch<'_>,
    ) -> Result<(), RasterError> {
        let level = self.group_depth.checked_sub(1).ok_or(RasterError::Stream)?;
        self.group_depth = level;
        for index in 0..self.plane {
            let source = self.group_pixel(scratch, level, index)?;
            if source.alpha == 0 {
                continue;
            }
            self.blend(destination, gamma, scratch, index, source)?;
        }
        Ok(())
    }

    /// One pixel of one group.
    fn group_pixel(
        &self,
        scratch: &PaintScratch<'_>,
        level: usize,
        index: usize,
    ) -> Result<Pixel, RasterError> {
        let (start, _) = self.group_range(level)?;
        let at = start
            .checked_add(
                index
                    .checked_mul(GROUP_PIXEL)
                    .ok_or(RasterError::Overflow)?,
            )
            .ok_or(RasterError::Overflow)?;
        let cell = scratch
            .groups
            .get(at..at.checked_add(GROUP_PIXEL).ok_or(RasterError::Overflow)?)
            .ok_or(RasterError::BufferTooSmall)?;
        let (pairs, _) = cell.as_chunks::<2>();
        let mut channels = [0_u32; 4];
        for (channel, pair) in channels.iter_mut().zip(pairs) {
            *channel = u32::from(u16::from_le_bytes(*pair));
        }
        let [red, green, blue, alpha] = channels;
        Ok(Pixel {
            red,
            green,
            blue,
            alpha,
        })
    }

    /// Write one pixel of one group.
    fn set_group_pixel(
        &self,
        scratch: &mut PaintScratch<'_>,
        level: usize,
        index: usize,
        pixel: Pixel,
    ) -> Result<(), RasterError> {
        let (start, _) = self.group_range(level)?;
        let at = start
            .checked_add(
                index
                    .checked_mul(GROUP_PIXEL)
                    .ok_or(RasterError::Overflow)?,
            )
            .ok_or(RasterError::Overflow)?;
        let cell = scratch
            .groups
            .get_mut(at..at.checked_add(GROUP_PIXEL).ok_or(RasterError::Overflow)?)
            .ok_or(RasterError::BufferTooSmall)?;
        let (pairs, _) = cell.as_chunks_mut::<2>();
        for (pair, channel) in
            pairs
                .iter_mut()
                .zip([pixel.red, pixel.green, pixel.blue, pixel.alpha])
        {
            *pair = u16::try_from(channel.min(LINEAR_ONE))
                .unwrap_or(u16::MAX)
                .to_le_bytes();
        }
        Ok(())
    }

    /// Cover the clip region with one fill.
    #[expect(
        clippy::too_many_arguments,
        reason = "the fill needs every one of its inputs and keeps none"
    )]
    fn paint(
        &self,
        destination: &mut Surface<'_>,
        transform: Affine,
        fill: Fill,
        stops: &[ColorStop],
        foreground: Paint,
        gamma: &Gamma,
        scratch: &mut PaintScratch<'_>,
    ) -> Result<(), RasterError> {
        // The fill's geometry is stated against the glyph's origin and the
        // plane against the box.
        let placed = Affine {
            dx: transform.dx.checked_sub(Fixed::from_i32(self.bounds.x))?,
            dy: transform.dy.checked_sub(Fixed::from_i32(self.bounds.y))?,
            ..transform
        };
        let gradient = Gradient::new(fill, stops, placed)?;
        let solid = match fill {
            Fill::Solid(color) => {
                Some(Paint::of(color.source, color.alpha, foreground).pixel(255, gamma)?)
            }
            _ => None,
        };
        if gradient.is_none() && solid.is_none() {
            return Ok(());
        }
        for index in 0..self.plane {
            let weight = self.weight(scratch, index);
            if weight == 0 {
                continue;
            }
            let source = if let Some(solid) = solid {
                solid
            } else if let Some(gradient) = gradient.as_ref() {
                let (x, y) = self.centre(index)?;
                match gradient.at(x, y, foreground, gamma)? {
                    Some(pixel) => pixel,
                    None => continue,
                }
            } else {
                continue;
            };
            let source = source.weighted(weight);
            if source.alpha == 0 {
                continue;
            }
            self.blend(destination, gamma, scratch, index, source)?;
        }
        Ok(())
    }

    /// The centre of one pixel of the box, in device coordinates.
    fn centre(&self, index: usize) -> Result<(Fixed, Fixed), RasterError> {
        let width = usize::try_from(self.bounds.width).map_err(|_| RasterError::Overflow)?;
        let column = index.checked_rem(width).ok_or(RasterError::Overflow)?;
        let row = index.checked_div(width).ok_or(RasterError::Overflow)?;
        let half = Fixed::from_bits(1 << 31);
        Ok((
            Fixed::from_i32(
                i32::try_from(column)
                    .map_err(|_| RasterError::Overflow)?
                    .checked_add(self.bounds.x)
                    .ok_or(RasterError::Overflow)?,
            )
            .checked_add(half)?,
            Fixed::from_i32(
                i32::try_from(row)
                    .map_err(|_| RasterError::Overflow)?
                    .checked_add(self.bounds.y)
                    .ok_or(RasterError::Overflow)?,
            )
            .checked_add(half)?,
        ))
    }

    /// Draw one source pixel onto the innermost open group, or onto the
    /// destination when no group is open.
    fn blend(
        &self,
        destination: &mut Surface<'_>,
        gamma: &Gamma,
        scratch: &mut PaintScratch<'_>,
        index: usize,
        source: Pixel,
    ) -> Result<(), RasterError> {
        if self.group_depth > 0 {
            let level = self.group_depth.checked_sub(1).ok_or(RasterError::Stream)?;
            let backdrop = self.group_pixel(scratch, level, index)?;
            return self.set_group_pixel(scratch, level, index, source.over(backdrop));
        }
        let width = usize::try_from(self.bounds.width).map_err(|_| RasterError::Overflow)?;
        let column = index.checked_rem(width).ok_or(RasterError::Overflow)?;
        let row = index.checked_div(width).ok_or(RasterError::Overflow)?;
        let Some(x) = place(self.bounds.x, column, destination.width()) else {
            return Ok(());
        };
        let Some(y) = place(self.bounds.y, row, destination.height()) else {
            return Ok(());
        };
        let backdrop = destination.pixel(x, y, gamma).unwrap_or_default();
        destination.set_pixel(x, y, source.over(backdrop), gamma)
    }
}

/// `base + step`, or `None` outside `0..limit`.
fn place(base: i32, step: usize, limit: u32) -> Option<u32> {
    let sum = base.checked_add(i32::try_from(step).ok()?)?;
    let position = u32::try_from(sum).ok()?;
    (position < limit).then_some(position)
}

/// A coverage byte as a linear level.
fn expand(value: u8) -> u32 {
    u32::from(value).saturating_mul(257)
}

/// A linear level as a coverage byte, rounded to nearest.
fn narrow(value: u32) -> u8 {
    let scaled = value
        .min(LINEAR_ONE)
        .saturating_mul(255)
        .saturating_add(LINEAR_ONE / 2);
    u8::try_from(scaled.checked_div(LINEAR_ONE).unwrap_or(0)).unwrap_or(u8::MAX)
}
