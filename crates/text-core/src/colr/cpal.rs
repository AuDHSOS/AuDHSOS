// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Colour records and palettes; `docs/microsoft/cpal.html:740`.

use crate::{
    Fixed, FontError,
    read::{self, add, mul},
};

/// Maximum palettes one CPAL table may declare.
pub const MAX_PALETTES: u16 = 1024;
/// The palette entry index that names the caller's text colour.
pub const FOREGROUND: u16 = 0xffff;

/// Where one colour comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorSource {
    /// An sRGB palette entry, not premultiplied.
    Palette {
        /// Red level.
        red: u8,
        /// Green level.
        green: u8,
        /// Blue level.
        blue: u8,
    },
    /// The text colour, which this crate reads from no setting.
    #[default]
    Foreground,
}

/// One resolved colour reference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color {
    /// The palette entry, or the caller's text colour.
    pub source: ColorSource,
    /// The palette record's alpha times the paint table's alpha, in `[0, 1]`.
    pub alpha: Fixed,
}

/// Which background a palette was drawn for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaletteUse {
    /// The palette suits a light background.
    pub light: bool,
    /// The palette suits a dark background.
    pub dark: bool,
}

/// The background a palette is chosen against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    /// A light background, such as white.
    Light,
    /// A dark background, such as black.
    Dark,
}

/// A validated CPAL table.
#[derive(Clone, Copy, Debug)]
pub struct Cpal<'a> {
    indices: &'a [u8],
    records: &'a [u8],
    types: Option<&'a [u8]>,
    entries: u16,
    palettes: u16,
}

impl<'a> Cpal<'a> {
    /// Validate a borrowed CPAL table and every palette's record range.
    /// # Errors
    /// Rejects versions above one, empty palettes, more than
    /// [`MAX_PALETTES`] palettes, and ranges outside the record array.
    pub fn parse(data: &'a [u8]) -> Result<Self, FontError> {
        let version = read::u16(data, 0)?;
        if version > 1 {
            return Err(FontError::UnsupportedFormat);
        }
        let entries = read::u16(data, 2)?;
        let palettes = read::u16(data, 4)?;
        let count = usize::from(read::u16(data, 6)?);
        if entries == 0 || palettes == 0 {
            return Err(FontError::InvalidTable);
        }
        if palettes > MAX_PALETTES {
            return Err(FontError::LimitExceeded);
        }
        let indices = read::bytes(data, 12, mul(usize::from(palettes), 2)?)?;
        let records = read::bytes(data, read::offset(read::u32(data, 8)?)?, mul(count, 4)?)?;
        for pair in indices.as_chunks::<2>().0 {
            let first = usize::from(read::u16(pair, 0)?);
            if add(first, usize::from(entries))? > count {
                return Err(FontError::InvalidTable);
            }
        }
        let types = if version == 1 {
            let header = add(12, indices.len())?;
            // The two label arrays name `name` table strings, which layout
            // never reads; their ranges are checked and nothing else.
            for (field, count) in [(add(header, 4)?, palettes), (add(header, 8)?, entries)] {
                let at = read::offset(read::u32(data, field)?)?;
                if at != 0 {
                    read::bytes(data, at, mul(usize::from(count), 2)?)?;
                }
            }
            match read::offset(read::u32(data, header)?)? {
                0 => None,
                at => Some(read::bytes(data, at, mul(usize::from(palettes), 4)?)?),
            }
        } else {
            None
        };
        Ok(Self {
            indices,
            records,
            types,
            entries,
            palettes,
        })
    }

    /// Number of palettes.
    #[must_use]
    pub const fn palette_count(self) -> u16 {
        self.palettes
    }

    /// Number of entries in each palette.
    #[must_use]
    pub const fn entry_count(self) -> u16 {
        self.entries
    }

    /// The colour of one entry, with `alpha` multiplied into the record's own.
    ///
    /// Entry [`FOREGROUND`] names the caller's text colour and carries the
    /// paint table's alpha alone.
    /// # Errors
    /// Rejects a palette or entry index outside the table.
    pub fn color(self, palette: u16, entry: u16, alpha: Fixed) -> Result<Color, FontError> {
        let alpha = alpha.clamp(Fixed::ZERO, Fixed::ONE);
        if entry == FOREGROUND {
            return Ok(Color {
                source: ColorSource::Foreground,
                alpha,
            });
        }
        if palette >= self.palettes || entry >= self.entries {
            return Err(FontError::InvalidTable);
        }
        let first = usize::from(read::u16(self.indices, mul(usize::from(palette), 2)?)?);
        let at = mul(add(first, usize::from(entry))?, 4)?;
        let record = read::bytes(self.records, at, 4)?;
        Ok(Color {
            source: ColorSource::Palette {
                red: read::u8(record, 2)?,
                green: read::u8(record, 1)?,
                blue: read::u8(record, 0)?,
            },
            alpha: alpha.mul_ratio(i64::from(read::u8(record, 3)?), 255)?,
        })
    }

    /// The backgrounds one palette was drawn for.
    ///
    /// A table without a palette types array declares no preference, which
    /// this reports as suiting either background.
    /// # Errors
    /// Rejects a palette index outside the table.
    pub fn usage(self, palette: u16) -> Result<PaletteUse, FontError> {
        if palette >= self.palettes {
            return Err(FontError::InvalidTable);
        }
        let Some(types) = self.types else {
            return Ok(PaletteUse {
                light: true,
                dark: true,
            });
        };
        let flags = read::u32(types, mul(usize::from(palette), 4)?)?;
        Ok(PaletteUse {
            light: flags & 1 != 0,
            dark: flags & 2 != 0,
        })
    }

    /// The first palette declared for `background`, else palette zero.
    /// # Errors
    /// Rejects a malformed palette types array.
    pub fn palette_for(self, background: Background) -> Result<u16, FontError> {
        for palette in 0..self.palettes {
            let usage = self.usage(palette)?;
            if match background {
                Background::Light => usage.light,
                Background::Dark => usage.dark,
            } {
                return Ok(palette);
            }
        }
        Ok(0)
    }
}
