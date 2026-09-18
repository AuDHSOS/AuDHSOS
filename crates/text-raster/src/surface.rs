// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The surface every operation of this crate writes through; D-178.
//!
//! Invariants: every byte a surface writes lies inside the slice it was built
//! over, because a position is checked against the width and the height before
//! an offset is computed from it, and the offset is then checked against the
//! slice; the bytes of a row past its last visible column are never written,
//! because an offset is computed from a column below the width.

use crate::error::RasterError;

/// How the bytes of one pixel are stored; D-178.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Format {
    /// One byte of coverage or alpha, on a linear scale.
    #[default]
    A8,
    /// Red, green, blue and alpha as little-endian `u16`, premultiplied,
    /// on a linear-light scale.
    Rgba16,
    /// Red, green, blue and one unused byte, opaque, in display space.
    Rgbx8888,
    /// Blue, green, red and one unused byte, opaque, in display space.
    Bgrx8888,
}

impl Format {
    /// Every format, in the order of its declaration.
    pub const ALL: &'static [Self] = &[Self::A8, Self::Rgba16, Self::Rgbx8888, Self::Bgrx8888];

    /// How many bytes one pixel of this format occupies.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::A8 => 1,
            Self::Rgba16 => 8,
            Self::Rgbx8888 | Self::Bgrx8888 => 4,
        }
    }

    /// The texel every byte of which is zero.
    #[must_use]
    pub const fn zero(self) -> Texel {
        match self {
            Self::A8 => Texel::Coverage(0),
            Self::Rgba16 => Texel::Linear([0; 4]),
            Self::Rgbx8888 | Self::Bgrx8888 => Texel::Display([0; 3]),
        }
    }

    /// Whether this format stores that texel.
    #[must_use]
    pub const fn carries(self, texel: Texel) -> bool {
        matches!(
            (self, texel),
            (Self::A8, Texel::Coverage(_))
                | (Self::Rgba16, Texel::Linear(_))
                | (Self::Rgbx8888 | Self::Bgrx8888, Texel::Display(_))
        )
    }
}

/// The stored value of one pixel, which is not a colour.
///
/// `Display` names its channels red, green and blue whatever order the format
/// stores them in, so a caller reads one surface the way it reads the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Texel {
    /// Coverage or alpha of an [`Format::A8`] surface.
    Coverage(u8),
    /// Premultiplied linear-light red, green, blue and alpha.
    Linear([u16; 4]),
    /// Display-space red, green and blue of an opaque surface.
    Display([u8; 3]),
}

/// A read-only plane of coverage bytes.
///
/// A [`Format::A8`] surface and a cached glyph both answer as one, so the
/// compositing of R6 has one input shape whether the coverage was just
/// rasterized or came out of the cache of R7.
#[derive(Clone, Copy, Debug)]
pub struct Mask<'a> {
    bytes: &'a [u8],
    width: u32,
    height: u32,
    stride: u32,
}

impl<'a> Mask<'a> {
    /// A mask of this shape over `bytes`.
    /// # Errors
    /// The errors of [`Surface::new`], for the same reasons.
    pub fn new(bytes: &'a [u8], width: u32, height: u32, stride: u32) -> Result<Self, RasterError> {
        if width == 0 || height == 0 {
            return Err(RasterError::Empty);
        }
        if stride < width {
            return Err(RasterError::Stride);
        }
        let needed = u64::from(height).saturating_mul(u64::from(stride));
        let given = bytes.len();
        if u64::try_from(given).unwrap_or(u64::MAX) < needed {
            return Err(RasterError::TooShort {
                needed: usize::try_from(needed).unwrap_or(usize::MAX),
                given,
            });
        }
        Ok(Self {
            bytes,
            width,
            height,
            stride,
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

    /// The coverage at `(x, y)`, or `None` outside the mask.
    #[must_use]
    pub fn coverage(&self, x: u32, y: u32) -> Option<u8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let row = u64::from(y).checked_mul(u64::from(self.stride))?;
        let at = usize::try_from(row.checked_add(u64::from(x))?).ok()?;
        self.bytes.get(at).copied()
    }
}

/// The bytes of a picture, the shape they carry, and bounds-checked access.
#[derive(Debug)]
pub struct Surface<'a> {
    bytes: &'a mut [u8],
    width: u32,
    height: u32,
    stride: u32,
    format: Format,
}

impl<'a> Surface<'a> {
    /// A surface of this shape over `bytes`.
    ///
    /// `stride` counts bytes and not pixels, because the four formats have
    /// four pixel sizes. Bytes past `height * stride`, and the bytes of a row
    /// past its last visible column, belong to the caller and are never
    /// written.
    ///
    /// # Errors
    /// Returns `Empty` for a zero width or height, `Stride` for a stride below
    /// the bytes one row of visible pixels needs, `Overflow` when the shape
    /// leaves `u32` or the address space, and `TooShort` when the slice holds
    /// less than `height * stride`.
    pub fn new(
        bytes: &'a mut [u8],
        width: u32,
        height: u32,
        stride: u32,
        format: Format,
    ) -> Result<Self, RasterError> {
        if width == 0 || height == 0 {
            return Err(RasterError::Empty);
        }
        let row = width
            .checked_mul(format.bytes_per_pixel())
            .ok_or(RasterError::Overflow)?;
        if stride < row {
            return Err(RasterError::Stride);
        }
        let needed = u64::from(height).saturating_mul(u64::from(stride));
        let given = bytes.len();
        if u64::try_from(given).unwrap_or(u64::MAX) < needed {
            return Err(RasterError::TooShort {
                needed: usize::try_from(needed).unwrap_or(usize::MAX),
                given,
            });
        }
        Ok(Self {
            bytes,
            width,
            height,
            stride,
            format,
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

    /// Bytes from the start of one row to the start of the next.
    #[must_use]
    pub const fn stride(&self) -> u32 {
        self.stride
    }

    /// How the bytes of one pixel are stored.
    #[must_use]
    pub const fn format(&self) -> Format {
        self.format
    }

    /// Byte offset of the pixel at `(x, y)`, or `None` outside the surface.
    fn offset(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let row = u64::from(y).checked_mul(u64::from(self.stride))?;
        let column = u64::from(x).checked_mul(u64::from(self.format.bytes_per_pixel()))?;
        usize::try_from(row.checked_add(column)?).ok()
    }

    /// Bytes of one row of visible pixels.
    fn row_bytes(&self) -> Option<usize> {
        usize::try_from(
            u64::from(self.width).checked_mul(u64::from(self.format.bytes_per_pixel()))?,
        )
        .ok()
    }

    /// The visible bytes of row `y`, or `None` beyond the last row.
    #[must_use]
    pub fn row(&self, y: u32) -> Option<&[u8]> {
        let start = self.offset(0, y)?;
        let len = self.row_bytes()?;
        self.bytes.get(start..start.checked_add(len)?)
    }

    /// The visible bytes of row `y`, mutable, or `None` beyond the last row.
    pub fn row_mut(&mut self, y: u32) -> Option<&mut [u8]> {
        let start = self.offset(0, y)?;
        let len = self.row_bytes()?;
        self.bytes.get_mut(start..start.checked_add(len)?)
    }

    /// The texel at `(x, y)`, or `None` outside the surface.
    #[must_use]
    pub fn texel(&self, x: u32, y: u32) -> Option<Texel> {
        let start = self.offset(x, y)?;
        let len = usize::try_from(self.format.bytes_per_pixel()).ok()?;
        let cell = self.bytes.get(start..start.checked_add(len)?)?;
        decode(self.format, cell)
    }

    /// Write `texel` at `(x, y)`.
    ///
    /// # Errors
    /// Returns `Format` when this surface does not store that texel, and
    /// `OutOfBounds` for a position beyond the visible pixels.
    pub fn set_texel(&mut self, x: u32, y: u32, texel: Texel) -> Result<(), RasterError> {
        if !self.format.carries(texel) {
            return Err(RasterError::Format);
        }
        let format = self.format;
        let start = self.offset(x, y).ok_or(RasterError::OutOfBounds)?;
        let len = usize::try_from(format.bytes_per_pixel()).map_err(|_| RasterError::Overflow)?;
        let end = start.checked_add(len).ok_or(RasterError::Overflow)?;
        let cell = self
            .bytes
            .get_mut(start..end)
            .ok_or(RasterError::OutOfBounds)?;
        encode(format, texel, cell);
        Ok(())
    }

    /// Write `texel` into every visible pixel.
    ///
    /// # Errors
    /// Returns `Format` when this surface does not store that texel.
    pub fn fill(&mut self, texel: Texel) -> Result<(), RasterError> {
        if !self.format.carries(texel) {
            return Err(RasterError::Format);
        }
        let format = self.format;
        let len = usize::try_from(format.bytes_per_pixel()).map_err(|_| RasterError::Overflow)?;
        for y in 0..self.height {
            let Some(row) = self.row_mut(y) else { continue };
            for cell in row.chunks_exact_mut(len) {
                encode(format, texel, cell);
            }
        }
        Ok(())
    }

    /// This surface as a read-only coverage plane, or `None` unless it is
    /// [`Format::A8`].
    #[must_use]
    pub fn mask(&self) -> Option<Mask<'_>> {
        (self.format == Format::A8)
            .then(|| Mask::new(self.bytes, self.width, self.height, self.stride).ok())
            .flatten()
    }

    /// Write zero into every byte of every visible pixel.
    pub fn clear(&mut self) {
        for y in 0..self.height {
            if let Some(row) = self.row_mut(y) {
                row.fill(0);
            }
        }
    }
}

/// Read one pixel's bytes as the texel of `format`.
fn decode(format: Format, cell: &[u8]) -> Option<Texel> {
    match format {
        Format::A8 => cell.first().copied().map(Texel::Coverage),
        Format::Rgba16 => {
            let (pairs, _) = cell.as_chunks::<2>();
            let mut channels = [0_u16; 4];
            for (channel, pair) in channels.iter_mut().zip(pairs) {
                *channel = u16::from_le_bytes(*pair);
            }
            Some(Texel::Linear(channels))
        }
        Format::Rgbx8888 => Some(Texel::Display([
            *cell.first()?,
            *cell.get(1)?,
            *cell.get(2)?,
        ])),
        Format::Bgrx8888 => Some(Texel::Display([
            *cell.get(2)?,
            *cell.get(1)?,
            *cell.first()?,
        ])),
    }
}

/// Write `texel` into one pixel's bytes. The caller has checked that `format`
/// carries `texel` and that `cell` is one pixel of it.
fn encode(format: Format, texel: Texel, cell: &mut [u8]) {
    let bytes = match (format, texel) {
        (Format::A8, Texel::Coverage(value)) => {
            if let Some(byte) = cell.first_mut() {
                *byte = value;
            }
            return;
        }
        (Format::Rgba16, Texel::Linear(channels)) => {
            let (pairs, _) = cell.as_chunks_mut::<2>();
            for (channel, pair) in channels.iter().zip(pairs) {
                *pair = channel.to_le_bytes();
            }
            return;
        }
        (Format::Rgbx8888, Texel::Display([r, g, b])) => [r, g, b, 0],
        (Format::Bgrx8888, Texel::Display([r, g, b])) => [b, g, r, 0],
        _ => return,
    };
    for (byte, value) in cell.iter_mut().zip(bytes) {
        *byte = value;
    }
}
