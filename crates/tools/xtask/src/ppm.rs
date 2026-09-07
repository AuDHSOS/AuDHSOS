// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The picture QEMU takes of a screen: a binary PPM.
//!
//! `screendump` writes `P6`, the width and the height, the largest channel
//! value, and then three bytes per pixel. That is all this reader takes,
//! and it takes it strictly: a file whose header says more pixels than the
//! file holds is an error, because a short read would otherwise become a
//! black pixel and a test would pass on a picture that is not there.
//!
//! Invariant: a pixel or a rectangle outside the picture is an error and
//! never a value read from somewhere else in it.

use std::fmt;

/// How many bytes one pixel occupies in the file.
const BYTES_PER_PIXEL: usize = 3;

/// Why a file is no picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PpmError {
    /// It does not begin with `P6`.
    Magic,
    /// A number of the header is missing or is none.
    Header(&'static str),
    /// The largest channel value is not 255.
    MaxValue(u32),
    /// The file holds fewer pixels than the header says.
    Short {
        /// How many bytes the header asks for.
        needed: usize,
        /// How many the file holds.
        given: usize,
    },
    /// The rectangle asked about lies outside the picture.
    Outside,
}

impl fmt::Display for PpmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PpmError::Magic => f.write_str("the file does not begin with P6"),
            PpmError::Header(what) => write!(f, "the header has no {what}"),
            PpmError::MaxValue(value) => write!(f, "the largest channel value is {value}, not 255"),
            PpmError::Short { needed, given } => {
                write!(
                    f,
                    "the picture needs {needed} bytes and the file holds {given}"
                )
            }
            PpmError::Outside => f.write_str("the rectangle lies outside the picture"),
        }
    }
}

/// A picture of a screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Image {
    /// Columns.
    width: u32,
    /// Rows.
    height: u32,
    /// Three bytes per pixel, row by row.
    rgb: Vec<u8>,
}

impl Image {
    /// Reads a binary PPM.
    ///
    /// # Errors
    ///
    /// [`PpmError`] for a file that is none.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Image, PpmError> {
        let mut reader = Header { bytes, at: 0 };
        reader.magic()?;
        let width = reader.number("width")?;
        let height = reader.number("height")?;
        let max = reader.number("largest channel value")?;
        if max != 255 {
            return Err(PpmError::MaxValue(max));
        }
        reader.one_space();
        let needed = usize::try_from(u64::from(width).saturating_mul(u64::from(height)))
            .unwrap_or(usize::MAX)
            .saturating_mul(BYTES_PER_PIXEL);
        let rgb = bytes.get(reader.at..).unwrap_or_default();
        if rgb.len() < needed {
            return Err(PpmError::Short {
                needed,
                given: rgb.len(),
            });
        }
        Ok(Image {
            width,
            height,
            rgb: rgb.get(..needed).unwrap_or_default().to_vec(),
        })
    }

    /// Columns.
    pub(crate) const fn width(&self) -> u32 {
        self.width
    }

    /// Rows.
    pub(crate) const fn height(&self) -> u32 {
        self.height
    }

    /// The color at `x`, `y`.
    ///
    /// # Errors
    ///
    /// [`PpmError::Outside`] for a pixel the picture does not hold.
    pub(crate) fn pixel(&self, x: u32, y: u32) -> Result<(u8, u8, u8), PpmError> {
        if x >= self.width || y >= self.height {
            return Err(PpmError::Outside);
        }
        let index = u64::from(y)
            .saturating_mul(u64::from(self.width))
            .saturating_add(u64::from(x));
        let at = usize::try_from(index)
            .unwrap_or(usize::MAX)
            .saturating_mul(BYTES_PER_PIXEL);
        let end = at.saturating_add(BYTES_PER_PIXEL);
        let bytes = self.rgb.get(at..end).ok_or(PpmError::Outside)?;
        match bytes {
            [red, green, blue] => Ok((*red, *green, *blue)),
            _other => Err(PpmError::Outside),
        }
    }

    /// How many pixels of the rectangle carry `color`.
    ///
    /// # Errors
    ///
    /// [`PpmError::Outside`] for a rectangle that reaches past the picture.
    pub(crate) fn count_of(
        &self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        color: (u8, u8, u8),
    ) -> Result<u32, PpmError> {
        if x.saturating_add(width) > self.width || y.saturating_add(height) > self.height {
            return Err(PpmError::Outside);
        }
        let mut count = 0_u32;
        for row in 0..height {
            for column in 0..width {
                if self.pixel(x.saturating_add(column), y.saturating_add(row))? == color {
                    count = count.saturating_add(1);
                }
            }
        }
        Ok(count)
    }
}

/// Where the reader stands in the header.
struct Header<'a> {
    /// The bytes of the file.
    bytes: &'a [u8],
    /// The byte read next.
    at: usize,
}

impl Header<'_> {
    /// Steps over the magic.
    fn magic(&mut self) -> Result<(), PpmError> {
        if self.bytes.get(..2) != Some(b"P6") {
            return Err(PpmError::Magic);
        }
        self.at = 2;
        Ok(())
    }

    /// Steps over spaces and comments.
    fn spaces(&mut self) {
        loop {
            match self.bytes.get(self.at) {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.at = self.at.saturating_add(1),
                Some(b'#') => {
                    while !matches!(self.bytes.get(self.at), Some(b'\n') | None) {
                        self.at = self.at.saturating_add(1);
                    }
                }
                _other => return,
            }
        }
    }

    /// Steps over the one byte of space after the last header number.
    fn one_space(&mut self) {
        if matches!(self.bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at = self.at.saturating_add(1);
        }
    }

    /// Reads one number of the header.
    fn number(&mut self, what: &'static str) -> Result<u32, PpmError> {
        self.spaces();
        let start = self.at;
        while matches!(self.bytes.get(self.at), Some(b'0'..=b'9')) {
            self.at = self.at.saturating_add(1);
        }
        let digits = self.bytes.get(start..self.at).unwrap_or_default();
        let text = std::str::from_utf8(digits).map_err(|_| PpmError::Header(what))?;
        text.parse().map_err(|_| PpmError::Header(what))
    }
}
