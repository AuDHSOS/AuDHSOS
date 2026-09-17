// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! OpenType head, hhea, hmtx, maxp, OS/2, and post tables.

use crate::{
    Fixed, Font, FontError, OutlineKind,
    cmap::Cmap,
    read::{self, add, mul},
};

/// Unscaled vertical metrics; descent uses signed font coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineMetrics {
    /// Distance above the alphabetic baseline.
    pub ascender: i16,
    /// Distance below the baseline, normally negative.
    pub descender: i16,
    /// Nonnegative extra space between lines.
    pub gap: i16,
}

impl LineMetrics {
    fn read(data: &[u8], at: usize) -> Result<Self, FontError> {
        Ok(Self {
            ascender: read::i16(data, at)?,
            descender: read::i16(data, add(at, 2)?)?,
            gap: read::i16(data, add(at, 4)?)?.max(0),
        })
    }
}

/// The validated head table fields needed by pure text processing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    /// Design units per em, from 16 through 16384.
    pub units_per_em: u16,
    /// Bounding box in design units: xMin, yMin, xMax, yMax.
    pub bounds: [i16; 4],
    /// Whether loca entries use 32-bit byte offsets.
    pub long_loca: bool,
}

impl Head {
    /// Parse a head table.
    /// # Errors
    /// Rejects truncation, versions, magic, units, bounds, and invalid formats.
    pub fn parse(data: &[u8]) -> Result<Self, FontError> {
        read::bytes(data, 0, 54)?;
        if read::u32(data, 0)? != 0x0001_0000
            || read::u32(data, 12)? != 0x5f0f_3cf5
            || read::i16(data, 52)? != 0
        {
            return Err(FontError::InvalidTable);
        }
        let units_per_em = read::u16(data, 18)?;
        let bounds = [
            read::i16(data, 36)?,
            read::i16(data, 38)?,
            read::i16(data, 40)?,
            read::i16(data, 42)?,
        ];
        if !(16..=16384).contains(&units_per_em) || bounds[0] > bounds[2] || bounds[1] > bounds[3] {
            return Err(FontError::InvalidTable);
        }
        let long_loca = match read::i16(data, 50)? {
            0 => false,
            1 => true,
            _ => return Err(FontError::InvalidTable),
        };
        Ok(Self {
            units_per_em,
            bounds,
            long_loca,
        })
    }
}

/// Maximum profile; declared maxima do not authorize allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Maxp {
    /// Number of glyphs including glyph zero.
    pub glyph_count: u16,
}

impl Maxp {
    /// Parse maxp and check compatibility with the outline kind.
    /// # Errors
    /// Rejects unsupported versions, empty glyph sets, and truncated profiles.
    pub fn parse(data: &[u8], kind: OutlineKind) -> Result<Self, FontError> {
        match (kind, read::u32(data, 0)?) {
            (OutlineKind::TrueType, 0x0001_0000) => {
                read::bytes(data, 0, 32)?;
                if !(1..=2).contains(&read::u16(data, 14)?) {
                    return Err(FontError::InvalidTable);
                }
            }
            (OutlineKind::PostScript, 0x0000_5000) => {
                read::bytes(data, 0, 6)?;
            }
            _ => return Err(FontError::UnsupportedFormat),
        }
        let glyph_count = read::u16(data, 4)?;
        if glyph_count == 0 {
            return Err(FontError::InvalidTable);
        }
        Ok(Self { glyph_count })
    }
}

/// OS/2 metrics used by layout and font selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Os2 {
    /// Declared weight, 1 through 1000.
    pub weight: u16,
    /// Typographic line metrics, absent in legacy 68-byte version zero.
    pub typo: Option<LineMetrics>,
    /// Whether the font requests typographic line metrics.
    pub use_typo: bool,
    /// Strikeout thickness and position in font units.
    pub strikeout: [i16; 2],
    /// Approximate x height, zero when unspecified.
    pub x_height: i16,
    /// Cap height, zero when unspecified.
    pub cap_height: i16,
}

impl Os2 {
    /// Parse OS/2 versions 0 through 5.
    /// # Errors
    /// Rejects unknown versions, short tables, and invalid weight/width classes.
    pub fn parse(data: &[u8]) -> Result<Self, FontError> {
        let version = read::u16(data, 0)?;
        let len = match version {
            0 if data.len() == 68 => 68,
            0 => 78,
            1 => 86,
            2..=4 => 96,
            5 => 100,
            _ => return Err(FontError::UnsupportedFormat),
        };
        read::bytes(data, 0, len)?;
        let weight = read::u16(data, 4)?;
        if !(1..=1000).contains(&weight) || !(1..=9).contains(&read::u16(data, 6)?) {
            return Err(FontError::InvalidTable);
        }
        Ok(Self {
            weight,
            typo: if len >= 78 {
                Some(LineMetrics::read(data, 68)?)
            } else {
                None
            },
            use_typo: read::u16(data, 62)? & 0x80 != 0,
            strikeout: [read::i16(data, 26)?, read::i16(data, 28)?],
            x_height: if version >= 2 {
                read::i16(data, 86)?
            } else {
                0
            },
            cap_height: if version >= 2 {
                read::i16(data, 88)?
            } else {
                0
            },
        })
    }
}

/// Unhinted PostScript metrics. Glyph names do not affect layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Post {
    /// Counter-clockwise italic angle in degrees.
    pub italic_angle: Fixed,
    /// Top of the underline in font units.
    pub underline_position: i16,
    /// Underline thickness in font units.
    pub underline_thickness: i16,
    /// Whether the font declares equal character advances.
    pub fixed_pitch: bool,
}

impl Post {
    /// Parse post versions 1, 2, 2.5, and 3, including glyph-name bounds.
    /// # Errors
    /// Rejects unknown versions, invalid names/counts, and truncated data.
    pub fn parse(data: &[u8], glyphs: u16) -> Result<Self, FontError> {
        read::bytes(data, 0, 32)?;
        match read::u32(data, 0)? {
            0x0001_0000 if glyphs == 258 => (),
            0x0003_0000 => (),
            0x0002_0000 => validate_names(data, glyphs)?,
            0x0002_5000 => {
                if read::u16(data, 32)? != glyphs {
                    return Err(FontError::InvalidTable);
                }
                for (id, delta) in read::bytes(data, 34, usize::from(glyphs))?
                    .iter()
                    .enumerate()
                {
                    let id = i32::try_from(id).map_err(|_| FontError::Overflow)?;
                    let mapped = id
                        .checked_add(i32::from(i8::from_be_bytes([*delta])))
                        .ok_or(FontError::Overflow)?;
                    if !(0..258).contains(&mapped) {
                        return Err(FontError::InvalidTable);
                    }
                }
            }
            _ => return Err(FontError::UnsupportedFormat),
        }
        Ok(Self {
            italic_angle: Fixed::from_16_16(read::i32(data, 4)?),
            underline_position: read::i16(data, 8)?,
            underline_thickness: read::i16(data, 10)?,
            fixed_pitch: read::u32(data, 12)? != 0,
        })
    }
}

fn validate_names(data: &[u8], glyphs: u16) -> Result<(), FontError> {
    if read::u16(data, 32)? != glyphs {
        return Err(FontError::InvalidTable);
    }
    let indices = read::bytes(data, 34, mul(usize::from(glyphs), 2)?)?;
    let mut max = 257;
    for pair in indices.as_chunks::<2>().0 {
        max = max.max(read::u16(pair, 0)?);
    }
    let mut at = add(34, indices.len())?;
    for _ in 258..=max {
        let len = usize::from(read::u8(data, at)?);
        at = add(at, 1)?;
        let name = read::bytes(data, at, len)?;
        if len == 0
            || len > 63
            || !name
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || matches!(*b, b'.' | b'_'))
        {
            return Err(FontError::InvalidTable);
        }
        at = add(at, len)?;
    }
    Ok(())
}

/// Validated horizontal metrics borrowing the font's hmtx table.
#[derive(Clone, Copy, Debug)]
pub struct Metrics<'a> {
    /// Font-wide header.
    pub head: Head,
    /// Glyph count.
    pub maxp: Maxp,
    /// OS/2 metrics if present.
    pub os2: Option<Os2>,
    /// PostScript metrics if present.
    pub post: Option<Post>,
    /// Selected line metrics in design units.
    pub line: LineMetrics,
    hmtx: &'a [u8],
    count: u16,
}

impl<'a> Metrics<'a> {
    /// Validate the metric tables of a face.
    /// # Errors
    /// Returns missing-table, table-validation, or count/bounds errors.
    pub fn parse(font: &Font<'a>) -> Result<Self, FontError> {
        let head = Head::parse(font.required(*b"head")?)?;
        let maxp = Maxp::parse(font.required(*b"maxp")?, font.outline_kind())?;
        let hhea = font.required(*b"hhea")?;
        read::bytes(hhea, 0, 36)?;
        if read::u32(hhea, 0)? != 0x0001_0000 || read::bytes(hhea, 24, 10)?.iter().any(|b| *b != 0)
        {
            return Err(FontError::InvalidTable);
        }
        let count = read::u16(hhea, 34)?;
        if count == 0 || count > maxp.glyph_count {
            return Err(FontError::InvalidTable);
        }
        let hmtx = font.required(*b"hmtx")?;
        read::bytes(
            hmtx,
            0,
            add(
                mul(usize::from(count), 2)?,
                mul(usize::from(maxp.glyph_count), 2)?,
            )?,
        )?;
        let os2 = font
            .table(*b"OS/2")
            .map(|t| Os2::parse(t.data))
            .transpose()?;
        let post = font
            .table(*b"post")
            .map(|t| Post::parse(t.data, maxp.glyph_count))
            .transpose()?;
        let line = os2
            .filter(|os| os.use_typo)
            .and_then(|os| os.typo)
            .unwrap_or(LineMetrics::read(hhea, 4)?);
        Ok(Self {
            head,
            maxp,
            os2,
            post,
            line,
            hmtx,
            count,
        })
    }

    /// Read the unscaled advance and left side bearing of a glyph.
    /// # Errors
    /// Returns `GlyphIndex` for an out-of-range glyph.
    pub fn horizontal(&self, glyph: u16) -> Result<(u16, i16), FontError> {
        if glyph >= self.maxp.glyph_count {
            return Err(FontError::GlyphIndex);
        }
        let count = usize::from(self.count);
        let id = usize::from(glyph);
        let advance_at = mul(id.min(read::sub(count, 1)?), 4)?;
        let bearing_at = if id < count {
            add(advance_at, 2)?
        } else {
            add(mul(count, 4)?, mul(read::sub(id, count)?, 2)?)?
        };
        Ok((
            read::u16(self.hmtx, advance_at)?,
            read::i16(self.hmtx, bearing_at)?,
        ))
    }

    /// Scale design units to the requested em size with one rounding.
    /// # Errors
    /// Returns `Overflow` if the scaled value exceeds Q32.32.
    pub fn scale(&self, units: i32, size: Fixed) -> Result<Fixed, FontError> {
        size.mul_ratio(i64::from(units), i64::from(self.head.units_per_em))
    }

    /// Read a fractional glyph advance at the requested size.
    /// # Errors
    /// Returns a glyph-index or arithmetic error.
    pub fn advance(&self, glyph: u16, size: Fixed) -> Result<Fixed, FontError> {
        self.scale(i32::from(self.horizontal(glyph)?.0), size)
    }
}

impl<'a> Font<'a> {
    pub(crate) fn required(&self, tag: [u8; 4]) -> Result<&'a [u8], FontError> {
        self.table(tag)
            .map(|t| t.data)
            .ok_or(FontError::MissingTable)
    }

    /// Validate and borrow the metric tables.
    /// # Errors
    /// Returns a missing-table or metric-validation error.
    pub fn metrics(&self) -> Result<Metrics<'a>, FontError> {
        Metrics::parse(self)
    }

    /// Validate and borrow the selected Unicode character map.
    /// # Errors
    /// Returns a maxp or cmap validation error.
    pub fn cmap(&self) -> Result<Cmap<'a>, FontError> {
        let maxp = Maxp::parse(self.required(*b"maxp")?, self.outline_kind())?;
        Cmap::parse(self.required(*b"cmap")?, maxp.glyph_count)
    }
}
