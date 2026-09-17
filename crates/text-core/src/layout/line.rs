// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{
    ClusterBox, Line, PositionedGlyph, Workspace,
    engine::{Context, add, hard},
};
use crate::{
    Fixed, Font, FontError, TextError, bidi,
    glyf::{Glyf, Point},
    resolve::Run,
    segment,
    shape::{self, Buffer, Caret, Gdef, LayoutTable},
    variation::{Instance, VariationPoint},
};

#[expect(
    clippy::too_many_arguments,
    reason = "caller-owned outline scratch is split from glyph storage"
)]
pub(super) fn advance(
    font: &Font<'_>,
    coords: &[Fixed],
    instance: &Instance<'_, '_>,
    glyph: u16,
    size: Fixed,
    points: &mut [Point],
    contours: &mut [usize],
    variation: &mut [VariationPoint],
) -> Result<Fixed, FontError> {
    match instance.advance(glyph, size, None) {
        Err(FontError::MissingOutline) => {
            let outline =
                Glyf::parse(font)?.outline_instance(glyph, coords, points, contours, variation)?;
            instance.advance(glyph, size, Some(&outline))
        }
        result => result,
    }
}
impl Context<'_, '_, '_> {
    fn bidi_line(
        &self,
        workspace: &mut Workspace<'_>,
        start: usize,
        bidi_end: usize,
    ) -> Result<usize, TextError> {
        let units = workspace
            .bidi
            .get(..self.scalars)
            .ok_or(TextError::BufferTooSmall)?;
        let first = units.partition_point(|u| u.byte() < start);
        let last = units.partition_point(|u| u.byte() < bidi_end);
        let info = bidi::reorder_line(units, first..last, workspace.levels, workspace.indices)?;
        workspace
            .ranks
            .get_mut(first..last)
            .ok_or(TextError::BufferTooSmall)?
            .fill(usize::MAX);
        for (rank, index) in workspace
            .indices
            .get(..info.count)
            .ok_or(TextError::BufferTooSmall)?
            .iter()
            .copied()
            .enumerate()
        {
            *workspace
                .ranks
                .get_mut(index)
                .ok_or(TextError::BufferTooSmall)? = rank;
        }
        Ok(first)
    }
    fn font_run(
        &self,
        workspace: &mut Workspace<'_>,
        run: &Run,
        from: usize,
        to: usize,
    ) -> Result<(usize, i64, [Fixed; 3]), TextError> {
        let font = self
            .set
            .chain(self.style.role)
            .get(run.face)
            .ok_or(TextError::NoFont)?;
        let instance = Instance::new(font, run.coordinates())?;
        let line = instance.line_metrics(self.style.size)?;
        let metrics = font.metrics()?;
        let units_per_em = i64::from(metrics.head.units_per_em);
        let design_size = Fixed::from_i32(i32::from(metrics.head.units_per_em));
        let gdef = font
            .table(*b"GDEF")
            .map(|t| Gdef::parse(t.data, run.coordinates().len()))
            .transpose()?
            .unwrap_or_default();
        let text = self.text.get(from..to).ok_or(TextError::InvalidInput)?;
        let n = shape::prepare(
            text,
            font.cmap()?,
            run.script,
            run.level & 1 != 0,
            workspace.glyphs,
        )?;
        let mut buffer = Buffer::new(workspace.glyphs, n)?;
        if text == "\t" {
            for glyph in buffer.glyphs_mut() {
                glyph.hidden = true;
            }
        }
        if !run.simple
            && text != "\t"
            && let Some(table) = font.table(*b"GSUB")
        {
            LayoutTable::parse(table.data, true)?.apply(
                shape::script_tag(run.script),
                run.language.tag(),
                shape::SUBSTITUTION_FEATURES,
                gdef,
                run.coordinates(),
                run.level & 1 != 0,
                &mut buffer,
            )?;
        }
        buffer.set_classes(gdef)?;
        if run.simple {
            for glyph in buffer.glyphs_mut() {
                glyph.advance = if glyph.hidden {
                    Fixed::ZERO
                } else {
                    advance(
                        font,
                        run.coordinates(),
                        &instance,
                        glyph.id,
                        design_size,
                        workspace.points,
                        workspace.contours,
                        workspace.variation,
                    )?
                };
            }
        } else {
            buffer.set_advances(|id| {
                advance(
                    font,
                    run.coordinates(),
                    &instance,
                    id,
                    design_size,
                    workspace.points,
                    workspace.contours,
                    workspace.variation,
                )
            })?;
            if text != "\t"
                && let Some(table) = font.table(*b"GPOS")
            {
                LayoutTable::parse(table.data, false)?.apply(
                    shape::script_tag(run.script),
                    run.language.tag(),
                    shape::POSITION_FEATURES,
                    gdef,
                    run.coordinates(),
                    run.level & 1 != 0,
                    &mut buffer,
                )?;
            }
            shape::finish(&mut buffer, run.level & 1 != 0)?;
        }
        Ok((buffer.glyphs().len(), units_per_em, line))
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "resolved run and line source intervals"
    )]
    fn copy_run(
        &self,
        workspace: &mut Workspace<'_>,
        run: &Run,
        index: usize,
        from: usize,
        first: usize,
        n: usize,
        units_per_em: i64,
        count: &mut usize,
    ) -> Result<(), TextError> {
        let units = workspace
            .bidi
            .get(..self.scalars)
            .ok_or(TextError::BufferTooSmall)?;
        for glyph in workspace.glyphs.get(..n).ok_or(TextError::BufferTooSmall)? {
            let begin = add(from, glyph.start)?;
            let finish = add(from, glyph.end)?;
            let scalar = units.partition_point(|u| u.byte() < begin);
            let scalar_end = units.partition_point(|u| u.byte() < finish);
            let rank = workspace
                .ranks
                .get(scalar..scalar_end)
                .ok_or(TextError::InvalidInput)?
                .iter()
                .copied()
                .min()
                .filter(|r| *r != usize::MAX)
                .unwrap_or(0);
            let level = *workspace
                .levels
                .get(scalar.checked_sub(first).ok_or(TextError::InvalidInput)?)
                .ok_or(TextError::BufferTooSmall)?;
            let level = if level == 255 { run.level } else { level };
            *workspace
                .line_glyphs
                .get_mut(*count)
                .ok_or(TextError::BufferTooSmall)? = PositionedGlyph {
                run: index,
                id: glyph.id,
                start: begin,
                end: finish,
                x: glyph.x.mul_div(self.style.size, units_per_em)?,
                y: glyph
                    .y
                    .mul_div(self.style.size, units_per_em)?
                    .checked_neg()?,
                advance: if glyph.hidden {
                    Fixed::ZERO
                } else {
                    glyph.advance.mul_div(self.style.size, units_per_em)?
                },
                level,
                hidden: glyph.hidden,
                rank,
                ordinal: *count,
                pen: Fixed::ZERO,
            };
            *count = add(*count, 1)?;
        }
        Ok(())
    }
    pub(super) fn shape(
        &mut self,
        workspace: &mut Workspace<'_>,
        start: usize,
        end: usize,
        soft: bool,
        top: Fixed,
        line_number: usize,
    ) -> Result<Line, TextError> {
        let source = self.text.get(start..end).ok_or(TextError::InvalidInput)?;
        self.charge(source.chars().count())?;
        let content = source.trim_end_matches(hard);
        let bidi_end = add(start, content.len())?;
        let visible = if soft {
            content.trim_end_matches([' ', '\t'])
        } else {
            content
        };
        let visible_end = add(start, visible.len())?;
        let first = self.bidi_line(workspace, start, bidi_end)?;
        let mut count = 0;
        let mut extents = self.base;
        let first_run = workspace
            .runs
            .get(..self.runs)
            .ok_or(TextError::BufferTooSmall)?
            .partition_point(|run| run.end <= start);
        for index in first_run..self.runs {
            let run = *workspace.runs.get(index).ok_or(TextError::BufferTooSmall)?;
            if run.start >= visible_end {
                break;
            }
            let mut from = run.start.max(start);
            let to = run.end.min(visible_end);
            if from >= to {
                continue;
            }
            while from < to {
                let remaining = self.text.get(from..to).ok_or(TextError::InvalidInput)?;
                let end = match remaining.find('\t') {
                    Some(0) => add(from, 1)?,
                    Some(offset) => add(from, offset)?,
                    None => to,
                };
                let (n, units_per_em, line) = self.font_run(workspace, &run, from, end)?;
                extents[0] = extents[0].max(line[0]);
                extents[1] = extents[1].min(line[1]);
                extents[2] = extents[2].max(line[2]);
                self.copy_run(
                    workspace,
                    &run,
                    index,
                    from,
                    first,
                    n,
                    units_per_em,
                    &mut count,
                )?;
                from = end;
            }
        }
        merge_clusters(
            workspace
                .line_glyphs
                .get_mut(..count)
                .ok_or(TextError::BufferTooSmall)?,
        )?;
        let (width, height, baseline) = self.position(workspace, count, extents, top)?;
        let clusters = self.clusters(
            workspace,
            count,
            start,
            end,
            visible_end,
            first,
            line_number,
            width,
        )?;
        Ok(Line {
            start,
            end,
            glyph_count: count,
            cluster_count: clusters,
            y: top,
            height,
            baseline,
            width,
            ..Line::default()
        })
    }
    fn position(
        &self,
        workspace: &mut Workspace<'_>,
        count: usize,
        extents: [Fixed; 3],
        top: Fixed,
    ) -> Result<(Fixed, Fixed, Fixed), TextError> {
        let glyphs = workspace
            .line_glyphs
            .get_mut(..count)
            .ok_or(TextError::BufferTooSmall)?;
        glyphs.sort_unstable_by_key(|g| {
            (
                g.rank,
                if g.level & 1 != 0 {
                    usize::MAX.saturating_sub(g.ordinal)
                } else {
                    g.ordinal
                },
            )
        });
        let mut pen = Fixed::ZERO;
        let mut minimum = Fixed::ZERO;
        let mut maximum = Fixed::ZERO;
        for glyph in glyphs.iter_mut() {
            if self.text.get(glyph.start..glyph.end) == Some("\t") {
                let stop = pen
                    .bits()
                    .div_euclid(self.tab.bits())
                    .checked_add(1)
                    .ok_or(TextError::Overflow)?
                    .checked_mul(self.tab.bits())
                    .ok_or(TextError::Overflow)?;
                glyph.advance = Fixed::from_bits(stop).checked_sub(pen)?;
                glyph.hidden = true;
                glyph.x = Fixed::ZERO;
            }
            glyph.pen = pen;
            glyph.x = glyph.x.checked_add(pen)?;
            pen = pen.checked_add(glyph.advance)?;
            minimum = minimum.min(pen);
            maximum = maximum.max(pen);
        }
        let ascent = extents[0].max(Fixed::ZERO);
        let baseline = top.checked_add(ascent)?;
        let height = ascent
            .checked_sub(extents[1].min(Fixed::ZERO))?
            .checked_add(extents[2])?;
        for glyph in glyphs {
            glyph.x = glyph.x.checked_sub(minimum)?;
            glyph.pen = glyph.pen.checked_sub(minimum)?;
            glyph.y = glyph.y.checked_add(baseline)?;
        }
        let width = maximum.checked_sub(minimum)?;
        Ok((width, height, baseline))
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "line source, bidi, and output intervals"
    )]
    fn clusters(
        &self,
        workspace: &mut Workspace<'_>,
        glyph_count: usize,
        start: usize,
        end: usize,
        visible_end: usize,
        first_scalar: usize,
        line: usize,
        width: Fixed,
    ) -> Result<usize, TextError> {
        let mut count = 0;
        let mut first = 0;
        while first < glyph_count {
            let lead = *workspace
                .line_glyphs
                .get(first)
                .ok_or(TextError::BufferTooSmall)?;
            let mut last = add(first, 1)?;
            while last < glyph_count
                && workspace
                    .line_glyphs
                    .get(last)
                    .is_some_and(|g| g.start == lead.start && g.end == lead.end)
            {
                last = add(last, 1)?;
            }
            self.cluster_group(workspace, first, last, line, &mut count)?;
            first = last;
        }
        let tail = self
            .text
            .get(visible_end..end)
            .ok_or(TextError::InvalidInput)?;
        let mut begin = visible_end;
        let base = workspace
            .bidi
            .get(first_scalar)
            .map_or(0, bidi::Unit::paragraph_level);
        for local in segment::grapheme_boundaries(tail).skip(1) {
            let finish = add(visible_end, local)?;
            let x = if base & 1 == 0 { width } else { Fixed::ZERO };
            *workspace
                .line_clusters
                .get_mut(count)
                .ok_or(TextError::BufferTooSmall)? = ClusterBox {
                start: begin,
                end: finish,
                line,
                left: x,
                right: x,
                level: base,
            };
            count = add(count, 1)?;
            begin = finish;
        }
        self.deleted_clusters(workspace, start, visible_end, line, &mut count)?;
        workspace
            .line_clusters
            .get_mut(..count)
            .ok_or(TextError::BufferTooSmall)?
            .sort_unstable_by_key(|c| (c.left, c.right, c.start));
        Ok(count)
    }
    fn deleted_clusters(
        &self,
        workspace: &mut Workspace<'_>,
        start: usize,
        end: usize,
        line: usize,
        count: &mut usize,
    ) -> Result<(), TextError> {
        let existing = *count;
        workspace
            .line_clusters
            .get_mut(..existing)
            .ok_or(TextError::BufferTooSmall)?
            .sort_unstable_by_key(|c| c.start);
        let source = self.text.get(start..end).ok_or(TextError::InvalidInput)?;
        let mut begin = start;
        for boundary in segment::grapheme_boundaries(source).skip(1) {
            let finish = add(start, boundary)?;
            let boxes = workspace
                .line_clusters
                .get(..existing)
                .ok_or(TextError::BufferTooSmall)?;
            let index = boxes.partition_point(|c| c.start < begin);
            if boxes.get(index).is_none_or(|c| c.start != begin) {
                let next = boxes.get(index);
                let previous = index.checked_sub(1).and_then(|i| boxes.get(i));
                let (x, level) = next
                    .map(|c| (if c.level & 1 == 0 { c.left } else { c.right }, c.level))
                    .or_else(|| {
                        previous.map(|c| (if c.level & 1 == 0 { c.right } else { c.left }, c.level))
                    })
                    .unwrap_or((Fixed::ZERO, 0));
                *workspace
                    .line_clusters
                    .get_mut(*count)
                    .ok_or(TextError::BufferTooSmall)? = ClusterBox {
                    start: begin,
                    end: finish,
                    line,
                    left: x,
                    right: x,
                    level,
                };
                *count = add(*count, 1)?;
            }
            begin = finish;
        }
        Ok(())
    }
    fn cluster_group(
        &self,
        workspace: &mut Workspace<'_>,
        first: usize,
        last: usize,
        line: usize,
        count: &mut usize,
    ) -> Result<(), TextError> {
        let lead = *workspace
            .line_glyphs
            .get(first)
            .ok_or(TextError::BufferTooSmall)?;
        let group = workspace
            .line_glyphs
            .get(first..last)
            .ok_or(TextError::BufferTooSmall)?;
        let mut left = lead.pen;
        let mut right = lead.pen;
        for glyph in group {
            let next = glyph.pen.checked_add(glyph.advance)?;
            left = left.min(glyph.pen).min(next);
            right = right.max(glyph.pen).max(next);
        }
        let source = self
            .text
            .get(lead.start..lead.end)
            .ok_or(TextError::InvalidInput)?;
        let boundaries = segment::grapheme_boundaries(source);
        let clusters = boundaries
            .clone()
            .count()
            .checked_sub(1)
            .ok_or(TextError::InvalidInput)?;
        let run = *workspace
            .runs
            .get(lead.run)
            .ok_or(TextError::BufferTooSmall)?;
        let ligature = group
            .iter()
            .find(|g| g.advance != Fixed::ZERO)
            .copied()
            .unwrap_or(lead);
        let caret_count = if clusters > 1 {
            self.carets(workspace, &run, ligature)?
        } else {
            0
        };
        let mut begin = lead.start;
        for (ordinal, local) in boundaries.skip(1).enumerate() {
            let finish = add(lead.start, local)?;
            let before = if lead.level & 1 == 0 {
                ordinal
            } else {
                clusters.checked_sub(ordinal).ok_or(TextError::Overflow)?
            };
            let after = if lead.level & 1 == 0 {
                add(ordinal, 1)?
            } else {
                before.checked_sub(1).ok_or(TextError::Overflow)?
            };
            let a = caret_edge(
                workspace,
                caret_count,
                clusters,
                before,
                left,
                right,
                ligature.x,
            )?;
            let b = caret_edge(
                workspace,
                caret_count,
                clusters,
                after,
                left,
                right,
                ligature.x,
            )?;
            *workspace
                .line_clusters
                .get_mut(*count)
                .ok_or(TextError::BufferTooSmall)? = ClusterBox {
                start: begin,
                end: finish,
                line,
                left: a.min(b),
                right: a.max(b),
                level: lead.level,
            };
            *count = add(*count, 1)?;
            begin = finish;
        }
        Ok(())
    }
    fn carets(
        &self,
        workspace: &mut Workspace<'_>,
        run: &Run,
        glyph: PositionedGlyph,
    ) -> Result<usize, TextError> {
        let font = self
            .set
            .chain(self.style.role)
            .get(run.face)
            .ok_or(TextError::NoFont)?;
        let Some(table) = font.table(*b"GDEF") else {
            return Ok(0);
        };
        let gdef = Gdef::parse(table.data, run.coordinates().len())?;
        let n = gdef.carets(glyph.id, run.coordinates(), workspace.caret_values)?;
        let carets = workspace
            .caret_values
            .get_mut(..n)
            .ok_or(TextError::BufferTooSmall)?;
        let points = if carets.iter().any(|c| matches!(c, Caret::Point(_))) {
            let glyf = Glyf::parse(font)?;
            let outline = if run.coordinates().is_empty() {
                glyf.outline(glyph.id, workspace.points, workspace.contours)?
            } else {
                glyf.outline_instance(
                    glyph.id,
                    run.coordinates(),
                    workspace.points,
                    workspace.contours,
                    workspace.variation,
                )?
            };
            outline.points
        } else {
            0
        };
        let units = i64::from(font.metrics()?.head.units_per_em);
        for caret in carets {
            let value = match *caret {
                Caret::Coordinate(v) => v,
                Caret::Point(i) => {
                    workspace
                        .points
                        .get(..points)
                        .and_then(|p| p.get(usize::from(i)))
                        .ok_or(FontError::InvalidTable)?
                        .x
                }
            };
            *caret = Caret::Coordinate(value.mul_div(self.style.size, units)?);
        }
        Ok(n)
    }
}
fn merge_clusters(glyphs: &mut [PositionedGlyph]) -> Result<(), TextError> {
    glyphs.sort_unstable_by_key(|g| (g.start, g.ordinal));
    let mut first = 0;
    while let Some(lead) = glyphs.get(first).copied() {
        let mut end = lead.end;
        let mut rank = lead.rank;
        let mut last = add(first, 1)?;
        while let Some(next) = glyphs.get(last).filter(|g| g.start < end) {
            end = end.max(next.end);
            rank = rank.min(next.rank);
            last = add(last, 1)?;
        }
        for glyph in glyphs.get_mut(first..last).ok_or(TextError::InvalidInput)? {
            glyph.start = lead.start;
            glyph.end = end;
            glyph.rank = rank;
        }
        first = last;
    }
    Ok(())
}

fn caret_edge(
    workspace: &Workspace<'_>,
    n: usize,
    count: usize,
    index: usize,
    left: Fixed,
    right: Fixed,
    origin: Fixed,
) -> Result<Fixed, TextError> {
    if index == 0 {
        return Ok(left);
    }
    if index == count {
        return Ok(right);
    }
    if add(n, 1)? == count
        && let Some(Caret::Coordinate(value)) = workspace
            .caret_values
            .get(index.checked_sub(1).ok_or(TextError::Overflow)?)
    {
        return Ok(origin.checked_add(*value)?);
    }
    Ok(left.checked_add(right.checked_sub(left)?.mul_ratio(
        i64::try_from(index).map_err(|_| TextError::Overflow)?,
        i64::try_from(count).map_err(|_| TextError::Overflow)?,
    )?)?)
}
