// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! R13: drawing a `LayoutView`. The one entry point the compositor needs.
//!
//! This step reads no font table: `text-core` decodes the outline and resolves
//! the paint stream (D-173), and this module positions each glyph, quantizes
//! its origin (D-174) and dispatches it to the monochrome path or the colour
//! path.

use text_core::{
    Fixed, Font,
    colr::{ColorStop, Colr, Extents, PaintOp, Scratch as ColrScratch},
    layout::LayoutView,
};

use crate::{
    cache::{CacheKey, Glyph, GlyphCache},
    error::RasterError,
    fill::fill,
    flatten::Edge,
    gamma::Gamma,
    mono::blend_mono,
    origin::{Origin, quantize},
    outline::outline_edges,
    paint::{Bounds, PaintScratch, draw_color_glyph},
    pixel::Paint,
    surface::{Format, Mask, Surface},
    transform::Affine,
};

/// The margin in pixels a glyph's ink box is grown by before it is rasterized,
/// so that a rounding of the box never clips a stem.
const MARGIN: i32 = 2;

/// Everything one call of [`draw`] borrows besides the text and the surface.
#[derive(Debug)]
pub struct Context<'a, 'b> {
    /// The transfer function `server-display` owns (D-175, D-183).
    pub gamma: &'a Gamma,
    /// The palette row a colour glyph is drawn from.
    pub palette: u16,
    /// The colour a monochrome glyph and a `Foreground` stop take.
    pub foreground: Paint,
    /// Coverage kept between calls, or `None` to rasterize every glyph.
    pub cache: Option<&'a mut GlyphCache<'b>>,
    /// One glyph's coverage, at least as many bytes as its box holds.
    pub coverage: &'a mut [u8],
    /// Resolved paint operations of one colour glyph.
    pub ops: &'a mut [PaintOp],
    /// Colour stops of one colour glyph.
    pub stops: &'a mut [ColorStop],
    /// What one colour glyph is drawn through.
    pub paint: PaintScratch<'a>,
}

/// Draw a laid-out paragraph onto `destination`.
///
/// `faces` is the role chain `text-core` resolved against, indexed by
/// `Run::face`. `at` is where the paragraph's origin lands, in device pixels.
/// A glyph the surface does not reach is skipped, not refused.
/// # Errors
/// Returns `Face` for a run naming a face the caller did not supply, and the
/// errors of the outline decoders and of the paint stream.
pub fn draw(
    destination: &mut Surface<'_>,
    view: &LayoutView<'_>,
    faces: &[Font<'_>],
    at: (Fixed, Fixed),
    context: &mut Context<'_, '_>,
) -> Result<(), RasterError> {
    for glyph in view.glyphs {
        if glyph.hidden {
            continue;
        }
        let run = view.runs.get(glyph.run).ok_or(RasterError::Face)?;
        let font = faces.get(run.face).ok_or(RasterError::Face)?;
        let origin = quantize(at.0.checked_add(glyph.x)?, at.1.checked_add(glyph.y)?)?;
        one_glyph(destination, font, run, glyph.id, origin, context)?;
    }
    Ok(())
}

/// Draw one positioned glyph.
fn one_glyph(
    destination: &mut Surface<'_>,
    font: &Font<'_>,
    run: &text_core::resolve::Run,
    glyph: u16,
    origin: Origin,
    context: &mut Context<'_, '_>,
) -> Result<(), RasterError> {
    let transform = placement(run.scale, origin)?;
    if let Ok(colr) = Colr::parse(font)
        && colr.covers(glyph).unwrap_or(false)
    {
        return color_glyph(destination, font, run, &colr, glyph, origin, context);
    }
    monochrome(destination, font, run, glyph, origin, transform, context)
}

/// The transform from font units to device pixels for one glyph.
///
/// The run's scale takes font units to pixels, y is flipped because font space
/// grows up and device space grows down, and the origin is the quantized one
/// of R4 with its subpixel offset added back.
fn placement(scale: Fixed, origin: Origin) -> Result<Affine, RasterError> {
    Ok(Affine {
        xx: scale,
        yx: Fixed::ZERO,
        xy: Fixed::ZERO,
        yy: scale.checked_neg()?,
        dx: Fixed::from_i32(origin.x).checked_add(origin.offset())?,
        dy: Fixed::from_i32(origin.y),
    })
}

/// Rasterize one outline and composite it in the text colour.
fn monochrome(
    destination: &mut Surface<'_>,
    font: &Font<'_>,
    run: &text_core::resolve::Run,
    glyph: u16,
    origin: Origin,
    transform: Affine,
    context: &mut Context<'_, '_>,
) -> Result<(), RasterError> {
    let key = CacheKey {
        generation: run.generation,
        face: u32::try_from(run.face).map_err(|_| RasterError::Overflow)?,
        glyph,
        size: run.scale,
        subpixel: origin.subpixel,
    };
    if let Some(cache) = context.cache.as_deref()
        && let Some(stored) = cache.get(key, run.coordinates())
    {
        let mask = Mask::new(stored.coverage, stored.width, stored.height, stored.width)?;
        let at = (
            origin
                .x
                .checked_add(stored.left)
                .ok_or(RasterError::Overflow)?,
            origin
                .y
                .checked_add(stored.top)
                .ok_or(RasterError::Overflow)?,
        );
        return blend_mono(destination, at, mask, context.foreground, context.gamma);
    }
    // The box is not known before the outline is flattened, so the edges are
    // produced against the pixel the origin sits in and the box read off them.
    // The subpixel offset stays in the transform, because it is what the four
    // positions of D-174 distinguish; only the whole pixels come off.
    let count = outline_edges(
        font,
        glyph,
        run.coordinates(),
        Affine {
            dx: origin.offset(),
            dy: Fixed::ZERO,
            ..transform
        },
        &mut context.paint.outline,
        context.paint.edges,
    )?;
    let edges = context
        .paint
        .edges
        .get_mut(..count)
        .ok_or(RasterError::BufferTooSmall)?;
    let Some(bounds) = edge_bounds(edges) else {
        return Ok(());
    };
    let pixels = usize::try_from(bounds.width)
        .map_err(|_| RasterError::Overflow)?
        .checked_mul(usize::try_from(bounds.height).map_err(|_| RasterError::Overflow)?)
        .ok_or(RasterError::Overflow)?;
    let bytes = context
        .coverage
        .get_mut(..pixels)
        .ok_or(RasterError::BufferTooSmall)?;
    for edge in edges.iter_mut() {
        edge.x0 = edge.x0.checked_sub(Fixed::from_i32(bounds.x))?;
        edge.x1 = edge.x1.checked_sub(Fixed::from_i32(bounds.x))?;
        edge.y0 = edge.y0.checked_sub(Fixed::from_i32(bounds.y))?;
        edge.y1 = edge.y1.checked_sub(Fixed::from_i32(bounds.y))?;
    }
    {
        let mut plane = Surface::new(bytes, bounds.width, bounds.height, bounds.width, Format::A8)?;
        fill(edges, &mut plane, context.paint.cells)?;
    }
    let coverage: &[u8] = bytes;
    let mask = Mask::new(coverage, bounds.width, bounds.height, bounds.width)?;
    let at = (
        origin
            .x
            .checked_add(bounds.x)
            .ok_or(RasterError::Overflow)?,
        origin
            .y
            .checked_add(bounds.y)
            .ok_or(RasterError::Overflow)?,
    );
    blend_mono(destination, at, mask, context.foreground, context.gamma)?;
    if let Some(cache) = context.cache.as_deref_mut() {
        cache.insert(
            key,
            run.coordinates(),
            Glyph {
                width: bounds.width,
                height: bounds.height,
                left: bounds.x,
                top: bounds.y,
                coverage,
            },
        )?;
    }
    Ok(())
}

/// Resolve and draw one colour glyph.
fn color_glyph(
    destination: &mut Surface<'_>,
    font: &Font<'_>,
    run: &text_core::resolve::Run,
    colr: &Colr<'_>,
    glyph: u16,
    origin: Origin,
    context: &mut Context<'_, '_>,
) -> Result<(), RasterError> {
    let painted = colr.paint(
        glyph,
        context.palette,
        run.coordinates(),
        context.ops,
        context.stops,
    )?;
    if !painted.bounded {
        return Ok(());
    }
    let transform = placement(run.scale, origin)?;
    let extents = {
        let mut scratch = ColrScratch {
            points: context.paint.outline.points,
            contours: context.paint.outline.contours,
            variation: context.paint.outline.variation,
            commands: context.paint.outline.commands,
        };
        let ops = context
            .ops
            .get(..painted.ops)
            .ok_or(RasterError::Overflow)?;
        match painted.clip {
            Some(clip) => Some(clip),
            None => colr.extents(font, run.coordinates(), ops, &mut scratch)?,
        }
    };
    let Some(extents) = extents else {
        return Ok(());
    };
    let Some(bounds) = clipped(device_bounds(extents, transform)?, destination) else {
        return Ok(());
    };
    let ops = context
        .ops
        .get(..painted.ops)
        .ok_or(RasterError::Overflow)?;
    let stops = context
        .stops
        .get(..painted.stops)
        .ok_or(RasterError::Overflow)?;
    draw_color_glyph(
        destination,
        bounds,
        transform,
        font,
        run.coordinates(),
        ops,
        stops,
        painted,
        context.foreground,
        context.gamma,
        &mut context.paint,
    )
}

/// The part of a box the destination reaches, or `None` for none of it.
fn clipped(bounds: Bounds, destination: &Surface<'_>) -> Option<Bounds> {
    let left = bounds.x.max(0);
    let top = bounds.y.max(0);
    let right = bounds
        .x
        .checked_add(i32::try_from(bounds.width).ok()?)?
        .min(i32::try_from(destination.width()).ok()?);
    let bottom = bounds
        .y
        .checked_add(i32::try_from(bounds.height).ok()?)?
        .min(i32::try_from(destination.height()).ok()?);
    if right <= left || bottom <= top {
        return None;
    }
    Some(Bounds {
        x: left,
        y: top,
        width: u32::try_from(right.checked_sub(left)?).ok()?,
        height: u32::try_from(bottom.checked_sub(top)?).ok()?,
    })
}

/// The device box of a font-unit box under one transform, grown by [`MARGIN`].
fn device_bounds(extents: Extents, transform: Affine) -> Result<Bounds, RasterError> {
    let mut low = (Fixed::from_bits(i64::MAX), Fixed::from_bits(i64::MAX));
    let mut high = (Fixed::from_bits(i64::MIN), Fixed::from_bits(i64::MIN));
    for (x, y) in [
        (extents.x_min, extents.y_min),
        (extents.x_min, extents.y_max),
        (extents.x_max, extents.y_min),
        (extents.x_max, extents.y_max),
    ] {
        let (px, py) = transform.apply(x, y)?;
        low = (low.0.min(px), low.1.min(py));
        high = (high.0.max(px), high.1.max(py));
    }
    box_of(low, high)
}

/// The device box two corners fall in, grown by [`MARGIN`] on every side.
fn box_of(low: (Fixed, Fixed), high: (Fixed, Fixed)) -> Result<Bounds, RasterError> {
    let left = floor(low.0)?
        .checked_sub(MARGIN)
        .ok_or(RasterError::Overflow)?;
    let top = floor(low.1)?
        .checked_sub(MARGIN)
        .ok_or(RasterError::Overflow)?;
    let right = ceil(high.0)?
        .checked_add(MARGIN)
        .ok_or(RasterError::Overflow)?;
    let bottom = ceil(high.1)?
        .checked_add(MARGIN)
        .ok_or(RasterError::Overflow)?;
    Ok(Bounds {
        x: left,
        y: top,
        width: u32::try_from(right.checked_sub(left).ok_or(RasterError::Overflow)?).unwrap_or(0),
        height: u32::try_from(bottom.checked_sub(top).ok_or(RasterError::Overflow)?).unwrap_or(0),
    })
}

/// The box a set of edges falls in, grown by [`MARGIN`]. `None` for no edge.
fn edge_bounds(edges: &[Edge]) -> Option<Bounds> {
    let first = edges.first()?;
    let mut low = (first.x0.min(first.x1), first.y0.min(first.y1));
    let mut high = (first.x0.max(first.x1), first.y0.max(first.y1));
    for edge in edges {
        low = (
            low.0.min(edge.x0).min(edge.x1),
            low.1.min(edge.y0).min(edge.y1),
        );
        high = (
            high.0.max(edge.x0).max(edge.x1),
            high.1.max(edge.y0).max(edge.y1),
        );
    }
    let bounds = box_of(low, high).ok()?;
    (bounds.width > 0 && bounds.height > 0).then_some(bounds)
}

/// The largest integer at or below a Q32.32 value.
fn floor(value: Fixed) -> Result<i32, RasterError> {
    i32::try_from(value.bits() >> 32).map_err(|_| RasterError::Overflow)
}

/// The smallest integer at or above a Q32.32 value.
fn ceil(value: Fixed) -> Result<i32, RasterError> {
    let whole = value.bits() >> 32;
    let up = if value.bits() & ((1_i64 << 32) - 1) == 0 {
        whole
    } else {
        whole.saturating_add(1)
    };
    i32::try_from(up).map_err(|_| RasterError::Overflow)
}
