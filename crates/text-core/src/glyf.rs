// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Unhinted quadratic outlines; OpenType glyf and loca tables.

use crate::{
    Fixed, Font, FontError, OutlineKind,
    metrics::Metrics,
    read::{self, add, mul},
    variation::{Axes, Gvar, VariationPoint},
};

/// Maximum simultaneously active glyph descriptions.
pub const MAX_DEPTH: usize = 32;
/// Maximum glyph visits for one decoded outline.
pub const MAX_COMPONENTS: usize = 1024;

/// A font-unit control point, including its quadratic on-curve flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: Fixed,
    /// Vertical coordinate, positive upward.
    pub y: Fixed,
    /// Whether this point lies on the contour.
    pub on_curve: bool,
    flags: u8,
}

impl Point {
    /// Construct a point with exact coordinates.
    #[must_use]
    pub const fn new(x: Fixed, y: Fixed, on_curve: bool) -> Self {
        Self {
            x,
            y,
            on_curve,
            flags: 0,
        }
    }

    fn translate(self, x: Fixed, y: Fixed) -> Result<Self, FontError> {
        Ok(Self::new(
            self.x.checked_add(x)?,
            self.y.checked_add(y)?,
            self.on_curve,
        ))
    }
}

/// Used lengths of caller-owned buffers and the unhinted phantom points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outline {
    /// Number of points written.
    pub points: usize,
    /// Number of contour endpoints written.
    pub contours: usize,
    /// Left, right, top, and bottom phantom points; not contour points.
    pub phantoms: [Point; 4],
}

/// Validated loca offsets with lazily decoded glyph records.
#[derive(Clone, Copy, Debug)]
pub struct Glyf<'a> {
    font: Font<'a>,
    metrics: Metrics<'a>,
    loca: &'a [u8],
    data: &'a [u8],
}

impl<'a> Glyf<'a> {
    /// Validate loca and the metric tables of a TrueType face.
    /// # Errors
    /// Returns format, missing-table, offset-order, or bounds errors.
    pub fn parse(font: &Font<'a>) -> Result<Self, FontError> {
        if font.outline_kind() != OutlineKind::TrueType {
            return Err(FontError::UnsupportedFormat);
        }
        let metrics = font.metrics()?;
        let result = Self {
            font: *font,
            metrics,
            loca: font.required(*b"loca")?,
            data: font.required(*b"glyf")?,
        };
        read::bytes(
            result.loca,
            0,
            mul(
                add(usize::from(metrics.maxp.glyph_count), 1)?,
                if metrics.head.long_loca { 4 } else { 2 },
            )?,
        )?;
        let mut previous = 0;
        for id in 0..=usize::from(metrics.maxp.glyph_count) {
            let next = result.offset(id)?;
            if next < previous || next > result.data.len() {
                return Err(FontError::InvalidTable);
            }
            previous = next;
        }
        Ok(result)
    }

    fn offset(&self, id: usize) -> Result<usize, FontError> {
        if self.metrics.head.long_loca {
            read::offset(read::u32(self.loca, mul(id, 4)?)?)
        } else {
            mul(usize::from(read::u16(self.loca, mul(id, 2)?)?), 2)
        }
    }

    /// Whether any glyph of this face carries a description.
    ///
    /// A face with a bitmap strike and an empty `glyf` has a loca table whose
    /// entries are all equal, and draws no outline this crate decodes. Work is
    /// O(glyph count) worst case and stops at the first description.
    /// # Errors
    /// Rejects a malformed loca table.
    pub fn any_outline(&self) -> Result<bool, FontError> {
        let mut previous = self.offset(0)?;
        for id in 1..=usize::from(self.metrics.maxp.glyph_count) {
            let next = self.offset(id)?;
            if next != previous {
                return Ok(true);
            }
            previous = next;
        }
        Ok(false)
    }

    fn record(&self, id: u16) -> Result<&'a [u8], FontError> {
        if id >= self.metrics.maxp.glyph_count {
            return Err(FontError::GlyphIndex);
        }
        let first = self.offset(usize::from(id))?;
        read::bytes(
            self.data,
            first,
            read::sub(self.offset(add(usize::from(id), 1)?)?, first)?,
        )
    }

    /// Decode one outline without allocation or grid fitting.
    ///
    /// Endpoints are inclusive point indices. Buffers are unspecified on error.
    /// # Errors
    /// Rejects malformed records, cycles, work limits, arithmetic overflow,
    /// invalid attachments, or insufficient caller storage.
    pub fn outline(
        &self,
        glyph: u16,
        points: &mut [Point],
        contours: &mut [usize],
    ) -> Result<Outline, FontError> {
        let mut output = Output {
            points,
            contours,
            point_count: 0,
            contour_count: 0,
            visits: 0,
            path: [0; MAX_DEPTH],
        };
        let phantoms = self.decode(glyph, 0, &mut output, None, &mut [])?;
        Ok(Outline {
            points: output.point_count,
            contours: output.contour_count,
            phantoms,
        })
    }

    /// Decode a normalized variation instance using caller-owned scratch space.
    ///
    /// Scratch holds the current glyph's points and four phantoms, plus the
    /// component counts and four phantoms of each active composite ancestor.
    /// A face without `fvar` has zero axes and accepts an empty coordinate
    /// slice, decoding the same outline as `outline`.
    /// # Errors
    /// Rejects malformed variation data, coordinates, or insufficient buffers.
    pub fn outline_instance(
        &self,
        glyph: u16,
        coords: &[Fixed],
        points: &mut [Point],
        contours: &mut [usize],
        workspace: &mut [VariationPoint],
    ) -> Result<Outline, FontError> {
        let Some(fvar) = self.font.table(*b"fvar") else {
            return if coords.is_empty() {
                self.outline(glyph, points, contours)
            } else {
                Err(FontError::InvalidTable)
            };
        };
        let axes = Axes::parse(fvar.data, self.font.table(*b"avar").map(|t| t.data))?;
        if coords.len() != axes.len()
            || coords
                .iter()
                .any(|value| value.bits().unsigned_abs() > Fixed::ONE.bits().unsigned_abs())
        {
            return Err(FontError::InvalidTable);
        }
        if axes.is_empty() {
            return self.outline(glyph, points, contours);
        }
        let Some(table) = self.font.table(*b"gvar") else {
            return self.outline(glyph, points, contours);
        };
        let gvar = Gvar::parse(table.data, axes.len(), self.metrics.maxp.glyph_count)?;
        let mut output = Output {
            points,
            contours,
            point_count: 0,
            contour_count: 0,
            visits: 0,
            path: [0; MAX_DEPTH],
        };
        let phantoms = self.decode(
            glyph,
            0,
            &mut output,
            Some(Instance { gvar, coords }),
            workspace,
        )?;
        Ok(Outline {
            points: output.point_count,
            contours: output.contour_count,
            phantoms,
        })
    }

    fn decode(
        &self,
        id: u16,
        depth: usize,
        output: &mut Output<'_>,
        instance: Option<Instance<'_, '_>>,
        workspace: &mut [VariationPoint],
    ) -> Result<[Point; 4], FontError> {
        if output
            .path
            .get(..depth)
            .ok_or(FontError::LimitExceeded)?
            .contains(&id)
        {
            return Err(FontError::Cycle);
        }
        *output.path.get_mut(depth).ok_or(FontError::LimitExceeded)? = id;
        output.visits = add(output.visits, 1)?;
        if output.visits > MAX_COMPONENTS {
            return Err(FontError::LimitExceeded);
        }
        let data = self.record(id)?;
        let phantoms = self.phantoms(id, data)?;
        if data.is_empty() {
            return vary_simple(
                id,
                output.point_count,
                output.contour_count,
                phantoms,
                output,
                instance,
                workspace,
            );
        }
        read::bytes(data, 0, 10)?;
        if read::i16(data, 2)? > read::i16(data, 6)? || read::i16(data, 4)? > read::i16(data, 8)? {
            return Err(FontError::InvalidTable);
        }
        let count = read::i16(data, 0)?;
        if count >= 0 {
            let start = output.point_count;
            let contour_start = output.contour_count;
            simple(
                data,
                usize::try_from(count).map_err(|_| FontError::InvalidTable)?,
                output,
            )?;
            vary_simple(
                id,
                start,
                contour_start,
                phantoms,
                output,
                instance,
                workspace,
            )
        } else {
            self.composite(data, (id, depth), phantoms, output, instance, workspace)
        }
    }

    fn phantoms(&self, id: u16, data: &[u8]) -> Result<[Point; 4], FontError> {
        let xmin = if data.is_empty() {
            0
        } else {
            i32::from(read::i16(data, 2)?)
        };
        let ymax = if data.is_empty() {
            0
        } else {
            i32::from(read::i16(data, 8)?)
        };
        let (advance, bearing) = self.metrics.horizontal(id)?;
        let left = Fixed::from_i32(xmin).checked_sub(Fixed::from_i32(i32::from(bearing)))?;
        let (top, bottom) = match (self.font.table(*b"vhea"), self.font.table(*b"vmtx")) {
            (None, None) => (
                Fixed::from_i32(i32::from(self.metrics.line.ascender)),
                Fixed::from_i32(i32::from(self.metrics.line.descender)),
            ),
            (Some(h), Some(m)) => {
                read::bytes(h.data, 0, 36)?;
                let count = usize::from(read::u16(h.data, 34)?);
                if count == 0 || count > usize::from(self.metrics.maxp.glyph_count) {
                    return Err(FontError::InvalidTable);
                }
                let index = usize::from(id);
                let at = mul(index.min(read::sub(count, 1)?), 4)?;
                let advance = read::u16(m.data, at)?;
                let bearing_at = if index < count {
                    add(at, 2)?
                } else {
                    add(mul(count, 4)?, mul(read::sub(index, count)?, 2)?)?
                };
                let top = Fixed::from_i32(ymax)
                    .checked_add(Fixed::from_i32(i32::from(read::i16(m.data, bearing_at)?)))?;
                (top, top.checked_sub(Fixed::from_i32(i32::from(advance)))?)
            }
            _ => return Err(FontError::MissingTable),
        };
        Ok([
            Point::new(left, Fixed::ZERO, true),
            Point::new(
                left.checked_add(Fixed::from_i32(i32::from(advance)))?,
                Fixed::ZERO,
                true,
            ),
            Point::new(Fixed::ZERO, top, true),
            Point::new(Fixed::ZERO, bottom, true),
        ])
    }

    fn composite(
        &self,
        data: &[u8],
        glyph: (u16, usize),
        mut phantoms: [Point; 4],
        output: &mut Output<'_>,
        instance: Option<Instance<'_, '_>>,
        workspace: &mut [VariationPoint],
    ) -> Result<[Point; 4], FontError> {
        let count = if instance.is_some() {
            component_count(data)?
        } else {
            0
        };
        let needed = if instance.is_some() {
            add(count, 4)?
        } else {
            0
        };
        if workspace.len() < needed {
            return Err(FontError::BufferTooSmall);
        }
        let (deltas, workspace) = workspace.split_at_mut(needed);
        prepare_components(glyph.0, instance, deltas, count, &mut phantoms)?;
        let mut component = 0;
        let start = output.point_count;
        let mut cursor = Cursor { data, at: 10 };
        let mut instructions = false;
        let mut first = true;
        loop {
            let flags = cursor.u16()?;
            if first && flags & 2 == 0 {
                return Err(FontError::InvalidTable);
            }
            first = false;
            if flags & 0xe010 != 0 || (flags & 0xc8).count_ones() > 1 || flags & 0x1800 == 0x1800 {
                return Err(FontError::InvalidTable);
            }
            let id = cursor.u16()?;
            let a = cursor.argument(flags)?;
            let b = cursor.argument(flags)?;
            let matrix = Matrix::read(&mut cursor, flags)?;
            let child_start = output.point_count;
            let mut child_phantoms =
                self.decode(id, add(glyph.1, 1)?, output, instance, workspace)?;
            let (x, y) = if flags & 2 != 0 {
                let mut p = Point::new(Fixed::from_i32(a), Fixed::from_i32(b), true);
                if instance.is_some() {
                    let delta = deltas.get(component).ok_or(FontError::InvalidTable)?;
                    p = p.translate(delta.dx, delta.dy)?;
                }
                let p = if flags & 0x800 != 0 {
                    matrix.apply(p)?
                } else {
                    p
                };
                (p.x, p.y)
            } else {
                let parent = output.attachment(start, child_start, a, &phantoms)?;
                let child = matrix.apply(output.attachment(
                    child_start,
                    output.point_count,
                    b,
                    &child_phantoms,
                )?)?;
                (
                    parent.x.checked_sub(child.x)?,
                    parent.y.checked_sub(child.y)?,
                )
            };
            for p in output
                .points
                .get_mut(child_start..output.point_count)
                .ok_or(FontError::BufferTooSmall)?
            {
                *p = matrix.apply(*p)?.translate(x, y)?;
            }
            for p in &mut child_phantoms {
                *p = matrix.apply(*p)?.translate(x, y)?;
            }
            if flags & 0x200 != 0 {
                phantoms = child_phantoms;
            }
            component = add(component, 1)?;
            instructions |= flags & 0x100 != 0;
            if flags & 0x20 == 0 {
                break;
            }
        }
        if instructions {
            let len = usize::from(cursor.u16()?);
            cursor.skip(len)?;
        }
        Ok(phantoms)
    }
}

#[derive(Clone, Copy)]
struct Instance<'a, 'b> {
    gvar: Gvar<'a>,
    coords: &'b [Fixed],
}

fn prepare_components(
    glyph: u16,
    instance: Option<Instance<'_, '_>>,
    deltas: &mut [VariationPoint],
    count: usize,
    phantoms: &mut [Point; 4],
) -> Result<(), FontError> {
    if let Some(instance) = instance {
        deltas.fill(VariationPoint::default());
        for (slot, p) in deltas
            .get_mut(count..)
            .ok_or(FontError::InvalidTable)?
            .iter_mut()
            .zip(*phantoms)
        {
            slot.original = p;
        }
        instance
            .gvar
            .evaluate(glyph, instance.coords, &[], deltas)?;
        for (p, slot) in phantoms
            .iter_mut()
            .zip(deltas.get(count..).ok_or(FontError::InvalidTable)?)
        {
            *p = slot.result()?;
        }
    }
    Ok(())
}

fn component_count(data: &[u8]) -> Result<usize, FontError> {
    let mut cursor = Cursor { data, at: 10 };
    let mut count = 0;
    loop {
        let flags = cursor.u16()?;
        cursor.u16()?;
        cursor.argument(flags)?;
        cursor.argument(flags)?;
        Matrix::read(&mut cursor, flags)?;
        count = add(count, 1)?;
        if count > MAX_COMPONENTS {
            return Err(FontError::LimitExceeded);
        }
        if flags & 0x20 == 0 {
            return Ok(count);
        }
    }
}

fn vary_simple(
    id: u16,
    start: usize,
    contour_start: usize,
    mut phantoms: [Point; 4],
    output: &mut Output<'_>,
    instance: Option<Instance<'_, '_>>,
    workspace: &mut [VariationPoint],
) -> Result<[Point; 4], FontError> {
    let Some(instance) = instance else {
        return Ok(phantoms);
    };
    let count = read::sub(output.point_count, start)?;
    let work = workspace
        .get_mut(..add(count, 4)?)
        .ok_or(FontError::BufferTooSmall)?;
    work.fill(VariationPoint::default());
    for (slot, point) in work.iter_mut().zip(
        output
            .points
            .get(start..output.point_count)
            .ok_or(FontError::InvalidTable)?,
    ) {
        slot.original = *point;
    }
    for (slot, point) in work
        .get_mut(count..)
        .ok_or(FontError::InvalidTable)?
        .iter_mut()
        .zip(phantoms)
    {
        slot.original = point;
    }
    let contours = output
        .contours
        .get_mut(contour_start..output.contour_count)
        .ok_or(FontError::InvalidTable)?;
    for end in contours.iter_mut() {
        *end = read::sub(*end, start)?;
    }
    instance
        .gvar
        .evaluate(id, instance.coords, contours, work)?;
    for end in contours.iter_mut() {
        *end = add(*end, start)?;
    }
    for (point, slot) in output
        .points
        .get_mut(start..output.point_count)
        .ok_or(FontError::InvalidTable)?
        .iter_mut()
        .zip(work.iter())
    {
        *point = slot.result()?;
    }
    for (point, slot) in phantoms
        .iter_mut()
        .zip(work.get(count..).ok_or(FontError::InvalidTable)?)
    {
        *point = slot.result()?;
    }
    Ok(phantoms)
}

struct Output<'a> {
    points: &'a mut [Point],
    contours: &'a mut [usize],
    point_count: usize,
    contour_count: usize,
    visits: usize,
    path: [u16; MAX_DEPTH],
}

impl Output<'_> {
    fn attachment(
        &self,
        start: usize,
        end: usize,
        index: i32,
        phantoms: &[Point; 4],
    ) -> Result<Point, FontError> {
        let index = usize::try_from(index).map_err(|_| FontError::InvalidTable)?;
        let count = read::sub(end, start)?;
        if index < count {
            self.points
                .get(add(start, index)?)
                .copied()
                .ok_or(FontError::InvalidTable)
        } else {
            phantoms
                .get(read::sub(index, count)?)
                .copied()
                .ok_or(FontError::InvalidTable)
        }
    }
}

fn simple(data: &[u8], count: usize, output: &mut Output<'_>) -> Result<(), FontError> {
    let mut cursor = Cursor { data, at: 10 };
    let mut total = 0;
    for i in 0..count {
        let end = add(usize::from(cursor.u16()?), 1)?;
        if end <= total {
            return Err(FontError::InvalidTable);
        }
        total = end;
        *output
            .contours
            .get_mut(add(output.contour_count, i)?)
            .ok_or(FontError::BufferTooSmall)? = read::sub(add(output.point_count, end)?, 1)?;
    }
    if count == 0 && data.len() == 10 {
        return Ok(());
    }
    let instructions = usize::from(cursor.u16()?);
    cursor.skip(instructions)?;
    let end = add(output.point_count, total)?;
    let points = output
        .points
        .get_mut(output.point_count..end)
        .ok_or(FontError::BufferTooSmall)?;
    let mut at = 0;
    while at < total {
        let flags = cursor.u8()?;
        if flags & 0x80 != 0 {
            return Err(FontError::InvalidTable);
        }
        let repeats = if flags & 8 != 0 {
            add(usize::from(cursor.u8()?), 1)?
        } else {
            1
        };
        let next = add(at, repeats)?;
        for p in points.get_mut(at..next).ok_or(FontError::InvalidTable)? {
            *p = Point {
                flags,
                on_curve: flags & 1 != 0,
                ..Point::default()
            };
        }
        at = next;
    }
    let mut position = 0_i32;
    for p in &mut *points {
        position = position
            .checked_add(cursor.delta(p.flags, 2, 16)?)
            .ok_or(FontError::Overflow)?;
        p.x = Fixed::from_i32(position);
    }
    position = 0;
    for p in points {
        position = position
            .checked_add(cursor.delta(p.flags, 4, 32)?)
            .ok_or(FontError::Overflow)?;
        p.y = Fixed::from_i32(position);
        p.flags = 0;
    }
    output.point_count = end;
    output.contour_count = add(output.contour_count, count)?;
    Ok(())
}

struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}
impl Cursor<'_> {
    fn u8(&mut self) -> Result<u8, FontError> {
        let value = read::u8(self.data, self.at)?;
        self.at = add(self.at, 1)?;
        Ok(value)
    }
    fn u16(&mut self) -> Result<u16, FontError> {
        let value = read::u16(self.data, self.at)?;
        self.at = add(self.at, 2)?;
        Ok(value)
    }
    fn i16(&mut self) -> Result<i16, FontError> {
        Ok(i16::from_be_bytes(self.u16()?.to_be_bytes()))
    }
    fn skip(&mut self, len: usize) -> Result<(), FontError> {
        read::bytes(self.data, self.at, len)?;
        self.at = add(self.at, len)?;
        Ok(())
    }
    fn delta(&mut self, flags: u8, short: u8, same: u8) -> Result<i32, FontError> {
        if flags & short != 0 {
            let value = i32::from(self.u8()?);
            Ok(if flags & same != 0 {
                value
            } else {
                value.checked_neg().ok_or(FontError::Overflow)?
            })
        } else if flags & same != 0 {
            Ok(0)
        } else {
            Ok(i32::from(self.i16()?))
        }
    }
    fn argument(&mut self, flags: u16) -> Result<i32, FontError> {
        Ok(match (flags & 1 != 0, flags & 2 != 0) {
            (true, true) => i32::from(self.i16()?),
            (true, false) => i32::from(self.u16()?),
            (false, true) => i32::from(i8::from_be_bytes([self.u8()?])),
            (false, false) => i32::from(self.u8()?),
        })
    }
}

struct Matrix {
    xx: Fixed,
    xy: Fixed,
    yx: Fixed,
    yy: Fixed,
}
impl Matrix {
    fn read(c: &mut Cursor<'_>, flags: u16) -> Result<Self, FontError> {
        let mut m = Self {
            xx: Fixed::ONE,
            xy: Fixed::ZERO,
            yx: Fixed::ZERO,
            yy: Fixed::ONE,
        };
        if flags & 8 != 0 {
            m.xx = Fixed::from_2_14(c.i16()?);
            m.yy = m.xx;
        } else if flags & 0x40 != 0 {
            m.xx = Fixed::from_2_14(c.i16()?);
            m.yy = Fixed::from_2_14(c.i16()?);
        } else if flags & 0x80 != 0 {
            m.xx = Fixed::from_2_14(c.i16()?);
            m.yx = Fixed::from_2_14(c.i16()?);
            m.xy = Fixed::from_2_14(c.i16()?);
            m.yy = Fixed::from_2_14(c.i16()?);
        }
        Ok(m)
    }
    fn apply(&self, p: Point) -> Result<Point, FontError> {
        Ok(Point::new(
            p.x.checked_mul(self.xx)?
                .checked_add(p.y.checked_mul(self.xy)?)?,
            p.x.checked_mul(self.yx)?
                .checked_add(p.y.checked_mul(self.yy)?)?,
            p.on_curve,
        ))
    }
}
