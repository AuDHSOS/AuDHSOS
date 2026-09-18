// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Paint tables and the operation stream they resolve to;
//! `docs/microsoft/colr.html:1714`.

use super::{
    Colr, Extents,
    cpal::Color,
    trig::{sin_cos, tan},
};
use crate::{
    Fixed, FontError,
    read::{self, add, mul},
};

/// Maximum paint tables on one root-to-leaf path.
pub const MAX_DEPTH: usize = 64;
/// Maximum paint table visits for one colour glyph.
pub const MAX_PAINTS: usize = 65_536;

/// A 2x3 affine transform in font units; `docs/microsoft/colr.html:2195`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Affine {
    /// x component of the transformed x basis vector.
    pub xx: Fixed,
    /// y component of the transformed x basis vector.
    pub yx: Fixed,
    /// x component of the transformed y basis vector.
    pub xy: Fixed,
    /// y component of the transformed y basis vector.
    pub yy: Fixed,
    /// Translation along x.
    pub dx: Fixed,
    /// Translation along y.
    pub dy: Fixed,
}

impl Default for Affine {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Affine {
    /// The transform that moves no point.
    pub const IDENTITY: Self = Self {
        xx: Fixed::ONE,
        yx: Fixed::ZERO,
        xy: Fixed::ZERO,
        yy: Fixed::ONE,
        dx: Fixed::ZERO,
        dy: Fixed::ZERO,
    };

    /// Map one position, `x' = xx*x + xy*y + dx` and `y' = yx*x + yy*y + dy`.
    /// # Errors
    /// Returns `Overflow` when a product or sum exceeds Q32.32.
    pub fn apply(self, x: Fixed, y: Fixed) -> Result<(Fixed, Fixed), FontError> {
        Ok((
            self.xx
                .checked_mul(x)?
                .checked_add(self.xy.checked_mul(y)?)?
                .checked_add(self.dx)?,
            self.yx
                .checked_mul(x)?
                .checked_add(self.yy.checked_mul(y)?)?
                .checked_add(self.dy)?,
        ))
    }

    /// The transform that applies `inner` and then `self`.
    /// # Errors
    /// Returns `Overflow` when a product or sum exceeds Q32.32.
    pub fn compose(self, inner: Self) -> Result<Self, FontError> {
        let (dx, dy) = self.apply(inner.dx, inner.dy)?;
        Ok(Self {
            xx: self
                .xx
                .checked_mul(inner.xx)?
                .checked_add(self.xy.checked_mul(inner.yx)?)?,
            yx: self
                .yx
                .checked_mul(inner.xx)?
                .checked_add(self.yy.checked_mul(inner.yx)?)?,
            xy: self
                .xx
                .checked_mul(inner.xy)?
                .checked_add(self.xy.checked_mul(inner.yy)?)?,
            yy: self
                .yx
                .checked_mul(inner.xy)?
                .checked_add(self.yy.checked_mul(inner.yy)?)?,
            dx,
            dy,
        })
    }

    const fn translate(dx: Fixed, dy: Fixed) -> Self {
        Self {
            dx,
            dy,
            ..Self::IDENTITY
        }
    }

    fn around(self, x: Fixed, y: Fixed) -> Result<Self, FontError> {
        Self::translate(x, y)
            .compose(self)?
            .compose(Self::translate(x.checked_neg()?, y.checked_neg()?))
    }
}

/// How a colour line continues past its outermost stops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Extend {
    /// Use the nearest colour stop. Unrecognized values resolve here.
    #[default]
    Pad,
    /// Repeat from the farthest colour stop.
    Repeat,
    /// Mirror the colour line from the nearest end.
    Reflect,
}

/// One position on a colour line and the colour there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColorStop {
    /// Position on the colour line.
    pub offset: Fixed,
    /// Colour at that position.
    pub color: Color,
}

/// A colour line: an extend mode and a range of the caller's stop slice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColorLine {
    /// Behaviour past the outermost stops.
    pub extend: Extend,
    /// Index of the first stop in the caller's slice.
    pub first: usize,
    /// Number of stops, in increasing offset order.
    pub count: usize,
}

/// What covers the clip region of one fill operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    /// One colour over the whole region.
    Solid(Color),
    /// A gradient along the line from `p0` to `p1`, rotated about `p2`.
    Linear {
        /// Start point x.
        x0: Fixed,
        /// Start point y.
        y0: Fixed,
        /// End point x.
        x1: Fixed,
        /// End point y.
        y1: Fixed,
        /// Rotation point x.
        x2: Fixed,
        /// Rotation point y.
        y2: Fixed,
        /// The colour line.
        line: ColorLine,
    },
    /// A gradient between two circles.
    Radial {
        /// Start circle centre x.
        x0: Fixed,
        /// Start circle centre y.
        y0: Fixed,
        /// Start circle radius.
        r0: Fixed,
        /// End circle centre x.
        x1: Fixed,
        /// End circle centre y.
        y1: Fixed,
        /// End circle radius.
        r1: Fixed,
        /// The colour line.
        line: ColorLine,
    },
    /// A gradient swept about a centre between two angles.
    Sweep {
        /// Centre x.
        x: Fixed,
        /// Centre y.
        y: Fixed,
        /// Start angle in half-turns counter-clockwise, the stored bias applied.
        start: Fixed,
        /// End angle in half-turns counter-clockwise, the stored bias applied.
        end: Fixed,
        /// The colour line.
        line: ColorLine,
    },
}

/// A compositing or blending mode; `docs/microsoft/colr.html:3094`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum CompositeMode {
    /// Clear. Unrecognized mode values resolve here.
    #[default]
    Clear = 0,
    /// Source.
    Src = 1,
    /// Destination.
    Dest = 2,
    /// Source over.
    SrcOver = 3,
    /// Destination over.
    DestOver = 4,
    /// Source in.
    SrcIn = 5,
    /// Destination in.
    DestIn = 6,
    /// Source out.
    SrcOut = 7,
    /// Destination out.
    DestOut = 8,
    /// Source atop.
    SrcAtop = 9,
    /// Destination atop.
    DestAtop = 10,
    /// Exclusive or.
    Xor = 11,
    /// Plus.
    Plus = 12,
    /// Screen.
    Screen = 13,
    /// Overlay.
    Overlay = 14,
    /// Darken.
    Darken = 15,
    /// Lighten.
    Lighten = 16,
    /// Colour dodge.
    ColorDodge = 17,
    /// Colour burn.
    ColorBurn = 18,
    /// Hard light.
    HardLight = 19,
    /// Soft light.
    SoftLight = 20,
    /// Difference.
    Difference = 21,
    /// Exclusion.
    Exclusion = 22,
    /// Multiply.
    Multiply = 23,
    /// Hue.
    Hue = 24,
    /// Saturation.
    Saturation = 25,
    /// Colour.
    Color = 26,
    /// Luminosity.
    Luminosity = 27,
}

impl CompositeMode {
    /// Whether a composition of these two operands is visually bounded;
    /// `docs/microsoft/colr.html:3301`.
    #[must_use]
    pub const fn bounds(self, source: bool, backdrop: bool) -> bool {
        match self {
            Self::Clear => true,
            Self::Src | Self::SrcOut => source,
            Self::Dest | Self::DestOut => backdrop,
            Self::SrcIn | Self::DestIn => source || backdrop,
            _ => source && backdrop,
        }
    }

    /// The mode of one stored value; unrecognized values give `Clear`.
    #[must_use]
    pub const fn from_value(value: u8) -> Self {
        match value {
            1 => Self::Src,
            2 => Self::Dest,
            3 => Self::SrcOver,
            4 => Self::DestOver,
            5 => Self::SrcIn,
            6 => Self::DestIn,
            7 => Self::SrcOut,
            8 => Self::DestOut,
            9 => Self::SrcAtop,
            10 => Self::DestAtop,
            11 => Self::Xor,
            12 => Self::Plus,
            13 => Self::Screen,
            14 => Self::Overlay,
            15 => Self::Darken,
            16 => Self::Lighten,
            17 => Self::ColorDodge,
            18 => Self::ColorBurn,
            19 => Self::HardLight,
            20 => Self::SoftLight,
            21 => Self::Difference,
            22 => Self::Exclusion,
            23 => Self::Multiply,
            24 => Self::Hue,
            25 => Self::Saturation,
            26 => Self::Color,
            27 => Self::Luminosity,
            _ => Self::Clear,
        }
    }
}

/// One step of the resolved drawing stream of a colour glyph.
///
/// The stream carries two stacks. `Clip` and `Unclip` bracket a clip region;
/// `Group` and `Compose` bracket an offscreen surface. Each `Compose` consumes
/// the two groups above it and leaves their combination.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintOp {
    /// Intersect the clip region with the outline of `glyph` under `transform`.
    Clip {
        /// Glyph whose outline bounds the following operations.
        glyph: u16,
        /// Transform of that outline, in font units.
        transform: Affine,
    },
    /// Restore the clip region the matching `Clip` narrowed.
    #[default]
    Unclip,
    /// Start an offscreen group.
    Group,
    /// Combine the two groups above with `mode` and leave one result.
    Compose(CompositeMode),
    /// Cover the clip region with `fill`, whose geometry is in `transform`.
    Fill {
        /// Transform of the fill geometry, in font units.
        transform: Affine,
        /// What covers the region.
        fill: Fill,
    },
}

/// The scale of one stored field, used to apply integer variation deltas.
#[derive(Clone, Copy)]
enum Unit {
    /// A signed font unit, FWORD or UFWORD.
    Word,
    /// A 2.14 fraction.
    F2Dot14,
    /// A 16.16 fraction.
    F16Dot16,
}

impl Unit {
    const fn step(self) -> Fixed {
        match self {
            Self::Word => Fixed::ONE,
            Self::F2Dot14 => Fixed::from_bits(1 << 18),
            Self::F16Dot16 => Fixed::from_bits(1 << 16),
        }
    }
}

/// One traversal of the paint graph of one colour glyph.
pub(super) struct Walk<'a, 'b> {
    pub(super) colr: &'b Colr<'a>,
    pub(super) palette: u16,
    pub(super) coords: &'b [Fixed],
    pub(super) ops: &'b mut [PaintOp],
    pub(super) stops: &'b mut [ColorStop],
    pub(super) op_count: usize,
    pub(super) stop_count: usize,
    pub(super) visits: usize,
    pub(super) path: [usize; MAX_DEPTH],
}

impl Walk<'_, '_> {
    fn emit(&mut self, op: PaintOp) -> Result<(), FontError> {
        *self
            .ops
            .get_mut(self.op_count)
            .ok_or(FontError::BufferTooSmall)? = op;
        self.op_count = add(self.op_count, 1)?;
        Ok(())
    }

    /// One field's value: the stored number plus its interpolated delta.
    fn varied(&self, stored: Fixed, unit: Unit, index: Option<u32>) -> Result<Fixed, FontError> {
        let (Some(index), Some(store)) = (index, self.colr.store) else {
            return Ok(stored);
        };
        let (outer, inner) = match self.colr.map {
            Some(map) => map.get(read::offset(index)?)?,
            None => (
                u16::try_from(index >> 16).map_err(|_| FontError::InvalidTable)?,
                u16::try_from(index & 0xffff).map_err(|_| FontError::InvalidTable)?,
            ),
        };
        stored.checked_add(
            store
                .delta(outer, inner, self.coords)?
                .checked_mul(unit.step())?,
        )
    }

    /// The six delta-set indices of a table whose `varIndexBase` lies at `at`.
    ///
    /// A non-variable format, a face without an item variation store, and the
    /// reserved base `0xFFFFFFFF` each give six absent indices.
    fn bases(&self, data: &[u8], at: usize, variable: bool) -> Result<[Option<u32>; 6], FontError> {
        if !variable || self.colr.store.is_none() {
            return Ok([None; 6]);
        }
        let base = read::u32(data, at)?;
        if base == 0xffff_ffff {
            return Ok([None; 6]);
        }
        let mut out = [None; 6];
        for (step, slot) in out.iter_mut().enumerate() {
            let step = u32::try_from(step).map_err(|_| FontError::Overflow)?;
            // The spec forbids the sequence from wrapping past 0xFFFFFFFF.
            *slot = Some(base.checked_add(step).ok_or(FontError::InvalidTable)?);
        }
        Ok(out)
    }

    fn word(&self, data: &[u8], at: usize, var: Option<u32>) -> Result<Fixed, FontError> {
        self.varied(
            Fixed::from_i32(i32::from(read::i16(data, at)?)),
            Unit::Word,
            var,
        )
    }

    fn length(&self, data: &[u8], at: usize, var: Option<u32>) -> Result<Fixed, FontError> {
        self.varied(
            Fixed::from_i32(i32::from(read::u16(data, at)?)),
            Unit::Word,
            var,
        )
    }

    fn fraction(&self, data: &[u8], at: usize, var: Option<u32>) -> Result<Fixed, FontError> {
        self.varied(Fixed::from_2_14(read::i16(data, at)?), Unit::F2Dot14, var)
    }

    fn scalar(&self, data: &[u8], at: usize, var: Option<u32>) -> Result<Fixed, FontError> {
        self.varied(Fixed::from_16_16(read::i32(data, at)?), Unit::F16Dot16, var)
    }

    /// Read a colour line into the caller's stop slice and return its range.
    ///
    /// Stops are written in increasing offset order, which a variable font can
    /// change, so each is placed by binary search and the tail moved up.
    fn color_line(&mut self, at: usize, variable: bool) -> Result<ColorLine, FontError> {
        let data = read::tail(self.colr.data, at)?;
        let extend = match read::u8(data, 0)? {
            1 => Extend::Repeat,
            2 => Extend::Reflect,
            _ => Extend::Pad,
        };
        let count = usize::from(read::u16(data, 1)?);
        let width = if variable { 10 } else { 6 };
        let records = read::bytes(data, 3, mul(count, width)?)?;
        let first = self.stop_count;
        for index in 0..count {
            let record = read::bytes(records, mul(index, width)?, width)?;
            let [offset_var, alpha_var, ..] = self.bases(record, 6, variable)?;
            let stop = ColorStop {
                offset: self.fraction(record, 0, offset_var)?,
                color: self.colr.cpal.color(
                    self.palette,
                    read::u16(record, 2)?,
                    self.fraction(record, 4, alpha_var)?,
                )?,
            };
            let at = self.place(first, stop.offset)?;
            self.insert(at, stop)?;
        }
        Ok(ColorLine {
            extend,
            first,
            count,
        })
    }

    fn place(&self, first: usize, offset: Fixed) -> Result<usize, FontError> {
        let stops = self
            .stops
            .get(first..self.stop_count)
            .ok_or(FontError::BufferTooSmall)?;
        add(first, stops.partition_point(|stop| stop.offset <= offset))
    }

    fn insert(&mut self, at: usize, stop: ColorStop) -> Result<(), FontError> {
        let end = add(self.stop_count, 1)?;
        self.stops
            .get_mut(at..end)
            .ok_or(FontError::BufferTooSmall)?
            .rotate_right(1);
        *self.stops.get_mut(at).ok_or(FontError::BufferTooSmall)? = stop;
        self.stop_count = end;
        Ok(())
    }

    /// One version 0 layer: its outline clipped, then filled opaque.
    pub(super) fn solid_layer(&mut self, glyph: u16, entry: u16) -> Result<(), FontError> {
        let glyph = self.colr.glyph(glyph)?;
        let color = self.colr.cpal.color(self.palette, entry, Fixed::ONE)?;
        self.emit(PaintOp::Clip {
            glyph,
            transform: Affine::IDENTITY,
        })?;
        self.emit(PaintOp::Fill {
            transform: Affine::IDENTITY,
            fill: Fill::Solid(color),
        })?;
        self.emit(PaintOp::Unclip)
    }

    /// The box of one `ClipBox` table, format 1 or the variable format 2.
    pub(super) fn clip_extents(&self, data: &[u8], variable: bool) -> Result<Extents, FontError> {
        let [b0, b1, b2, b3, ..] = self.bases(data, 9, variable)?;
        Ok(Extents {
            x_min: self.word(data, 1, b0)?,
            y_min: self.word(data, 3, b1)?,
            x_max: self.word(data, 5, b2)?,
            y_max: self.word(data, 7, b3)?,
        })
    }

    /// Resolve the paint table at `at` and its sub-graph into operations.
    ///
    /// Returns whether the sub-graph is visually bounded, which the section
    /// "Metrics and boundedness of color glyphs using version 1 formats"
    /// defines per format (`docs/microsoft/colr.html:1100`).
    pub(super) fn paint(
        &mut self,
        at: usize,
        depth: usize,
        transform: Affine,
    ) -> Result<bool, FontError> {
        if self
            .path
            .get(..depth)
            .ok_or(FontError::LimitExceeded)?
            .contains(&at)
        {
            return Err(FontError::Cycle);
        }
        *self.path.get_mut(depth).ok_or(FontError::LimitExceeded)? = at;
        self.visits = add(self.visits, 1)?;
        if self.visits > MAX_PAINTS {
            return Err(FontError::LimitExceeded);
        }
        let data = read::tail(self.colr.data, at)?;
        let next = add(depth, 1)?;
        match read::u8(data, 0)? {
            1 => self.layers(data, next, transform),
            format @ (2 | 3) => {
                let [alpha, ..] = self.bases(data, 5, format == 3)?;
                let color = self.colr.cpal.color(
                    self.palette,
                    read::u16(data, 1)?,
                    self.fraction(data, 3, alpha)?,
                )?;
                self.emit(PaintOp::Fill {
                    transform,
                    fill: Fill::Solid(color),
                })?;
                Ok(false)
            }
            format @ (4 | 5) => self.linear(at, transform, format == 5),
            format @ (6 | 7) => self.radial(at, transform, format == 7),
            format @ (8 | 9) => self.sweep(at, transform, format == 9),
            10 => {
                let glyph = self.colr.glyph(read::u16(data, 4)?)?;
                let child = add(at, read::offset(read::u24(data, 1)?)?)?;
                self.emit(PaintOp::Clip { glyph, transform })?;
                self.paint(child, next, transform)?;
                self.emit(PaintOp::Unclip)?;
                Ok(true)
            }
            11 => {
                let glyph = self.colr.glyph(read::u16(data, 1)?)?;
                let root = self.colr.root(glyph)?.ok_or(FontError::InvalidTable)?;
                self.paint(root, next, transform)
            }
            format @ 12..=31 => {
                let inner = self.matrix(data, at, format)?;
                let child = add(at, read::offset(read::u24(data, 1)?)?)?;
                self.paint(child, next, transform.compose(inner)?)
            }
            32 => {
                let mode = CompositeMode::from_value(read::u8(data, 4)?);
                let source = add(at, read::offset(read::u24(data, 1)?)?)?;
                let backdrop = add(at, read::offset(read::u24(data, 5)?)?)?;
                self.emit(PaintOp::Group)?;
                let backdrop = self.paint(backdrop, next, transform)?;
                self.emit(PaintOp::Group)?;
                let source = self.paint(source, next, transform)?;
                self.emit(PaintOp::Compose(mode))?;
                Ok(mode.bounds(source, backdrop))
            }
            // A format this version does not define, and its sub-graph, are
            // ignored and count as bounded; the rest of the graph resolves.
            _ => Ok(true),
        }
    }

    fn layers(&mut self, data: &[u8], depth: usize, transform: Affine) -> Result<bool, FontError> {
        let count = usize::from(read::u8(data, 1)?);
        let first = read::offset(read::u32(data, 2)?)?;
        let list = self.colr.layer_list.ok_or(FontError::InvalidTable)?;
        let layers = read::tail(self.colr.data, list)?;
        if add(first, count)? > read::offset(read::u32(layers, 0)?)? {
            return Err(FontError::InvalidTable);
        }
        let mut bounded = true;
        for index in 0..count {
            let at = add(4, mul(add(first, index)?, 4)?)?;
            let child = add(list, read::offset(read::u32(layers, at)?)?)?;
            bounded &= self.paint(child, depth, transform)?;
        }
        Ok(bounded)
    }

    fn linear(&mut self, at: usize, transform: Affine, variable: bool) -> Result<bool, FontError> {
        let data = read::tail(self.colr.data, at)?;
        let [b0, b1, b2, b3, b4, b5] = self.bases(data, 16, variable)?;
        let fill = Fill::Linear {
            x0: self.word(data, 4, b0)?,
            y0: self.word(data, 6, b1)?,
            x1: self.word(data, 8, b2)?,
            y1: self.word(data, 10, b3)?,
            x2: self.word(data, 12, b4)?,
            y2: self.word(data, 14, b5)?,
            line: self.color_line(add(at, read::offset(read::u24(data, 1)?)?)?, variable)?,
        };
        self.emit(PaintOp::Fill { transform, fill })?;
        Ok(false)
    }

    fn radial(&mut self, at: usize, transform: Affine, variable: bool) -> Result<bool, FontError> {
        let data = read::tail(self.colr.data, at)?;
        let [b0, b1, b2, b3, b4, b5] = self.bases(data, 16, variable)?;
        let fill = Fill::Radial {
            x0: self.word(data, 4, b0)?,
            y0: self.word(data, 6, b1)?,
            r0: self.length(data, 8, b2)?,
            x1: self.word(data, 10, b3)?,
            y1: self.word(data, 12, b4)?,
            r1: self.length(data, 14, b5)?,
            line: self.color_line(add(at, read::offset(read::u24(data, 1)?)?)?, variable)?,
        };
        self.emit(PaintOp::Fill { transform, fill })?;
        Ok(false)
    }

    fn sweep(&mut self, at: usize, transform: Affine, variable: bool) -> Result<bool, FontError> {
        let data = read::tail(self.colr.data, at)?;
        let [b0, b1, b2, b3, ..] = self.bases(data, 12, variable)?;
        // Sweep angles carry a bias of one half-turn so that a full turn fits.
        let fill = Fill::Sweep {
            x: self.word(data, 4, b0)?,
            y: self.word(data, 6, b1)?,
            start: self.fraction(data, 8, b2)?.checked_add(Fixed::ONE)?,
            end: self.fraction(data, 10, b3)?.checked_add(Fixed::ONE)?,
            line: self.color_line(add(at, read::offset(read::u24(data, 1)?)?)?, variable)?,
        };
        self.emit(PaintOp::Fill { transform, fill })?;
        Ok(false)
    }

    /// The transform of one of the twenty affine paint formats, 12 to 31.
    fn matrix(&self, data: &[u8], at: usize, format: u8) -> Result<Affine, FontError> {
        let variable = format % 2 == 1;
        match format {
            12 | 13 => {
                let table =
                    read::tail(self.colr.data, add(at, read::offset(read::u24(data, 4)?)?)?)?;
                let [b0, b1, b2, b3, b4, b5] = self.bases(table, 24, variable)?;
                Ok(Affine {
                    xx: self.scalar(table, 0, b0)?,
                    yx: self.scalar(table, 4, b1)?,
                    xy: self.scalar(table, 8, b2)?,
                    yy: self.scalar(table, 12, b3)?,
                    dx: self.scalar(table, 16, b4)?,
                    dy: self.scalar(table, 20, b5)?,
                })
            }
            14 | 15 => {
                let [b0, b1, ..] = self.bases(data, 8, variable)?;
                Ok(Affine::translate(
                    self.word(data, 4, b0)?,
                    self.word(data, 6, b1)?,
                ))
            }
            16..=19 => {
                let [b0, b1, b2, b3, ..] =
                    self.bases(data, if format == 17 { 8 } else { 12 }, variable)?;
                let scale = Affine {
                    xx: self.fraction(data, 4, b0)?,
                    yy: self.fraction(data, 6, b1)?,
                    ..Affine::IDENTITY
                };
                if format < 18 {
                    return Ok(scale);
                }
                scale.around(self.word(data, 8, b2)?, self.word(data, 10, b3)?)
            }
            20..=23 => {
                let [b0, b1, b2, ..] =
                    self.bases(data, if format == 21 { 6 } else { 10 }, variable)?;
                let factor = self.fraction(data, 4, b0)?;
                let scale = Affine {
                    xx: factor,
                    yy: factor,
                    ..Affine::IDENTITY
                };
                if format < 22 {
                    return Ok(scale);
                }
                scale.around(self.word(data, 6, b1)?, self.word(data, 8, b2)?)
            }
            24..=27 => {
                let [b0, b1, b2, ..] =
                    self.bases(data, if format == 25 { 6 } else { 10 }, variable)?;
                let (sine, cosine) = sin_cos(self.fraction(data, 4, b0)?)?;
                let rotate = Affine {
                    xx: cosine,
                    yx: sine,
                    xy: sine.checked_neg()?,
                    yy: cosine,
                    ..Affine::IDENTITY
                };
                if format < 26 {
                    return Ok(rotate);
                }
                rotate.around(self.word(data, 6, b1)?, self.word(data, 8, b2)?)
            }
            _ => {
                let [b0, b1, b2, b3, ..] =
                    self.bases(data, if format == 29 { 8 } else { 12 }, variable)?;
                let skew = Affine {
                    xy: tan(self.fraction(data, 4, b0)?)?.checked_neg()?,
                    yx: tan(self.fraction(data, 6, b1)?)?,
                    ..Affine::IDENTITY
                };
                if format < 30 {
                    return Ok(skew);
                }
                skew.around(self.word(data, 8, b2)?, self.word(data, 10, b3)?)
            }
        }
    }
}
