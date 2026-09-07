// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A surface: the bytes of a picture, and what may be done to them.
//!
//! The surface borrows its bytes. What they are is the caller's business —
//! the framebuffer of the machine, a back buffer out of the memory server,
//! or an array a test owns — and the drawing is the same in every case. A
//! row may be wider than the picture, which is what `stride` says, and the
//! bytes past the last visible column of a row are never touched.
//!
//! Invariants: every byte written lies inside the slice the surface was
//! built over, because a drawing operation clips to the surface first and
//! every offset is then computed from coordinates that are inside it; a
//! drawing operation records what it changed in the damage set.

use core::ops::Range;

use crate::format::{BYTES_PER_PIXEL, Color, PixelFormat};
use crate::rect::{Damage, Rect};

/// Why a surface could not be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SurfaceError {
    /// Width or height is zero.
    Empty,
    /// The row length is below the width.
    Stride,
    /// The dimensions do not fit an address of this machine.
    Overflow,
    /// The slice is shorter than `height * stride * 4` bytes.
    TooShort {
        /// What the dimensions need.
        needed: usize,
        /// What the slice holds.
        given: usize,
    },
}

impl core::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SurfaceError::Empty => f.write_str("a surface of zero width or zero height"),
            SurfaceError::Stride => f.write_str("a row shorter than the width of the surface"),
            SurfaceError::Overflow => f.write_str("a surface larger than this address space"),
            SurfaceError::TooShort { needed, given } => {
                write!(f, "a surface of {needed} bytes over a slice of {given}")
            }
        }
    }
}

/// How many bytes a picture of these dimensions needs, or `None` when that
/// is not a length of this machine.
fn needed_bytes(height: u32, stride: u32) -> Option<usize> {
    let pixels = u64::from(height).saturating_mul(u64::from(stride));
    let bytes = pixels.checked_mul(u64::from(BYTES_PER_PIXEL))?;
    usize::try_from(bytes).ok()
}

/// The bytes of a picture and what is drawn into them.
#[derive(Debug)]
pub struct Surface<'a> {
    /// The bytes, `stride * height * 4` of them at least.
    bytes: &'a mut [u8],
    /// Visible columns.
    width: u32,
    /// Visible rows.
    height: u32,
    /// Pixels from the start of one row to the start of the next.
    stride: u32,
    /// The order of the channels in `bytes`.
    format: PixelFormat,
    /// What changed since the damage was last cleared.
    damage: Damage,
}

impl<'a> Surface<'a> {
    /// A surface of this size over `bytes`.
    ///
    /// # Errors
    ///
    /// [`SurfaceError::Empty`] for a zero dimension, [`SurfaceError::Stride`]
    /// for a row shorter than the width, [`SurfaceError::Overflow`] for
    /// dimensions whose product is not a length of this machine, and
    /// [`SurfaceError::TooShort`] for a slice that does not hold the rows.
    pub fn new(
        bytes: &'a mut [u8],
        width: u32,
        height: u32,
        stride: u32,
        format: PixelFormat,
    ) -> Result<Self, SurfaceError> {
        if width == 0 || height == 0 {
            return Err(SurfaceError::Empty);
        }
        if stride < width {
            return Err(SurfaceError::Stride);
        }
        let needed = needed_bytes(height, stride).ok_or(SurfaceError::Overflow)?;
        let given = bytes.len();
        if given < needed {
            return Err(SurfaceError::TooShort { needed, given });
        }
        Ok(Surface {
            bytes,
            width,
            height,
            stride,
            format,
            damage: Damage::new(),
        })
    }

    /// Visible columns.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Visible rows.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Pixels from the start of one row to the start of the next.
    #[must_use]
    pub const fn stride(&self) -> u32 {
        self.stride
    }

    /// The order of the channels.
    #[must_use]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    /// The whole visible area.
    #[must_use]
    pub const fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    /// What changed since the damage was last cleared.
    #[must_use]
    pub const fn damage(&self) -> &Damage {
        &self.damage
    }

    /// Forgets what changed, which is what a presentation does.
    pub const fn clear_damage(&mut self) {
        self.damage.clear();
    }

    /// Records that `rect` changed, for a caller that wrote through
    /// [`row_bytes_mut`](Self::row_bytes_mut).
    pub fn add_damage(&mut self, rect: Rect) {
        self.damage.push(rect.clip_to(self.width, self.height));
    }

    /// The bytes of the surface, whatever stands in them.
    #[must_use]
    pub const fn bytes(&self) -> &[u8] {
        self.bytes
    }

    /// The byte range of `count` pixels from `x` in row `y`, or `None` when
    /// they do not all lie inside the surface.
    fn range(&self, x: u32, y: u32, count: u32) -> Option<Range<usize>> {
        if y >= self.height || x.saturating_add(count) > self.width {
            return None;
        }
        let pixel = u64::from(y)
            .saturating_mul(u64::from(self.stride))
            .saturating_add(u64::from(x));
        let start = pixel.saturating_mul(u64::from(BYTES_PER_PIXEL));
        let end = start.saturating_add(u64::from(count).saturating_mul(u64::from(BYTES_PER_PIXEL)));
        Some(
            usize::try_from(start).unwrap_or(usize::MAX)
                ..usize::try_from(end).unwrap_or(usize::MAX),
        )
    }

    /// The bytes of `count` pixels from `x` in row `y`.
    #[must_use]
    pub fn row_bytes(&self, x: u32, y: u32, count: u32) -> Option<&[u8]> {
        self.bytes.get(self.range(x, y, count)?)
    }

    /// The bytes of `count` pixels from `x` in row `y`, to write into.
    #[must_use]
    pub fn row_bytes_mut(&mut self, x: u32, y: u32, count: u32) -> Option<&mut [u8]> {
        let range = self.range(x, y, count)?;
        self.bytes.get_mut(range)
    }

    /// The color at `x`, `y`, or `None` outside the surface.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<Color> {
        let bytes = self.row_bytes(x, y, 1)?;
        let pixel = bytes
            .as_chunks::<4>()
            .0
            .first()
            .copied()
            .unwrap_or_default();
        Some(Color::decode(self.format, pixel))
    }

    /// Writes one pixel and reports whether it was inside the surface. It
    /// records no damage: the operation that draws does that once for
    /// everything it drew.
    pub fn set_pixel(&mut self, x: u32, y: u32, color: Color) -> bool {
        let encoded = color.encode(self.format);
        let row = self.row_bytes_mut(x, y, 1).unwrap_or_default();
        let inside = !row.is_empty();
        for chunk in row.as_chunks_mut::<4>().0 {
            *chunk = encoded;
        }
        inside
    }

    /// Fills the part of `rect` that lies inside the surface and answers
    /// with what it filled.
    pub fn fill(&mut self, rect: Rect, color: Color) -> Rect {
        let area = rect.clip_to(self.width, self.height);
        let encoded = color.encode(self.format);
        for y in area.y..area.bottom() {
            let row = self.row_bytes_mut(area.x, y, area.w).unwrap_or_default();
            for chunk in row.as_chunks_mut::<4>().0 {
                *chunk = encoded;
            }
        }
        self.damage.push(area);
        area
    }

    /// Copies `src_rect` of `source` to `x`, `y` of this surface, clipped to
    /// both, and answers with what it wrote. The formats may differ: every
    /// pixel is read as a color and written as one.
    pub fn blit(&mut self, source: &Surface<'_>, src_rect: Rect, x: u32, y: u32) -> Rect {
        let from = src_rect.clip_to(source.width(), source.height());
        let placed = Rect::new(x, y, from.w, from.h).clip_to(self.width, self.height);
        let target_format = self.format;
        let source_format = source.format();
        for row in 0..placed.h {
            let taken = source
                .row_bytes(from.x, from.y.saturating_add(row), placed.w)
                .unwrap_or_default();
            let written = self
                .row_bytes_mut(placed.x, placed.y.saturating_add(row), placed.w)
                .unwrap_or_default();
            let pixels = taken.as_chunks::<4>().0;
            for (target, pixel) in written.as_chunks_mut::<4>().0.iter_mut().zip(pixels) {
                let color = Color::decode(source_format, *pixel);
                *target = color.encode(target_format);
            }
        }
        self.damage.push(placed);
        placed
    }
}
