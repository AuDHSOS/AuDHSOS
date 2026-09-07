// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One page and the marks on it.
//!
//! A page holds its content stream as it is built, operator by operator.
//! The coordinate system is the one PDF itself uses: the origin sits at the
//! bottom left corner and `y` grows upwards. Whoever lays out a document
//! usually counts downwards from the top, and converts once, at the edge,
//! rather than everywhere.
//!
//! What the page keeps besides the operators is what the stream is
//! currently set to: the fill and stroke colours, the face and size text is
//! being set in, and where the open text object last began a line. Nothing
//! is written twice that is already true, and a line of text costs what it
//! says and little more. A page of prose is one text object, opened at the
//! first word and closed by whatever needs the pen back — a path, a rule,
//! or the end of the page — and every run inside it moves by the distance
//! from the last one rather than by naming a matrix again.

use crate::font::{self, Font};
use crate::units::{Color, Mils, PageSize, number};

/// Where a link goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    /// An address outside the document.
    Uri(String),
    /// A place inside the document: the page, and the height on it.
    Page {
        /// Index of the page in the document.
        index: usize,
        /// The height the view is scrolled to.
        top: Mils,
    },
}

/// A clickable rectangle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The left edge.
    pub x: Mils,
    /// The bottom edge.
    pub y: Mils,
    /// The width.
    pub width: Mils,
    /// The height.
    pub height: Mils,
    /// Where it goes.
    pub target: LinkTarget,
}

/// One piece of a path, in the coordinate system of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Segment {
    /// Begins a subpath at a point.
    Move {
        /// Where it begins.
        x: Mils,
        /// Where it begins.
        y: Mils,
    },
    /// A straight line to a point.
    Line {
        /// Where it goes.
        x: Mils,
        /// Where it goes.
        y: Mils,
    },
    /// A cubic curve to a point, with its two control points first.
    Curve {
        /// The control point of the start.
        x1: Mils,
        /// The control point of the start.
        y1: Mils,
        /// The control point of the end.
        x2: Mils,
        /// The control point of the end.
        y2: Mils,
        /// Where it goes.
        x: Mils,
        /// Where it goes.
        y: Mils,
    },
    /// Closes the subpath, back to where it began.
    Close,
}

/// How a path is filled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fill {
    /// The colour.
    pub color: Color,
    /// Whether a point is inside by the even-odd rule rather than by the
    /// non-zero winding rule.
    pub even_odd: bool,
}

/// How a path is stroked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stroke {
    /// The colour.
    pub color: Color,
    /// The width of the line.
    pub width: Mils,
    /// The dashes, on and off in turn; empty for a solid line.
    pub dashes: Vec<Mils>,
}

/// A page and its content stream.
#[derive(Clone, Debug)]
pub struct Page {
    /// The size of the page.
    size: PageSize,
    /// The content stream, as far as it has been built.
    content: Vec<u8>,
    /// The links on the page.
    links: Vec<Link>,
    /// The colour the stream is currently filling with, so that a run of
    /// text in one colour does not set it again for every line.
    fill: Option<Color>,
    /// The colour the stream is currently stroking with, for the same
    /// reason.
    stroke: Option<Color>,
    /// The face and size text is currently set in, which outlives a text
    /// object because it belongs to the graphics state.
    face: Option<(Font, Mils)>,
    /// Where the open text object last began a line. A text object begins
    /// with no line at all, which is why the first run of one names its
    /// place outright and every run after it says how far it moved.
    line: Option<(Mils, Mils)>,
    /// Where the pen stands after the last run: a run that begins exactly
    /// there needs no move at all, because showing text is itself a move.
    pen: Option<(Mils, Mils)>,
    /// The distance between two lines the stream is currently set to, so
    /// that a paragraph after the first of them is one string per line.
    leading: Option<Mils>,
    /// Whether a text object is open.
    typesetting: bool,
}

impl Page {
    /// An empty page.
    #[must_use]
    pub const fn new(size: PageSize) -> Self {
        Self {
            size,
            content: Vec::new(),
            links: Vec::new(),
            fill: None,
            stroke: None,
            face: None,
            line: None,
            pen: None,
            leading: None,
            typesetting: false,
        }
    }

    /// The size of the page.
    #[must_use]
    pub const fn size(&self) -> PageSize {
        self.size
    }

    /// The content stream.
    #[must_use]
    pub fn content(&self) -> &[u8] {
        &self.content
    }

    /// The links on the page.
    #[must_use]
    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// Whether nothing has been drawn yet.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Sets `text` at `x`, with `y` the baseline.
    ///
    /// A line is not one string. Whatever `WinAnsiEncoding` cannot say is
    /// set in the Symbol font instead, so the text goes down as the runs
    /// its characters need, each placed at the width of everything before
    /// it.
    pub fn text(&mut self, font: Font, size: Mils, x: Mils, y: Mils, color: Color, text: &str) {
        let mut at = x;
        for run in font::runs(font, text) {
            let width = run.font.width_of(&run.bytes, size);
            self.text_encoded(run.font, size, at, y, color, &run.bytes);
            at = at.saturating_add(width);
        }
    }

    /// Sets already encoded text at `x`, with `y` the baseline.
    ///
    /// The text object stays open for whatever comes next and the face is
    /// named only when it changes. How the place is given depends on where
    /// the pen already stands. A run that begins exactly where the last one
    /// ended needs no move at all: showing text moves the pen by the width
    /// of what was shown, which is the same number this crate measured with.
    /// A run that begins a new line at the distance the stream is already
    /// set to is shown with the operator that means *next line, then this*.
    /// Anything else moves by the distance from the last line, and only the
    /// first run of a text object names a matrix.
    pub fn text_encoded(
        &mut self,
        font: Font,
        size: Mils,
        x: Mils,
        y: Mils,
        color: Color,
        encoded: &[u8],
    ) {
        if encoded.is_empty() {
            return;
        }
        self.set_fill(color);
        self.begin_text();
        if self.face != Some((font, size)) {
            self.push(&format!("/{} {} Tf\n", font.resource(), number(size)));
            self.face = Some((font, size));
        }
        let show = self.place(x, y);
        self.content.push(b'(');
        for byte in encoded {
            escape(*byte, &mut self.content);
        }
        self.push(show);
        // The pen has moved by what was shown. Where the width came from a
        // table this crate filled in by hand — Symbol, and only Symbol —
        // the next run says its place outright rather than trusting it.
        self.pen = if font == Font::Symbol {
            None
        } else {
            Some((x.saturating_add(font.width_of(encoded, size)), y))
        };
    }

    /// Moves the pen to where the next run is set, and answers with the
    /// operator that shows it.
    fn place(&mut self, x: Mils, y: Mils) -> &'static str {
        if self.pen == Some((x, y)) {
            return ") Tj\n";
        }
        let Some((from_x, from_y)) = self.line else {
            self.push(&format!("1 0 0 1 {} {} Tm\n", number(x), number(y)));
            self.line = Some((x, y));
            return ") Tj\n";
        };
        let down = from_y.saturating_sub(y);
        if x == from_x && down > 0 {
            if self.leading != Some(down) {
                self.push(&format!("{} TL\n", number(down)));
                self.leading = Some(down);
            }
            self.line = Some((x, y));
            return ")'\n";
        }
        self.push(&format!(
            "{} {} Td\n",
            number(x.saturating_sub(from_x)),
            number(y.saturating_sub(from_y))
        ));
        self.line = Some((x, y));
        ") Tj\n"
    }

    /// Opens a text object, unless one is open.
    fn begin_text(&mut self) {
        if self.typesetting {
            return;
        }
        self.push("BT\n");
        self.typesetting = true;
        // The text and line matrices are the identity again inside a new
        // text object, so the next run has nothing to move from and the pen
        // stands nowhere. The leading is not reset: it belongs to the
        // graphics state and outlives the object, as the face does.
        self.line = None;
        self.pen = None;
    }

    /// Closes the open text object, if there is one.
    ///
    /// Everything that is not text does this first: PDF allows no path
    /// inside a text object. So does whoever finishes the page, which is
    /// [`crate::Document::push`] — a stream that ended mid-sentence would
    /// be a stream no viewer accepts.
    pub fn end_text(&mut self) {
        if !self.typesetting {
            return;
        }
        self.push("ET\n");
        self.typesetting = false;
        self.line = None;
        self.pen = None;
    }

    /// Fills a rectangle.
    pub fn rect(&mut self, x: Mils, y: Mils, width: Mils, height: Mils, color: Color) {
        if width <= 0 || height <= 0 {
            return;
        }
        self.end_text();
        self.set_fill(color);
        self.push(&format!(
            "{} {} {} {} re f\n",
            number(x),
            number(y),
            number(width),
            number(height)
        ));
    }

    /// Draws a path, filled, stroked, or both.
    ///
    /// A path with neither is not drawn: PDF has an operator for that
    /// (`n`, which only ends the path) and no reason to write one, since
    /// nothing would appear.
    pub fn path(&mut self, segments: &[Segment], fill: Option<Fill>, stroke: Option<&Stroke>) {
        if segments.is_empty() || (fill.is_none() && stroke.is_none()) {
            return;
        }
        self.end_text();
        if let Some(fill) = fill {
            self.set_fill(fill.color);
        }
        if let Some(stroke) = stroke {
            self.set_stroke(stroke.color);
            self.push(&format!("{} w\n", number(stroke.width.max(0))));
            let dashes: Vec<String> = stroke.dashes.iter().map(|dash| number(*dash)).collect();
            self.push(&format!("[{}] 0 d\n", dashes.join(" ")));
        }
        for segment in segments {
            match *segment {
                Segment::Move { x, y } => {
                    self.push(&format!("{} {} m\n", number(x), number(y)));
                }
                Segment::Line { x, y } => {
                    self.push(&format!("{} {} l\n", number(x), number(y)));
                }
                Segment::Curve {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                } => {
                    self.push(&format!(
                        "{} {} {} {} {} {} c\n",
                        number(x1),
                        number(y1),
                        number(x2),
                        number(y2),
                        number(x),
                        number(y)
                    ));
                }
                Segment::Close => self.push("h\n"),
            }
        }
        self.push(paint(fill, stroke.is_some()));
    }

    /// Draws a horizontal rule of `thickness`, with `y` its middle.
    pub fn rule(&mut self, x: Mils, y: Mils, width: Mils, thickness: Mils, color: Color) {
        let half = thickness.wrapping_div(2);
        self.rect(x, y.saturating_sub(half), width, thickness, color);
    }

    /// Adds a clickable rectangle.
    pub fn link(&mut self, link: Link) {
        self.links.push(link);
    }

    /// Emits the stroke colour unless it is already the current one.
    fn set_stroke(&mut self, color: Color) {
        if self.stroke == Some(color) {
            return;
        }
        if color.is_gray() {
            self.push(&format!("{} G\n", color.level()));
        } else {
            let [red, green, blue] = color.components();
            self.push(&format!("{red} {green} {blue} RG\n"));
        }
        self.stroke = Some(color);
    }

    /// Emits the fill colour unless it is already the current one.
    fn set_fill(&mut self, color: Color) {
        if self.fill == Some(color) {
            return;
        }
        if color.is_gray() {
            self.push(&format!("{} g\n", color.level()));
        } else {
            let [red, green, blue] = color.components();
            self.push(&format!("{red} {green} {blue} rg\n"));
        }
        self.fill = Some(color);
    }

    /// Appends raw operators.
    fn push(&mut self, text: &str) {
        self.content.extend_from_slice(text.as_bytes());
    }
}

/// The operator that paints a path: filled, stroked, or both, and by the
/// winding rule the fill asked for.
const fn paint(fill: Option<Fill>, stroked: bool) -> &'static str {
    match (fill, stroked) {
        (Some(Fill { even_odd: true, .. }), true) => "B*\n",
        (
            Some(Fill {
                even_odd: false, ..
            }),
            true,
        ) => "B\n",
        (Some(Fill { even_odd: true, .. }), false) => "f*\n",
        (
            Some(Fill {
                even_odd: false, ..
            }),
            false,
        ) => "f\n",
        (None, _) => "S\n",
    }
}

/// Writes one byte of a literal string, escaping what would end it.
///
/// A byte above the printable range is written as an octal escape rather
/// than raw: the content stream stays plain ASCII, which is what makes a
/// written document readable in a text editor when something has gone
/// wrong with it.
pub(crate) fn escape(byte: u8, out: &mut Vec<u8>) {
    match byte {
        b'(' | b')' | b'\\' => {
            out.push(b'\\');
            out.push(byte);
        }
        0x20..=0x7E => out.push(byte),
        _ => {
            out.extend_from_slice(format!("\\{byte:03o}").as_bytes());
        }
    }
}
