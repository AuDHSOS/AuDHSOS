// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! One page and the marks on it.
//!
//! A page holds its content stream as it is built, operator by operator.
//! The coordinate system is the one PDF itself uses: the origin sits at the
//! bottom left corner and `y` grows upwards. Whoever lays out a document
//! usually counts downwards from the top, and converts once, at the edge,
//! rather than everywhere.

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
    pub fn text(&mut self, font: Font, size: Mils, x: Mils, y: Mils, color: Color, text: &str) {
        self.text_encoded(font, size, x, y, color, &font::encode(text));
    }

    /// Sets already encoded text at `x`, with `y` the baseline.
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
        self.push(&format!(
            "BT /{} {} Tf 1 0 0 1 {} {} Tm ",
            font.resource(),
            number(size),
            number(x),
            number(y)
        ));
        self.content.push(b'(');
        for byte in encoded {
            escape(*byte, &mut self.content);
        }
        self.push(") Tj ET\n");
    }

    /// Fills a rectangle.
    pub fn rect(&mut self, x: Mils, y: Mils, width: Mils, height: Mils, color: Color) {
        if width <= 0 || height <= 0 {
            return;
        }
        self.set_fill(color);
        self.push(&format!(
            "{} {} {} {} re f\n",
            number(x),
            number(y),
            number(width),
            number(height)
        ));
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

    /// Emits the fill colour unless it is already the current one.
    fn set_fill(&mut self, color: Color) {
        if self.fill == Some(color) {
            return;
        }
        let [red, green, blue] = color.components();
        self.push(&format!("{red} {green} {blue} rg\n"));
        self.fill = Some(color);
    }

    /// Appends raw operators.
    fn push(&mut self, text: &str) {
        self.content.extend_from_slice(text.as_bytes());
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
