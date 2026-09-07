// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod css;
mod matrix;
mod number;
mod paint;
mod path;
mod render;

#[cfg(test)]
mod tests;

use doc_html::tree::{Element, Node};
use doc_pdf::font::Font;
use doc_pdf::page::{Fill, Segment, Stroke};
use doc_pdf::units::{Color, Mils};

/// A length in thousandths of a user unit: the unit an SVG is written in,
/// counted the way `doc-pdf` counts a point.
pub type Unit = i64;

/// One mark of a drawing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// A path, filled, stroked, or both.
    Path {
        /// What it is made of.
        segments: Vec<Segment>,
        /// How it is filled, if it is.
        fill: Option<Fill>,
        /// How it is stroked, if it is.
        stroke: Option<Stroke>,
    },
    /// A line of text, at the place its anchor put it.
    Text {
        /// Where it starts.
        x: Unit,
        /// The baseline.
        y: Unit,
        /// The size it is set at.
        size: Unit,
        /// The face it is set in.
        font: Font,
        /// The colour.
        color: Color,
        /// What it says.
        text: String,
    },
}

/// A drawing, in its own units, with the origin at its bottom left corner
/// and `y` growing upwards as PDF counts it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drawing {
    /// How wide the drawing is.
    pub width: Unit,
    /// How tall it is.
    pub height: Unit,
    /// What is on it.
    pub items: Vec<Item>,
}

/// Reads an SVG.
///
/// Anything that is not an SVG — or is one this crate can make no marks
/// out of — is `None`, which is a caller's cue to say what the picture
/// was instead of drawing it.
#[must_use]
pub fn parse(svg: &str) -> Option<Drawing> {
    let nodes = doc_html::tree::parse(svg);
    let root = find(&nodes)?;
    let drawing = render::drawing(root);
    (!drawing.items.is_empty()).then_some(drawing)
}

/// The `svg` element of a document, wherever it stands.
fn find(nodes: &[Node]) -> Option<&Element> {
    for node in nodes {
        if let Node::Element(element) = node {
            if element.name == "svg" {
                return Some(element);
            }
            if let Some(found) = find(&element.children) {
                return Some(found);
            }
        }
    }
    None
}

impl Drawing {
    /// How tall the drawing is when it is drawn `width` wide.
    #[must_use]
    pub const fn height_at(&self, width: Mils) -> Mils {
        self.map(self.height, width)
    }

    /// The drawing, scaled to `width` and moved so that its bottom left
    /// corner sits at `(x, y)`.
    #[must_use]
    pub fn placed(&self, x: Mils, y: Mils, width: Mils) -> Vec<Item> {
        self.items
            .iter()
            .map(|item| match item {
                Item::Path {
                    segments,
                    fill,
                    stroke,
                } => Item::Path {
                    segments: segments
                        .iter()
                        .map(|segment| self.segment(*segment, x, y, width))
                        .collect(),
                    fill: *fill,
                    stroke: stroke.as_ref().map(|stroke| Stroke {
                        color: stroke.color,
                        width: self.map(stroke.width, width).max(100),
                        dashes: stroke
                            .dashes
                            .iter()
                            .map(|dash| self.map(*dash, width))
                            .collect(),
                    }),
                },
                Item::Text {
                    x: text_x,
                    y: text_y,
                    size,
                    font,
                    color,
                    text,
                } => Item::Text {
                    x: x.saturating_add(self.map(*text_x, width)),
                    y: y.saturating_add(self.map(*text_y, width)),
                    size: self.map(*size, width),
                    font: *font,
                    color: *color,
                    text: text.clone(),
                },
            })
            .collect()
    }

    /// One segment, scaled and moved.
    fn segment(&self, segment: Segment, x: Mils, y: Mils, width: Mils) -> Segment {
        let place = |px: Unit, py: Unit| {
            (
                x.saturating_add(self.map(px, width)),
                y.saturating_add(self.map(py, width)),
            )
        };
        match segment {
            Segment::Move { x: px, y: py } => {
                let (x, y) = place(px, py);
                Segment::Move { x, y }
            }
            Segment::Line { x: px, y: py } => {
                let (x, y) = place(px, py);
                Segment::Line { x, y }
            }
            Segment::Curve {
                x1,
                y1,
                x2,
                y2,
                x: px,
                y: py,
            } => {
                let (x1, y1) = place(x1, y1);
                let (x2, y2) = place(x2, y2);
                let (x, y) = place(px, py);
                Segment::Curve {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                }
            }
            Segment::Close => Segment::Close,
        }
    }

    /// A length of the drawing, as a length on the page. A drawing of no
    /// width has no lengths, and the division says so itself.
    const fn map(&self, length: Unit, width: Mils) -> Mils {
        match length.saturating_mul(width).checked_div(self.width) {
            Some(value) => value,
            None => 0,
        }
    }
}
