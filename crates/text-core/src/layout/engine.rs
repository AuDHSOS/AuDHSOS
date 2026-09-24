// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

use super::{LayoutBuffers, LayoutInfo, LayoutView, MAX_SCALARS, Workspace, face::Faces};
use crate::{
    Fixed, FontSet, TextError, TextStyle, bidi, resolve,
    segment::{self, Break},
    shape::Budget,
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
    pub shaping: Budget,
    pub base: [Fixed; 3],
    pub tab: Fixed,
    pub faces: Faces<'f>,
}
/// Candidate line ends `settle` steps through.
#[derive(Clone, Copy)]
enum Positions {
    /// Allowed boundaries through this byte.
    Allowed(usize),
    /// Grapheme boundaries before this byte.
    Graphemes(usize),
}
/// First window of a layout, in scalars.
const FIRST_WINDOW: usize = 16;
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
        #[cfg(test)]
        {
            super::probe::VALIDATIONS.with(|v| v.set(0));
            super::probe::WORK.with(|w| w.set(0));
        }
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
                &mut None,
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
            shaping: Budget::default(),
            base,
            tab,
            faces: Faces::default(),
        })
    }
    pub(super) fn charge(&mut self, n: usize) -> Result<(), TextError> {
        self.work = add(self.work, n)?;
        #[cfg(test)]
        super::probe::WORK.with(|w| w.set(self.work));
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
    /// End of the visible text of `start..end`: trailing hard breaks and, at a
    /// soft end, trailing spaces and tabs excluded.
    pub(super) fn visible_end(
        &self,
        start: usize,
        end: usize,
        soft: bool,
    ) -> Result<usize, TextError> {
        let content = self
            .text
            .get(start..end)
            .ok_or(TextError::InvalidInput)?
            .trim_end_matches(hard);
        let visible = if soft {
            content.trim_end_matches([' ', '\t'])
        } else {
            content
        };
        add(start, visible.len())
    }
    fn fits(
        &mut self,
        workspace: &mut Workspace<'_>,
        start: usize,
        end: usize,
        width: Fixed,
    ) -> Result<bool, TextError> {
        let soft = self.soft(end)?;
        Ok(self
            .shape(workspace, start, end, soft, Fixed::ZERO, 0)?
            .width
            <= width)
    }
    /// Allowed, grapheme-aligned line boundaries in `start + 1..=bound`,
    /// through the first mandatory one.
    fn allowed<'w>(
        &'w self,
        workspace: &'w Workspace<'_>,
        start: usize,
        bound: usize,
    ) -> Result<impl Iterator<Item = (usize, Break)> + 'w, TextError> {
        let breaks = workspace
            .breaks
            .get(..self.breaks)
            .ok_or(TextError::BufferTooSmall)?;
        let first = breaks.partition_point(|boundary| boundary.byte <= start);
        let suffix = self.text.get(start..).ok_or(TextError::InvalidInput)?;
        let mut graphemes = segment::grapheme_boundaries(suffix).peekable();
        let mut done = false;
        Ok(breaks
            .get(first..)
            .unwrap_or_default()
            .iter()
            .take_while(move |b| b.byte <= bound)
            .filter(move |b| {
                b.kind != Break::Prohibited
                    && b.byte.checked_sub(start).is_some_and(|local| {
                        while graphemes.next_if(|g| *g < local).is_some() {}
                        graphemes.peek() == Some(&local)
                    })
            })
            .take_while(move |b| !core::mem::replace(&mut done, b.kind == Break::Mandatory))
            .map(|b| (b.byte, b.kind)))
    }
    /// Grapheme boundaries in `start + 1..stop`.
    fn graphemes(
        &self,
        start: usize,
        stop: usize,
    ) -> Result<impl Iterator<Item = usize> + '_, TextError> {
        let text = self.text.get(start..stop).ok_or(TextError::InvalidInput)?;
        Ok(segment::grapheme_boundaries(text)
            .skip(1)
            .filter_map(move |local| start.checked_add(local))
            .filter(move |end| *end < stop))
    }
    /// Measurement window: the first grapheme boundary at least `scalars`
    /// scalars after `start`, clamped to the paragraph end; `true` at that end.
    fn window_end(
        &self,
        workspace: &Workspace<'_>,
        start: usize,
        scalars: usize,
    ) -> Result<(usize, bool), TextError> {
        let suffix = self.text.get(start..).ok_or(TextError::InvalidInput)?;
        let target = suffix
            .char_indices()
            .nth(scalars)
            .map_or(suffix.len(), |(i, _)| i);
        let local = segment::grapheme_boundaries(suffix)
            .find(|b| *b >= target)
            .unwrap_or(suffix.len());
        let end = add(start, local)?;
        if let Some((byte, _)) = self
            .allowed(workspace, start, end)?
            .find(|(_, kind)| *kind == Break::Mandatory)
        {
            return Ok((byte, true));
        }
        Ok((end, end == self.text.len()))
    }
    /// Estimated widths of `positions` from the logically sorted cluster boxes
    /// of the window `start..end`: the last position that fits before the
    /// first that does not, and the window's visible width.
    fn estimate(
        &self,
        workspace: &Workspace<'_>,
        start: usize,
        end: usize,
        count: usize,
        positions: impl Iterator<Item = usize>,
        width: Fixed,
    ) -> Result<(Option<usize>, Fixed), TextError> {
        let mut clusters = workspace
            .line_clusters
            .get(..count)
            .ok_or(TextError::BufferTooSmall)?
            .iter()
            .peekable();
        let mut sum = Fixed::ZERO;
        let mut fit = None;
        for position in positions {
            let visible = self.visible_end(start, position, self.soft(position)?)?;
            while let Some(c) = clusters.next_if(|c| c.end <= visible) {
                sum = sum.checked_add(c.right.checked_sub(c.left)?)?;
            }
            if sum > width {
                break;
            }
            fit = Some(position);
        }
        // Trailing spaces at a soft window end make no candidate overflow.
        let visible = self.visible_end(start, end, self.soft(end)?)?;
        for c in clusters.take_while(|c| c.end <= visible) {
            sum = sum.checked_add(c.right.checked_sub(c.left)?)?;
        }
        Ok((fit, sum))
    }
    /// From the estimated `end`, step to the last position that fits exactly
    /// and whose successor does not; `None` if no earlier position fits.
    fn settle(
        &mut self,
        workspace: &mut Workspace<'_>,
        start: usize,
        mut end: usize,
        width: Fixed,
        positions: Positions,
    ) -> Result<Option<usize>, TextError> {
        let mut back = false;
        while !self.fits(workspace, start, end, width)? {
            let previous = match positions {
                Positions::Allowed(bound) => self
                    .allowed(workspace, start, bound)?
                    .map(|(byte, _)| byte)
                    .take_while(|byte| *byte < end)
                    .last(),
                Positions::Graphemes(stop) => {
                    self.graphemes(start, stop)?.take_while(|g| *g < end).last()
                }
            };
            let Some(previous) = previous else {
                return Ok(None);
            };
            end = previous;
            back = true;
        }
        if back {
            return Ok(Some(end));
        }
        loop {
            let next = match positions {
                Positions::Allowed(bound) => self
                    .allowed(workspace, start, bound)?
                    .map(|(byte, _)| byte)
                    .find(|byte| *byte > end),
                Positions::Graphemes(stop) => self.graphemes(start, stop)?.find(|g| *g > end),
            };
            match next {
                Some(next) if self.fits(workspace, start, next, width)? => end = next,
                _ => break,
            }
        }
        Ok(Some(end))
    }
    /// Greedy line end after `start`.
    ///
    /// One shaped window per line estimates every candidate from its cluster
    /// advances; exact shaping checks the estimated candidate, its successor,
    /// and each candidate the estimate misses, O(w) each for w scalars.
    /// `scalars` is the first window's length; it doubles until the window
    /// overflows or reaches the paragraph end.
    fn choose(
        &mut self,
        workspace: &mut Workspace<'_>,
        start: usize,
        width: Option<Fixed>,
        scalars: usize,
    ) -> Result<usize, TextError> {
        let Some(width) = width else {
            return self
                .allowed(workspace, start, self.text.len())?
                .find(|(_, kind)| *kind == Break::Mandatory)
                .map(|(byte, _)| byte)
                .ok_or(TextError::InvalidInput);
        };
        let mut scalars = scalars.max(1);
        let (end, count, fit) = loop {
            let (end, complete) = self.window_end(workspace, start, scalars)?;
            let count = self
                .shape(workspace, start, end, false, Fixed::ZERO, 0)?
                .cluster_count;
            workspace
                .line_clusters
                .get_mut(..count)
                .ok_or(TextError::BufferTooSmall)?
                .sort_unstable_by_key(|c| c.start);
            let candidates = self.allowed(workspace, start, end)?.map(|(byte, _)| byte);
            let (fit, total) = self.estimate(workspace, start, end, count, candidates, width)?;
            if complete || total > width {
                break (end, count, fit);
            }
            scalars = scalars.checked_mul(2).ok_or(TextError::Overflow)?;
        };
        let first = self
            .allowed(workspace, start, end)?
            .next()
            .map(|(byte, _)| byte);
        let stop = first.unwrap_or(end);
        let graphemes = self.graphemes(start, stop)?;
        let (grapheme, _) = self.estimate(workspace, start, end, count, graphemes, width)?;
        if let Some(seed) = fit.or(first)
            && let Some(end) =
                self.settle(workspace, start, seed, width, Positions::Allowed(end))?
        {
            return Ok(end);
        }
        let fallback = self.graphemes(start, stop)?.next().unwrap_or(stop);
        Ok(match grapheme {
            Some(seed) => self
                .settle(workspace, start, seed, width, Positions::Graphemes(stop))?
                .unwrap_or(fallback),
            None => fallback,
        })
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
        let mut window = FIRST_WINDOW;
        let trailing = self.text.chars().next_back().is_some_and(hard);
        loop {
            let end = if start == self.text.len() {
                start
            } else {
                self.choose(workspace, start, width, window)?
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
            // Next window: 1.5 × this line's scalars + 2.
            let scalars = self
                .text
                .get(start..end)
                .ok_or(TextError::InvalidInput)?
                .chars()
                .count();
            window = scalars
                .saturating_add(scalars.div_ceil(2))
                .saturating_add(2);
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
