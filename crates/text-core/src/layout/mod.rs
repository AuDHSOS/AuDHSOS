// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Fractional line layout, cursor geometry, and deterministic serialization.
mod encode;
mod engine;
mod line;
#[cfg(feature = "alloc")]
mod owned;
mod queries;
use crate::{Fixed, Role, bidi, glyf, resolve::Run, segment, shape, variation::VariationPoint};
pub use engine::{layout_into, measure_into};
#[cfg(feature = "alloc")]
pub use owned::{Layout, layout, measure};

/// Largest accepted text input in Unicode scalars.
pub const MAX_SCALARS: usize = 65_536;
/// Numeric placement in output units, with positive y downward.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PositionedGlyph {
    /// Index into the layout's resolved runs.
    pub run: usize,
    /// Font glyph index.
    pub id: u16,
    /// Original cluster start byte.
    pub start: usize,
    /// Original cluster end byte.
    pub end: usize,
    /// Horizontal glyph origin including positioning adjustments.
    pub x: Fixed,
    /// Vertical glyph origin including positioning adjustments.
    pub y: Fixed,
    /// Fractional horizontal advance.
    pub advance: Fixed,
    /// Line-resolved embedding level.
    pub level: u8,
    /// Suppress ink; a tab can retain a nonzero advance.
    pub hidden: bool,
    pen: Fixed,
    rank: usize,
    ordinal: usize,
}
/// One visual line, including source hard-break bytes when present.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Line {
    /// Inclusive source byte start.
    pub start: usize,
    /// Exclusive source byte end.
    pub end: usize,
    /// First visual glyph index.
    pub glyph_start: usize,
    /// Number of visual glyphs.
    pub glyph_count: usize,
    /// First visual cluster-box index.
    pub cluster_start: usize,
    /// Number of cluster boxes.
    pub cluster_count: usize,
    /// Top coordinate.
    pub y: Fixed,
    /// Line height, including line gap.
    pub height: Fixed,
    /// Alphabetic baseline coordinate.
    pub baseline: Fixed,
    /// Advance-box width.
    pub width: Fixed,
    /// A whole grapheme exceeds the requested width.
    pub overflow: bool,
}
/// One original grapheme's interval on a visual line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClusterBox {
    /// Inclusive UTF-8 start.
    pub start: usize,
    /// Exclusive UTF-8 end.
    pub end: usize,
    /// Visual line index.
    pub line: usize,
    /// Left edge of its advance interval.
    pub left: Fixed,
    /// Right edge of its advance interval.
    pub right: Fixed,
    /// Line-resolved embedding level.
    pub level: u8,
}
/// Which neighboring cluster gives a cursor its visual position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Affinity {
    /// Position after the preceding logical cluster.
    Upstream,
    /// Position before the following logical cluster.
    #[default]
    Downstream,
}
/// A cursor position at a grapheme boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    /// Original UTF-8 boundary.
    pub byte: usize,
    /// Visual line index.
    pub line: usize,
    /// Horizontal coordinate.
    pub x: Fixed,
    /// Top coordinate.
    pub y: Fixed,
    /// Cursor height.
    pub height: Fixed,
    /// Logical affinity at bidi and line boundaries.
    pub affinity: Affinity,
}
/// A visual selection rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: Fixed,
    /// Top edge.
    pub y: Fixed,
    /// Width.
    pub width: Fixed,
    /// Height.
    pub height: Fixed,
}
/// Used buffer lengths and the resulting advance box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutInfo {
    /// Maximum line width.
    pub width: Fixed,
    /// Sum of line heights.
    pub height: Fixed,
    /// Input UTF-8 byte length.
    pub text_len: usize,
    /// Font snapshot identifier.
    pub generation: u64,
    /// Role whose face indices are used.
    pub role: Role,
    /// Initialized run count.
    pub runs: usize,
    /// Initialized line count.
    pub lines: usize,
    /// Initialized glyph count.
    pub glyphs: usize,
    /// Initialized cluster count.
    pub clusters: usize,
}
/// Borrowed successful output; scratch changes invalidate this view.
#[derive(Clone, Copy, Debug)]
pub struct LayoutView<'a> {
    /// Box, generation, and used lengths.
    pub info: LayoutInfo,
    /// Logical resolved runs and explicit instance metadata.
    pub runs: &'a [Run],
    /// Visual lines.
    pub lines: &'a [Line],
    /// Glyphs in visual order within each line.
    pub glyphs: &'a [PositionedGlyph],
    /// Grapheme boxes in visual order within each line.
    pub clusters: &'a [ClusterBox],
}
/// Caller-provided persistent output storage.
pub struct LayoutBuffers<'a> {
    /// One slot per resulting line.
    pub lines: &'a mut [Line],
    /// One slot per resulting glyph.
    pub glyphs: &'a mut [PositionedGlyph],
    /// One slot per original grapheme.
    pub clusters: &'a mut [ClusterBox],
}
/// Caller-provided Unicode, shaping, outline, and candidate-line workspace.
pub struct Workspace<'a> {
    /// One record per input scalar.
    pub bidi: &'a mut [bidi::Unit],
    /// One index per input scalar.
    pub indices: &'a mut [usize],
    /// One line level per input scalar.
    pub levels: &'a mut [u8],
    /// One visual rank per input scalar.
    pub ranks: &'a mut [usize],
    /// One line-break scratch record per input scalar.
    pub line_units: &'a mut [segment::LineUnit],
    /// Scalar count plus one boundary records.
    pub breaks: &'a mut [segment::LineBoundary],
    /// At most one resolved run per grapheme.
    pub runs: &'a mut [Run],
    /// Largest expanded shaping run, including canonical decomposition.
    pub glyphs: &'a mut [shape::Glyph],
    /// Largest shaped candidate line.
    pub line_glyphs: &'a mut [PositionedGlyph],
    /// Largest candidate line's original graphemes.
    pub line_clusters: &'a mut [ClusterBox],
    /// Largest requested glyph's unhinted outline points.
    pub points: &'a mut [glyf::Point],
    /// Largest requested glyph's contour ends.
    pub contours: &'a mut [usize],
    /// Variation outline scratch, including active composite ancestors.
    pub variation: &'a mut [VariationPoint],
    /// Largest requested ligature's caret records.
    pub caret_values: &'a mut [shape::Caret],
}
