// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The measurements of a page.
//!
//! One place for every number a reader would notice: the margins, the type
//! sizes, the leading, the colours. Everything here is in thousandths of a
//! point, and a size is always paired with the line height it is set on,
//! because a size without its leading is half a decision.

use doc_pdf::units::{Color, Mils, PageSize, pt};

/// A type size and the line height it is set on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Size {
    /// The point size.
    pub(crate) size: Mils,
    /// The distance from one baseline to the next.
    pub(crate) leading: Mils,
}

impl Size {
    /// A size and its leading.
    const fn new(size: Mils, leading: Mils) -> Self {
        Self { size, leading }
    }
}

/// The page.
pub(crate) const PAGE: PageSize = PageSize::A4;
/// The margin on the left and on the right.
pub(crate) const SIDE: Mils = pt(64);
/// The margin at the top of the text.
pub(crate) const TOP: Mils = pt(72);
/// The margin at the bottom of the text.
pub(crate) const BOTTOM: Mils = pt(64);
/// The width a line of prose may take.
pub(crate) const MEASURE: Mils = PAGE.width.saturating_sub(SIDE.saturating_mul(2));

/// Body text.
pub(crate) const BODY: Size = Size::new(pt(10), 14_200);
/// The first-level heading, which is also the title of a document.
pub(crate) const TITLE: Size = Size::new(pt(21), pt(26));
/// A second-level heading.
pub(crate) const HEADING_2: Size = Size::new(pt(15), pt(19));
/// A third-level heading.
pub(crate) const HEADING_3: Size = Size::new(12_500, pt(16));
/// A heading below the third level.
pub(crate) const HEADING_4: Size = Size::new(pt(11), pt(14));
/// Code, in a fixed-pitch face.
pub(crate) const CODE: Size = Size::new(8_600, 11_600);
/// The running head and the page number.
pub(crate) const FURNITURE: Size = Size::new(pt(8), pt(10));
/// A line of an RFC, whose text is seventy-two columns of fixed pitch.
pub(crate) const RFC: Size = Size::new(9_200, 11_400);

/// The space before a heading of the given level.
pub(crate) const fn space_before(level: usize) -> Mils {
    match level {
        0 | 1 => pt(20),
        2 => pt(16),
        3 => pt(12),
        _ => pt(10),
    }
}

/// The space after a heading of the given level.
pub(crate) const fn space_after(level: usize) -> Mils {
    match level {
        0 | 1 => pt(8),
        2 => pt(6),
        _ => pt(4),
    }
}

/// The size of a heading of the given level.
pub(crate) const fn heading(level: usize) -> Size {
    match level {
        0 | 1 => TITLE,
        2 => HEADING_2,
        3 => HEADING_3,
        _ => HEADING_4,
    }
}

/// The space between two blocks.
pub(crate) const PARAGRAPH: Mils = pt(9);
/// The space between the items of a list written without blank lines.
pub(crate) const TIGHT: Mils = pt(2);
/// How far a list or a quotation is indented.
pub(crate) const INDENT: Mils = pt(18);
/// The padding inside a code block.
pub(crate) const PADDING: Mils = pt(7);

/// Text.
pub(crate) const INK: Color = Color::gray(26);
/// A heading.
pub(crate) const HEADING_INK: Color = Color::rgb(17, 17, 17);
/// Something said quietly: the running head, a caption, a source path.
pub(crate) const QUIET: Color = Color::gray(128);
/// A link.
pub(crate) const LINK: Color = Color::rgb(20, 70, 140);
/// A code span, and code inside a block.
pub(crate) const CODE_INK: Color = Color::rgb(40, 50, 70);
/// The ground a code block sits on.
pub(crate) const CODE_GROUND: Color = Color::gray(244);
/// A rule, a table border, the bar beside a quotation.
pub(crate) const RULE: Color = Color::gray(200);
/// The ground of a table header.
pub(crate) const TABLE_GROUND: Color = Color::gray(238);
