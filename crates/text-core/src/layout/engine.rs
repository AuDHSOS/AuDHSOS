// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{LayoutBuffers, LayoutInfo, LayoutView, MAX_SCALARS, Workspace};
use crate::{
    Fixed, FontSet, TextError, TextStyle, bidi, resolve,
    segment::{self, Break},
    variation::Instance,
};

pub(super) struct Context<'a, 's, 'f> {
    pub set: &'a FontSet<'s, 'f>,
    pub style: &'a TextStyle,
    pub text: &'a str,
    pub scalars: usize,
    pub runs: usize,
    pub breaks: usize,
    pub work: usize,
    pub base: [Fixed; 3],
    pub tab: Fixed,
}
pub(super) fn add(a: usize, b: usize) -> Result<usize, TextError> {
    a.checked_add(b).ok_or(TextError::Overflow)
}
pub(super) const fn hard(c: char) -> bool {
    matches!(
        c,
        '\r' | '\n' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}
impl<'a, 's, 'f> Context<'a, 's, 'f> {
    fn new(
        set: &'a FontSet<'s, 'f>,
        style: &'a TextStyle,
        text: &'a str,
        workspace: &mut Workspace<'_>,
    ) -> Result<Self, TextError> {
        style.validate()?;
        let font = set.chain(style.role).first().ok_or(TextError::NoFont)?;
        let scalars = text.chars().count();
        if scalars > MAX_SCALARS {
            return Err(TextError::LimitExceeded);
        }
        let info = bidi::resolve(
            text,
            bidi::Direction::Auto,
            workspace.bidi,
            workspace.indices,
        )?;
        let breaks = segment::line_breaks(text, workspace.line_units, workspace.breaks)?;
        let runs = resolve::resolve_into(
            set,
            style,
            text,
            workspace
                .bidi
                .get(..info.count)
                .ok_or(TextError::BufferTooSmall)?,
            workspace.glyphs,
            workspace.runs,
        )?;
        let mut coords = [Fixed::ZERO; 64];
        let count = resolve::coordinates(font, style.weight, &mut coords)?;
        let coords = coords.get(..count).ok_or(TextError::Overflow)?;
        let instance = Instance::new(font, coords)?;
        let base = instance.line_metrics(style.size)?;
        let space = font.cmap()?.glyph_index(' ');
        let tab = if space == 0 {
            style.size
        } else {
            super::line::advance(
                font,
                coords,
                &instance,
                space,
                style.size,
                workspace.points,
                workspace.contours,
                workspace.variation,
            )?
            .mul_ratio(4, 1)?
        };
        let tab = if tab > Fixed::ZERO { tab } else { style.size };
        Ok(Self {
            set,
            style,
            text,
            scalars,
            runs,
            breaks,
            work: 0,
            base,
            tab,
        })
    }
    pub(super) fn charge(&mut self, n: usize) -> Result<(), TextError> {
        self.work = add(self.work, n)?;
        if self.work > 1_000_000 {
            Err(TextError::LimitExceeded)
        } else {
            Ok(())
        }
    }
    fn soft(&self, end: usize) -> Result<bool, TextError> {
        Ok(end < self.text.len()
            && !self
                .text
                .get(..end)
                .ok_or(TextError::InvalidInput)?
                .chars()
                .next_back()
                .is_some_and(hard))
    }
    fn choose(
        &mut self,
        workspace: &mut Workspace<'_>,
        start: usize,
        width: Option<Fixed>,
    ) -> Result<usize, TextError> {
        let mut best = None;
        let first = workspace
            .breaks
            .get(..self.breaks)
            .ok_or(TextError::BufferTooSmall)?
            .partition_point(|boundary| boundary.byte <= start);
        for i in first..self.breaks {
            let boundary = *workspace.breaks.get(i).ok_or(TextError::BufferTooSmall)?;
            if boundary.byte <= start || boundary.kind == Break::Prohibited {
                continue;
            }
            if width.is_none() && boundary.kind != Break::Mandatory {
                continue;
            }
            let suffix = self.text.get(start..).ok_or(TextError::InvalidInput)?;
            let local = boundary
                .byte
                .checked_sub(start)
                .ok_or(TextError::InvalidInput)?;
            if !segment::grapheme_boundaries(suffix)
                .take_while(|byte| *byte <= local)
                .any(|byte| byte == local)
            {
                continue;
            }
            let candidate = self.shape(
                workspace,
                start,
                boundary.byte,
                self.soft(boundary.byte)?,
                Fixed::ZERO,
                0,
            )?;
            if width.is_none_or(|width| candidate.width <= width) {
                best = Some(boundary.byte);
                if boundary.kind == Break::Mandatory {
                    return Ok(boundary.byte);
                }
            } else {
                if let Some(best) = best {
                    return Ok(best);
                }
                let text = self
                    .text
                    .get(start..boundary.byte)
                    .ok_or(TextError::InvalidInput)?;
                let mut previous = None;
                for local in segment::grapheme_boundaries(text).skip(1) {
                    let end = add(start, local)?;
                    let candidate =
                        self.shape(workspace, start, end, self.soft(end)?, Fixed::ZERO, 0)?;
                    if width.is_some_and(|width| candidate.width > width) {
                        return Ok(previous.unwrap_or(end));
                    }
                    previous = Some(end);
                }
                return Ok(boundary.byte);
            }
        }
        best.ok_or(TextError::InvalidInput)
    }
    fn execute(
        &mut self,
        workspace: &mut Workspace<'_>,
        mut buffers: Option<&mut LayoutBuffers<'_>>,
        width: Option<Fixed>,
    ) -> Result<LayoutInfo, TextError> {
        let mut info = LayoutInfo {
            text_len: self.text.len(),
            generation: self.set.generation,
            role: self.style.role,
            runs: self.runs,
            ..LayoutInfo::default()
        };
        if width.is_some_and(|w| w < Fixed::ZERO) {
            return Err(TextError::InvalidInput);
        }
        if self.text.is_empty() {
            return Ok(info);
        }
        let mut start = 0;
        let trailing = self.text.chars().next_back().is_some_and(hard);
        loop {
            let end = if start == self.text.len() {
                start
            } else {
                self.choose(workspace, start, width)?
            };
            let mut line = self.shape(
                workspace,
                start,
                end,
                self.soft(end)?,
                info.height,
                info.lines,
            )?;
            line.glyph_start = info.glyphs;
            line.cluster_start = info.clusters;
            line.overflow = width.is_some_and(|w| line.width > w);
            if let Some(out) = buffers.as_deref_mut() {
                *out.lines
                    .get_mut(info.lines)
                    .ok_or(TextError::BufferTooSmall)? = line;
                out.glyphs
                    .get_mut(info.glyphs..add(info.glyphs, line.glyph_count)?)
                    .ok_or(TextError::BufferTooSmall)?
                    .copy_from_slice(
                        workspace
                            .line_glyphs
                            .get(..line.glyph_count)
                            .ok_or(TextError::BufferTooSmall)?,
                    );
                out.clusters
                    .get_mut(info.clusters..add(info.clusters, line.cluster_count)?)
                    .ok_or(TextError::BufferTooSmall)?
                    .copy_from_slice(
                        workspace
                            .line_clusters
                            .get(..line.cluster_count)
                            .ok_or(TextError::BufferTooSmall)?,
                    );
            }
            info.lines = add(info.lines, 1)?;
            info.glyphs = add(info.glyphs, line.glyph_count)?;
            info.clusters = add(info.clusters, line.cluster_count)?;
            info.width = info.width.max(line.width);
            info.height = info.height.checked_add(line.height)?;
            if end == self.text.len() && (start == end || !trailing) {
                break;
            }
            start = end;
        }
        Ok(info)
    }
}
/// Lay out text into caller buffers; all failures invalidate scratch and output contents.
/// # Errors
/// Returns malformed fonts, invalid settings, capacity, numeric, or work-limit errors.
pub fn layout_into<'a>(
    set: &FontSet<'_, '_>,
    style: &TextStyle,
    text: &str,
    max_width: Option<Fixed>,
    workspace: &'a mut Workspace<'_>,
    buffers: &'a mut LayoutBuffers<'_>,
) -> Result<LayoutView<'a>, TextError> {
    let mut context = Context::new(set, style, text, workspace)?;
    let info = context.execute(workspace, Some(buffers), max_width)?;
    Ok(LayoutView {
        info,
        runs: workspace
            .runs
            .get(..info.runs)
            .ok_or(TextError::BufferTooSmall)?,
        lines: buffers
            .lines
            .get(..info.lines)
            .ok_or(TextError::BufferTooSmall)?,
        glyphs: buffers
            .glyphs
            .get(..info.glyphs)
            .ok_or(TextError::BufferTooSmall)?,
        clusters: buffers
            .clusters
            .get(..info.clusters)
            .ok_or(TextError::BufferTooSmall)?,
    })
}
/// Measure through the identical candidate-line and cluster-geometry path.
/// # Errors
/// Returns the same font, settings, scratch, numeric, and work-limit errors as layout.
pub fn measure_into(
    set: &FontSet<'_, '_>,
    style: &TextStyle,
    text: &str,
    max_width: Option<Fixed>,
    workspace: &mut Workspace<'_>,
) -> Result<(Fixed, Fixed), TextError> {
    let mut context = Context::new(set, style, text, workspace)?;
    let info = context.execute(workspace, None, max_width)?;
    Ok((info.width, info.height))
}
