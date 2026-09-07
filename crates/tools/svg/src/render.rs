// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The tree of an SVG, walked into marks.
//!
//! One pass collects what the rest of the drawing refers to — the
//! stylesheet, and every element that carries an `id`, which is what
//! `use` and a marker are found by. The second pass walks the tree with
//! two things in hand: the state an element inherits, and the transform it
//! sits under. Both are made anew for every element from its parent's, so
//! nothing has to be undone on the way back up.
//!
//! The transform starts by turning the drawing over. SVG counts down the
//! page and PDF counts up it, so the first thing every point goes through
//! is the matrix that flips it. Text is placed by that same matrix and
//! then set upright, because a mirrored letter is not what anybody meant
//! by a mirrored coordinate system.

use std::collections::BTreeMap;

use doc_html::tree::{Element, Node};
use doc_pdf::page::{Fill, Segment, Stroke};

use crate::css::Sheet;
use crate::matrix::Matrix;
use crate::number::{self, ONE};
use crate::paint::{Anchor, State};
use crate::{Drawing, Item, Unit, path};

/// The value of an attribute, whatever case the drawing wrote its name
/// in. The markup reader lower-cases every name it reads, and SVG writes
/// `viewBox`, `refX`, and `markerUnits` in the case a schema gave them;
/// asking for one under the name it is written in is what a reader of
/// this crate will do, so the lower-casing happens here rather than at
/// every call.
fn attribute<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    element.attribute(&name.to_ascii_lowercase())
}

/// The elements that carry no marks of their own.
const SILENT: [&str; 8] = [
    "defs", "style", "title", "desc", "metadata", "marker", "clippath", "mask",
];

/// A drawing under construction.
struct Renderer<'a> {
    /// The stylesheet of the drawing.
    sheet: Sheet,
    /// Every element that carries an `id`.
    named: BTreeMap<String, &'a Element>,
    /// The marks so far.
    items: Vec<Item>,
    /// How deep the walk is, so that a document that refers to itself
    /// cannot run for ever.
    depth: usize,
}

/// The drawing an `svg` element stands for.
pub(crate) fn drawing(root: &Element) -> Drawing {
    let (offset_x, offset_y, width, height) = box_of(root);
    let mut renderer = Renderer {
        sheet: Sheet::parse(&stylesheet(root, false)),
        named: BTreeMap::new(),
        items: Vec::new(),
        depth: 0,
    };
    renderer.collect(root);
    // Move the view box to the origin, then turn the drawing over.
    let base = Matrix::new(
        ONE,
        0,
        0,
        ONE.saturating_neg(),
        offset_x.saturating_neg(),
        height.saturating_add(offset_y),
    );
    renderer.children(root, &State::default(), base);
    Drawing {
        width,
        height,
        items: renderer.items,
    }
}

impl<'a> Renderer<'a> {
    /// Notes every element that carries an `id`.
    fn collect(&mut self, element: &'a Element) {
        if let Some(id) = attribute(element, "id") {
            self.named.entry(id.to_owned()).or_insert(element);
        }
        for node in &element.children {
            if let Node::Element(child) = node {
                self.collect(child);
            }
        }
    }

    /// Walks the children of an element.
    fn children(&mut self, element: &'a Element, state: &State, matrix: Matrix) {
        for node in &element.children {
            if let Node::Element(child) = node {
                self.element(child, state, matrix);
            }
        }
    }

    /// Walks one element.
    fn element(&mut self, element: &'a Element, parent: &State, matrix: Matrix) {
        let name = element.name.as_str();
        if SILENT.contains(&name) {
            return;
        }
        let state = self.resolve(element, parent);
        let local = attribute(element, "transform")
            .map_or(matrix, |text| crate::matrix::parse(text).then(matrix));
        match name {
            "text" => self.text(element, &state, local),
            "use" => self.reuse(element, &state, local),
            "svg" | "g" | "a" | "switch" => self.children(element, &state, local),
            _ => {
                let segments = Self::shape(element, local);
                if segments.is_empty() {
                    self.children(element, &state, local);
                } else {
                    self.stroke_and_fill(&segments, &state, local);
                }
            }
        }
    }

    /// The state an element draws under: what it inherits, what its own
    /// attributes say, what the stylesheet says, and what its `style`
    /// attribute says, in that order.
    fn resolve(&self, element: &Element, parent: &State) -> State {
        let mut state = parent.clone();
        state.marker_end = None;
        for (property, value) in &element.attributes {
            state.apply(property, value);
        }
        let classes = attribute(element, "class").unwrap_or_default();
        for (property, value) in self.sheet.declarations(&element.name, classes) {
            state.apply(property, value);
        }
        if let Some(inline) = attribute(element, "style") {
            for declaration in inline.split(';') {
                if let Some((property, value)) = declaration.split_once(':') {
                    state.apply(property.trim().to_ascii_lowercase().as_str(), value);
                }
            }
        }
        state
    }

    /// The segments of a shape, already transformed. A shape this crate
    /// does not draw has none.
    fn shape(element: &Element, matrix: Matrix) -> Vec<Segment> {
        let at = |name: &str| {
            attribute(element, name)
                .and_then(number::value)
                .unwrap_or(0)
        };
        let radius = |name: &str, other: &str| {
            attribute(element, name)
                .and_then(number::value)
                .or_else(|| attribute(element, other).and_then(number::value))
                .unwrap_or(0)
        };
        let segments = match element.name.as_str() {
            "path" => attribute(element, "d").map(path::data).unwrap_or_default(),
            "rect" => path::rectangle(
                at("x"),
                at("y"),
                at("width"),
                at("height"),
                radius("rx", "ry"),
                radius("ry", "rx"),
            ),
            "circle" => path::ellipse(at("cx"), at("cy"), at("r"), at("r")),
            "ellipse" => path::ellipse(at("cx"), at("cy"), at("rx"), at("ry")),
            "line" => path::polygon(&[at("x1"), at("y1"), at("x2"), at("y2")], false),
            "polygon" | "polyline" => {
                let points = attribute(element, "points")
                    .map(number::list)
                    .unwrap_or_default();
                path::polygon(&points, element.name == "polygon")
            }
            _ => Vec::new(),
        };
        transform(&segments, matrix)
    }

    /// Adds a path, and whatever the end of it carries.
    fn stroke_and_fill(&mut self, segments: &[Segment], state: &State, matrix: Matrix) {
        let scale = matrix.scale();
        let stroke = state.stroke.map(|color| Stroke {
            color,
            width: state.width.saturating_mul(scale).wrapping_div(ONE).max(1),
            dashes: state
                .dashes
                .iter()
                .map(|dash| dash.saturating_mul(scale).wrapping_div(ONE))
                .collect(),
        });
        self.items.push(Item::Path {
            segments: segments.to_vec(),
            fill: state.fill.map(|color| Fill {
                color,
                even_odd: state.even_odd,
            }),
            stroke,
        });
        if let Some(name) = state.marker_end.clone() {
            self.marker(&name, segments, state, scale);
        }
    }

    /// Draws the marker at the end of a path, turned to point the way the
    /// path was going.
    fn marker(&mut self, name: &str, segments: &[Segment], state: &State, scale: Unit) {
        let Some(marker) = self.named.get(name).copied() else {
            return;
        };
        let Some(((x, y), direction)) = ending(segments) else {
            return;
        };
        let (view_x, view_y, view_width, view_height) = box_of(marker);
        let size = |name: &str, fallback: Unit| {
            attribute(marker, name)
                .and_then(number::value)
                .unwrap_or(fallback)
        };
        let unit = if attribute(marker, "markerUnits") == Some("userSpaceOnUse") {
            ONE
        } else {
            state.width.saturating_mul(scale).wrapping_div(ONE)
        };
        let across = ratio(size("markerWidth", ONE.saturating_mul(3)), view_width, unit);
        let down = ratio(
            size("markerHeight", ONE.saturating_mul(3)),
            view_height,
            unit,
        );
        let reference = Matrix::translation(
            size("refX", 0).saturating_add(view_x).saturating_neg(),
            size("refY", 0).saturating_add(view_y).saturating_neg(),
        );
        // The marker is drawn in the space the path is already in, which
        // has been turned over once; turning it over again keeps its own
        // drawing the right way up.
        let placed = reference
            .then(Matrix::scaling(across, down.saturating_neg()))
            .then(direction)
            .then(Matrix::translation(x, y));
        self.depth = self.depth.saturating_add(1);
        let inherited = State {
            fill: state.stroke.or(state.fill),
            ..State::default()
        };
        self.children(marker, &inherited, placed);
        self.depth = self.depth.saturating_sub(1);
    }

    /// Draws what a `use` points at, where it points at it.
    fn reuse(&mut self, element: &'a Element, state: &State, matrix: Matrix) {
        let Some(target) = attribute(element, "xlink:href")
            .or_else(|| attribute(element, "href"))
            .and_then(|href| href.strip_prefix('#'))
            .and_then(|name| self.named.get(name).copied())
        else {
            return;
        };
        let offset = Matrix::translation(
            attribute(element, "x").and_then(number::value).unwrap_or(0),
            attribute(element, "y").and_then(number::value).unwrap_or(0),
        );
        self.depth = self.depth.saturating_add(1);
        if self.depth <= 16 {
            // What is used is drawn where the `use` is, and a `g` in the
            // definitions is drawn for its children rather than skipped.
            let placed = offset.then(matrix);
            match target.name.as_str() {
                "g" | "svg" | "symbol" => self.children(target, state, placed),
                _ => self.element(target, state, placed),
            }
        }
        self.depth = self.depth.saturating_sub(1);
    }

    /// Adds a line of text, where its anchor puts it.
    fn text(&mut self, element: &Element, state: &State, matrix: Matrix) {
        let text = words(element);
        if text.is_empty() {
            return;
        }
        let Some(color) = state.fill else {
            return;
        };
        let (x, y) = matrix.apply(
            attribute(element, "x").and_then(number::value).unwrap_or(0),
            attribute(element, "y").and_then(number::value).unwrap_or(0),
        );
        let size = state.size.saturating_mul(matrix.scale()).wrapping_div(ONE);
        let width = state.font.width_of_str(&text, size);
        let x = match state.anchor {
            Anchor::Start => x,
            Anchor::Middle => x.saturating_sub(width.wrapping_div(2)),
            Anchor::End => x.saturating_sub(width),
        };
        self.items.push(Item::Text {
            x,
            y,
            size,
            font: state.font,
            color,
            text,
        });
    }
}

/// The share one length is of another, times a scale. A whole of nothing
/// has no shares, and the division says so itself.
const fn ratio(length: Unit, whole: Unit, unit: Unit) -> Unit {
    match length.saturating_mul(unit).checked_div(whole) {
        Some(value) => value,
        None => unit,
    }
}

/// The text inside an element, with its whitespace collapsed.
fn words(element: &Element) -> String {
    let mut out = String::new();
    gather(element, &mut out);
    out.split_whitespace().collect::<Vec<&str>>().join(" ")
}

/// Every piece of text below an element.
fn gather(element: &Element, out: &mut String) {
    for node in &element.children {
        match node {
            Node::Text(text) => out.push_str(text),
            Node::Element(child) => {
                out.push(' ');
                gather(child, out);
            }
        }
    }
}

/// The stylesheet of a drawing: the text of every `style` element it
/// carries, and none of the text around them.
fn stylesheet(element: &Element, inside: bool) -> String {
    let inside = inside || element.name == "style";
    let mut out = String::new();
    for node in &element.children {
        match node {
            Node::Text(text) if inside => out.push_str(text),
            Node::Text(_) => {}
            Node::Element(child) => out.push_str(&stylesheet(child, inside)),
        }
    }
    out
}

/// The view box of an element: where it starts and how large it is. An
/// element without one is as large as its `width` and `height` say, and a
/// drawing that says neither is a square of a thousand units, which is
/// what a viewer would show it as.
fn box_of(element: &Element) -> (Unit, Unit, Unit, Unit) {
    if let Some(values) = attribute(element, "viewBox").map(number::list)
        && let (Some(x), Some(y), Some(width), Some(height)) = (
            values.first().copied(),
            values.get(1).copied(),
            values.get(2).copied(),
            values.get(3).copied(),
        )
        && width > 0
        && height > 0
    {
        return (x, y, width, height);
    }
    let side = |name: &str| {
        attribute(element, name)
            .and_then(number::value)
            .filter(|value| *value > 0)
            .unwrap_or(ONE.saturating_mul(1000))
    };
    (0, 0, side("width"), side("height"))
}

/// Every segment, moved through a transform.
fn transform(segments: &[Segment], matrix: Matrix) -> Vec<Segment> {
    segments
        .iter()
        .map(|segment| match *segment {
            Segment::Move { x, y } => {
                let (x, y) = matrix.apply(x, y);
                Segment::Move { x, y }
            }
            Segment::Line { x, y } => {
                let (x, y) = matrix.apply(x, y);
                Segment::Line { x, y }
            }
            Segment::Curve {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let (x1, y1) = matrix.apply(x1, y1);
                let (x2, y2) = matrix.apply(x2, y2);
                let (x, y) = matrix.apply(x, y);
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
        })
        .collect()
}

/// Where a path ends, and the turn that points along its last stretch.
fn ending(segments: &[Segment]) -> Option<((Unit, Unit), Matrix)> {
    let points: Vec<(Unit, Unit)> = segments
        .iter()
        .filter_map(|segment| match *segment {
            Segment::Move { x, y } | Segment::Line { x, y } | Segment::Curve { x, y, .. } => {
                Some((x, y))
            }
            Segment::Close => None,
        })
        .collect();
    let last = points.last().copied()?;
    let before = points
        .iter()
        .rev()
        .find(|point| **point != last)
        .copied()
        .unwrap_or(last);
    let (dx, dy) = (
        last.0.saturating_sub(before.0),
        last.1.saturating_sub(before.1),
    );
    let length = magnitude(dx, dy);
    if length == 0 {
        return Some((last, Matrix::IDENTITY));
    }
    let unit = |value: Unit| {
        value
            .saturating_mul(ONE)
            .checked_div(length)
            .unwrap_or_default()
    };
    let (ux, uy) = (unit(dx), unit(dy));
    Some((last, Matrix::new(ux, uy, uy.saturating_neg(), ux, 0, 0)))
}

/// The length of a vector, to the nearest thousandth.
fn magnitude(x: Unit, y: Unit) -> Unit {
    let squared = x
        .saturating_mul(x)
        .saturating_add(y.saturating_mul(y))
        .unsigned_abs();
    Unit::try_from(squared.isqrt()).unwrap_or(0)
}
