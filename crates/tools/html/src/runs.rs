// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What runs inside a line: text, the faces it is set in, and links.
//!
//! Whitespace in HTML is not what it says. A newline and eight spaces of
//! indentation between two tags are one space, and the same space can be
//! written on either side of the tag that separates two words. So text is
//! collected word by word, with a space owed rather than written, and the
//! debt is paid when the next word or the next mark arrives. Nothing owes
//! a space at the beginning of a line or at its end.
//!
//! Inside an element that is set in a face of its own, everything is a
//! run: an element that would be a block elsewhere is read for its text
//! and nothing more. A block inside a line is markup nobody meant, and
//! guessing at it would cost more than it is worth.

use doc_markdown::{Inline, plain};

use crate::element::{self, Face};
use crate::tree::{Element, Node};

/// The runs of a sequence of nodes.
pub(crate) fn inlines(nodes: &[Node]) -> Vec<Inline> {
    let mut runs = Runs::new();
    runs.nodes(nodes);
    runs.take()
}

/// A line under construction.
pub(crate) struct Runs {
    /// The runs finished so far.
    out: Vec<Inline>,
    /// The text of the run being built.
    buffer: String,
    /// Whether a space is owed before whatever comes next.
    space: bool,
}

impl Runs {
    /// An empty line.
    pub(crate) const fn new() -> Self {
        Self {
            out: Vec::new(),
            buffer: String::new(),
            space: false,
        }
    }

    /// Whether nothing has been written yet.
    pub(crate) const fn is_empty(&self) -> bool {
        self.out.is_empty() && self.buffer.is_empty()
    }

    /// Adds text, collapsing its whitespace.
    pub(crate) fn text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if text.starts_with(char::is_whitespace) {
            self.space = true;
        }
        for (index, word) in text.split_whitespace().enumerate() {
            if index > 0 {
                self.space = true;
            }
            self.word(word);
        }
        if text.ends_with(char::is_whitespace) {
            self.space = true;
        }
    }

    /// Adds a finished run. Text joins the text before it rather than
    /// standing beside it: two runs of the same face are one run.
    pub(crate) fn mark(&mut self, inline: Inline) {
        if let Inline::Text(value) = inline {
            self.word(&value);
            return;
        }
        if self.space && !self.is_empty() {
            self.buffer.push(' ');
        }
        self.space = false;
        self.flush();
        self.out.push(inline);
    }

    /// Adds text that needs no collapsing, paying the space that is owed.
    fn word(&mut self, word: &str) {
        if word.is_empty() {
            return;
        }
        if self.space && !self.is_empty() {
            self.buffer.push(' ');
        }
        self.space = false;
        self.buffer.push_str(word);
    }

    /// Adds several finished runs.
    pub(crate) fn marks(&mut self, inlines: Vec<Inline>) {
        for inline in inlines {
            self.mark(inline);
        }
    }

    /// Walks nodes, adding what they say. An element that would be a
    /// block elsewhere is read for its text here, and stands apart from
    /// what surrounds it: the markup gave it a line of its own, and a
    /// line is at least a space.
    pub(crate) fn nodes(&mut self, nodes: &[Node]) {
        for node in nodes {
            match node {
                Node::Text(text) => self.text(text),
                Node::Element(element) => {
                    let apart = element::is_block(&element.name);
                    if apart {
                        self.space = true;
                    }
                    self.marks(of_element(element));
                    if apart {
                        self.space = true;
                    }
                }
            }
        }
    }

    /// The finished line, which leaves the builder empty.
    pub(crate) fn take(&mut self) -> Vec<Inline> {
        self.flush();
        self.space = false;
        std::mem::take(&mut self.out)
    }

    /// Moves the text built so far into a run of its own.
    fn flush(&mut self) {
        if !self.buffer.is_empty() {
            self.out
                .push(Inline::Text(std::mem::take(&mut self.buffer)));
        }
    }
}

/// The runs one element stands for.
pub(crate) fn of_element(element: &Element) -> Vec<Inline> {
    let name = element.name.as_str();
    if element::is_dropped(name) {
        return Vec::new();
    }
    match name {
        "br" => return vec![Inline::Break],
        "img" => return vec![image(element)],
        "a" => return anchor(element),
        _ => {}
    }
    let inner = inlines(&element.children);
    if inner.is_empty() {
        return Vec::new();
    }
    match element::face(name) {
        Some(Face::Mono) => vec![Inline::Code(plain(&inner))],
        Some(Face::Emphasis) => vec![Inline::Emphasis(inner)],
        Some(Face::Strong) => vec![Inline::Strong(inner)],
        None => inner,
    }
}

/// A link. One that points into the document it is written in keeps its
/// text and loses its link: a PDF page has no name for the place the
/// fragment refers to, and a link that goes nowhere is worse than none.
fn anchor(element: &Element) -> Vec<Inline> {
    let inner = inlines(&element.children);
    let Some(href) = element.attribute("href") else {
        return inner;
    };
    if href.is_empty() || href.starts_with('#') || inner.is_empty() {
        return inner;
    }
    vec![Inline::Link {
        content: inner,
        href: href.to_owned(),
    }]
}

/// An image: the file it is in and what it says it shows. What becomes of
/// it is not decided here — a picture on a line of its own can be drawn,
/// and one in the middle of a sentence has to be said instead.
fn image(element: &Element) -> Inline {
    Inline::Image {
        source: element.attribute("src").unwrap_or_default().to_owned(),
        alt: element.attribute("alt").unwrap_or_default().to_owned(),
    }
}
