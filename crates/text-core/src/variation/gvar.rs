// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use crate::{
    Fixed, FontError,
    glyf::Point,
    read::{self, add, mul, sub},
};

/// Caller-owned workspace for one point's original position and variation deltas.
#[derive(Clone, Copy, Debug, Default)]
pub struct VariationPoint {
    pub(crate) original: Point,
    pub(crate) dx: Fixed,
    pub(crate) dy: Fixed,
    tx: Fixed,
    ty: Fixed,
    touched: bool,
    shared: bool,
    repeats: usize,
    shared_repeats: usize,
}
impl VariationPoint {
    pub(crate) fn result(self) -> Result<Point, FontError> {
        Ok(Point::new(
            self.original.x.checked_add(self.dx)?,
            self.original.y.checked_add(self.dy)?,
            self.original.on_curve,
        ))
    }
}

/// A validated gvar table with borrowed tuple and glyph offset arrays.
#[derive(Clone, Copy, Debug)]
pub struct Gvar<'a> {
    data: &'a [u8],
    offsets: &'a [u8],
    shared: &'a [u8],
    axes: usize,
    glyphs: u16,
    long: bool,
}
impl<'a> Gvar<'a> {
    /// Parse gvar using counts from fvar and maxp.
    /// # Errors
    /// Rejects malformed offsets/counts, tuple coordinates, or table versions.
    pub fn parse(data: &'a [u8], axes: usize, glyphs: u16) -> Result<Self, FontError> {
        if read::u32(data, 0)? != 0x0001_0000 {
            return Err(FontError::UnsupportedFormat);
        }
        if usize::from(read::u16(data, 4)?) != axes
            || read::u16(data, 12)? != glyphs
            || read::u16(data, 14)? > 1
        {
            return Err(FontError::InvalidTable);
        }
        let count = usize::from(read::u16(data, 6)?);
        if axes > 64 || count > 4096 {
            return Err(FontError::LimitExceeded);
        }
        let long = read::u16(data, 14)? != 0;
        let offsets = read::bytes(
            data,
            20,
            mul(add(usize::from(glyphs), 1)?, if long { 4 } else { 2 })?,
        )?;
        let start = read::offset(read::u32(data, 16)?)?;
        if start < add(20, offsets.len())? {
            return Err(FontError::Overlap);
        }
        let shared = read::bytes(
            data,
            read::offset(read::u32(data, 8)?)?,
            mul(mul(axes, count)?, 2)?,
        )?;
        for pair in shared.as_chunks::<2>().0 {
            coordinate(read::i16(pair, 0)?)?;
        }
        let result = Self {
            data: read::tail(data, start)?,
            offsets,
            shared,
            axes,
            glyphs,
            long,
        };
        let mut previous = 0;
        for id in 0..=usize::from(glyphs) {
            let next = result.offset(id)?;
            if next < previous || next > result.data.len() {
                return Err(FontError::InvalidTable);
            }
            previous = next;
        }
        Ok(result)
    }
    fn offset(self, id: usize) -> Result<usize, FontError> {
        if self.long {
            read::offset(read::u32(self.offsets, mul(id, 4)?)?)
        } else {
            mul(usize::from(read::u16(self.offsets, mul(id, 2)?)?), 2)
        }
    }
    fn glyph(self, id: u16) -> Result<&'a [u8], FontError> {
        if id >= self.glyphs {
            return Err(FontError::GlyphIndex);
        }
        let start = self.offset(usize::from(id))?;
        read::bytes(
            self.data,
            start,
            sub(self.offset(add(usize::from(id), 1)?)?, start)?,
        )
    }

    /// Apply glyph deltas to points followed by four phantom points.
    ///
    /// Contour endpoints exclude phantoms; pass no contours for component offsets.
    /// Original coordinates are copied into caller workspace before interpolation.
    /// # Errors
    /// Returns malformed tuple/delta, coordinate, arithmetic, or buffer errors.
    pub fn apply(
        self,
        glyph: u16,
        coords: &[Fixed],
        points: &mut [Point],
        contours: &[usize],
        workspace: &mut [VariationPoint],
    ) -> Result<(), FontError> {
        let work = workspace
            .get_mut(..points.len())
            .ok_or(FontError::BufferTooSmall)?;
        for (point, slot) in points.iter().zip(work.iter_mut()) {
            *slot = VariationPoint {
                original: *point,
                ..VariationPoint::default()
            };
        }
        self.evaluate(glyph, coords, contours, work)?;
        for (point, slot) in points.iter_mut().zip(work.iter()) {
            *point = slot.result()?;
        }
        Ok(())
    }

    fn validate_input(
        self,
        coords: &[Fixed],
        contours: &[usize],
        work: &[VariationPoint],
    ) -> Result<(), FontError> {
        if coords.len() != self.axes
            || coords
                .iter()
                .any(|v| *v < Fixed::from_i32(-1) || *v > Fixed::ONE)
            || work.len() < 4
        {
            return Err(FontError::InvalidTable);
        }
        let mut prior = 0;
        for end in contours {
            if *end < prior || *end >= sub(work.len(), 4)? {
                return Err(FontError::InvalidTable);
            }
            prior = add(*end, 1)?;
        }
        Ok(())
    }

    pub(crate) fn evaluate(
        self,
        glyph: u16,
        coords: &[Fixed],
        contours: &[usize],
        work: &mut [VariationPoint],
    ) -> Result<(), FontError> {
        self.validate_input(coords, contours, work)?;
        let data = self.glyph(glyph)?;
        if data.is_empty() {
            return Ok(());
        }
        let flags = read::u16(data, 0)?;
        if flags & 0x7000 != 0 {
            return Err(FontError::InvalidTable);
        }
        let count = usize::from(flags & 0x0fff);
        if mul(count, work.len())? > 16 * 1024 * 1024 {
            return Err(FontError::LimitExceeded);
        }
        let payload = usize::from(read::u16(data, 2)?);
        let mut at = payload;
        if flags & 0x8000 != 0 {
            let mut c = Cursor { data, at };
            points(&mut c, work)?;
            at = c.at;
            for p in work.iter_mut() {
                p.shared = p.touched;
                p.shared_repeats = p.repeats;
            }
        } else {
            for p in work.iter_mut() {
                p.shared = true;
                p.shared_repeats = 1;
            }
        }
        let mut header = Cursor {
            data: read::bytes(data, 0, payload)?,
            at: 4,
        };
        for _ in 0..count {
            let size = usize::from(header.u16()?);
            let flags = header.u16()?;
            if flags & 0x1000 != 0 {
                return Err(FontError::InvalidTable);
            }
            let peak = if flags & 0x8000 != 0 {
                header.take(mul(self.axes, 2)?)?
            } else {
                read::bytes(
                    self.shared,
                    mul(usize::from(flags & 0x0fff), mul(self.axes, 2)?)?,
                    mul(self.axes, 2)?,
                )?
            };
            let intermediate = if flags & 0x4000 != 0 {
                Some((
                    header.take(mul(self.axes, 2)?)?,
                    header.take(mul(self.axes, 2)?)?,
                ))
            } else {
                None
            };
            let scalar = tuple_scalar(coords, peak, intermediate)?;
            let tuple = read::bytes(data, at, size)?;
            at = add(at, size)?;
            for p in work.iter_mut() {
                p.tx = Fixed::ZERO;
                p.ty = Fixed::ZERO;
                p.touched = p.shared;
                p.repeats = p.shared_repeats;
            }
            let mut c = Cursor { data: tuple, at: 0 };
            if flags & 0x2000 != 0 {
                points(&mut c, work)?;
            }
            deltas(&mut c, work, true)?;
            deltas(&mut c, work, false)?;
            if c.at != tuple.len() {
                return Err(FontError::InvalidTable);
            }
            interpolate(work, contours)?;
            for p in work.iter_mut() {
                p.dx = p.dx.checked_add(p.tx.checked_mul(scalar)?)?;
                p.dy = p.dy.checked_add(p.ty.checked_mul(scalar)?)?;
            }
        }
        // Phantom directions represent advances and side bearings independently.
        let phantom = sub(work.len(), 4)?;
        for (i, p) in work
            .get_mut(phantom..)
            .ok_or(FontError::InvalidTable)?
            .iter_mut()
            .enumerate()
        {
            if i < 2 {
                p.dy = Fixed::ZERO;
            } else {
                p.dx = Fixed::ZERO;
            }
        }
        Ok(())
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], FontError> {
        let bytes = read::bytes(self.data, self.at, n)?;
        self.at = add(self.at, n)?;
        Ok(bytes)
    }
    fn byte(&mut self) -> Result<u8, FontError> {
        read::u8(self.take(1)?, 0)
    }
    fn u16(&mut self) -> Result<u16, FontError> {
        read::u16(self.take(2)?, 0)
    }
}

fn coordinate(value: i16) -> Result<Fixed, FontError> {
    if (-16384..=16384).contains(&value) {
        Ok(Fixed::from_2_14(value))
    } else {
        Err(FontError::InvalidTable)
    }
}
fn tuple_scalar(
    coords: &[Fixed],
    peak: &[u8],
    region: Option<(&[u8], &[u8])>,
) -> Result<Fixed, FontError> {
    let mut scalar = Fixed::ONE;
    for (axis, current) in coords.iter().copied().enumerate() {
        let at = mul(axis, 2)?;
        let peak = coordinate(read::i16(peak, at)?)?;
        let bounds = region
            .map(|(start, end)| {
                Ok::<_, FontError>((
                    coordinate(read::i16(start, at)?)?,
                    coordinate(read::i16(end, at)?)?,
                ))
            })
            .transpose()?;
        if peak == Fixed::ZERO {
            continue;
        }
        let factor = if let Some((start, end)) = bounds {
            if start > peak || peak > end {
                return Err(FontError::InvalidTable);
            }
            if current == peak {
                Fixed::ONE
            } else if current <= start || current >= end {
                Fixed::ZERO
            } else if current < peak {
                current
                    .checked_sub(start)?
                    .checked_div(peak.checked_sub(start)?)?
            } else {
                end.checked_sub(current)?
                    .checked_div(end.checked_sub(peak)?)?
            }
        } else if current == Fixed::ZERO || (current < Fixed::ZERO) != (peak < Fixed::ZERO) {
            Fixed::ZERO
        } else if current.bits().unsigned_abs() >= peak.bits().unsigned_abs() {
            Fixed::ONE
        } else {
            current.checked_div(peak)?
        };
        scalar = scalar.checked_mul(factor)?;
    }
    Ok(scalar)
}

fn points(c: &mut Cursor<'_>, work: &mut [VariationPoint]) -> Result<(), FontError> {
    let first = c.byte()?;
    let count = if first & 0x80 != 0 {
        add(mul(usize::from(first & 0x7f), 256)?, usize::from(c.byte()?))?
    } else {
        usize::from(first)
    };
    for p in work.iter_mut() {
        p.touched = count == 0;
        p.repeats = usize::from(count == 0);
    }
    if count == 0 {
        return Ok(());
    }
    let mut done = 0;
    let mut point = 0;
    while done < count {
        let control = c.byte()?;
        let n = add(usize::from(control & 0x7f), 1)?;
        if add(done, n)? > count {
            return Err(FontError::InvalidTable);
        }
        for _ in 0..n {
            let delta = if control & 0x80 != 0 {
                usize::from(c.u16()?)
            } else {
                usize::from(c.byte()?)
            };
            point = add(point, delta)?;
            let slot = work.get_mut(point).ok_or(FontError::InvalidTable)?;
            slot.touched = true;
            slot.repeats = add(slot.repeats, 1)?;
            done = add(done, 1)?;
        }
    }
    Ok(())
}

fn deltas(
    c: &mut Cursor<'_>,
    work: &mut [VariationPoint],
    horizontal: bool,
) -> Result<(), FontError> {
    let mut remaining = 0;
    let mut control = 0;
    for p in work.iter_mut().filter(|p| p.touched) {
        for _ in 0..p.repeats {
            if remaining == 0 {
                control = c.byte()?;
                remaining = add(usize::from(control & 0x3f), 1)?;
            }
            let value = if control & 0x80 != 0 {
                0
            } else if control & 0x40 != 0 {
                i32::from(i16::from_be_bytes(c.u16()?.to_be_bytes()))
            } else {
                i32::from(i8::from_be_bytes([c.byte()?]))
            };
            if horizontal {
                p.tx = p.tx.checked_add(Fixed::from_i32(value))?;
            } else {
                p.ty = p.ty.checked_add(Fixed::from_i32(value))?;
            }
            remaining = sub(remaining, 1)?;
        }
    }
    if remaining != 0 {
        return Err(FontError::InvalidTable);
    }
    Ok(())
}

fn interpolate(work: &mut [VariationPoint], contours: &[usize]) -> Result<(), FontError> {
    let mut start = 0;
    for end in contours.iter().copied() {
        let contour = work
            .get_mut(start..add(end, 1)?)
            .ok_or(FontError::InvalidTable)?;
        let Some(first) = contour.iter().position(|p| p.touched) else {
            start = add(end, 1)?;
            continue;
        };
        let mut before = first;
        for after in (add(first, 1)?..contour.len()).chain(0..=first) {
            if !contour.get(after).ok_or(FontError::InvalidTable)?.touched {
                continue;
            }
            let a = *contour.get(before).ok_or(FontError::InvalidTable)?;
            let b = *contour.get(after).ok_or(FontError::InvalidTable)?;
            let mut target = add(before, 1)?
                .checked_rem(contour.len())
                .ok_or(FontError::InvalidTable)?;
            while target != after {
                let p = contour.get_mut(target).ok_or(FontError::InvalidTable)?;
                p.tx = infer(p.original.x, a.original.x, b.original.x, a.tx, b.tx)?;
                p.ty = infer(p.original.y, a.original.y, b.original.y, a.ty, b.ty)?;
                target = add(target, 1)?
                    .checked_rem(contour.len())
                    .ok_or(FontError::InvalidTable)?;
            }
            before = after;
        }
        start = add(end, 1)?;
    }
    Ok(())
}
fn infer(
    value: Fixed,
    mut a: Fixed,
    mut b: Fixed,
    mut da: Fixed,
    mut db: Fixed,
) -> Result<Fixed, FontError> {
    if a == b {
        return Ok(if da == db { da } else { Fixed::ZERO });
    }
    if a > b {
        core::mem::swap(&mut a, &mut b);
        core::mem::swap(&mut da, &mut db);
    }
    if value <= a {
        return Ok(da);
    }
    if value >= b {
        return Ok(db);
    }
    da.checked_add(
        db.checked_sub(da)?
            .checked_mul(value.checked_sub(a)?)?
            .checked_div(b.checked_sub(a)?)?,
    )
}
