// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A tree in, the blocks of a document out.
//!
//! The builder holds one line under construction and a list of finished
//! blocks. Text and inline elements go into the line; a block element ends
//! it, whatever it says, and adds a block of its own. That is the whole of
//! it, and it is why an element nobody wrote a case for costs nothing: it
//! is neither of those things, so its children are read where it stood.
//!
//! Everything with content of its own — a list item, a cell, a section, a
//! quotation — is built by a builder of its own over its children, which
//! is how an item comes to hold a paragraph, a table, and another list
//! without a single case for any of that here.
//!
//! Headings count from where they stand. A page written as one section
//! after another gives every one of them an `h1`, and the number in the
//! tag says nothing about the depth; the number of sections around it
//! does. So the level of a heading is its own number plus the sections it
//! sits in, which is what makes an outline out of a document that has
//! only one kind of heading in it.

use doc_markdown::{Align, Block, Inline};

use crate::element;
use crate::runs::{self, Runs};
use crate::tree::{Element, Node};

/// The blocks of a document.
pub(crate) fn document(nodes: &[Node]) -> Vec<Block> {
    let mut builder = Builder::new(0);
    builder.nodes(nodes);
    builder.finish()
}

/// A document being built.
struct Builder {
    /// The blocks finished so far.
    out: Vec<Block>,
    /// The line being built.
    runs: Runs,
    /// How many sections this builder is inside.
    depth: usize,
}

impl Builder {
    /// A builder at the given section depth.
    const fn new(depth: usize) -> Self {
        Self {
            out: Vec::new(),
            runs: Runs::new(),
            depth,
        }
    }

    /// Walks a sequence of nodes.
    fn nodes(&mut self, nodes: &[Node]) {
        for node in nodes {
            match node {
                Node::Text(text) => self.runs.text(text),
                Node::Element(element) => self.element(element),
            }
        }
    }

    /// The finished blocks.
    fn finish(mut self) -> Vec<Block> {
        self.flush();
        self.out
    }

    /// Ends the line, if anything is on it.
    fn flush(&mut self) {
        let content = self.runs.take();
        if !content.is_empty() {
            self.out.push(Block::Paragraph(content));
        }
    }

    /// One element.
    fn element(&mut self, element: &Element) {
        let name = element.name.as_str();
        if element::is_dropped(name) {
            return;
        }
        if let Some(level) = element::heading(name) {
            self.heading(level, element);
            return;
        }
        // An element that says of itself that it belongs in a line is
        // read in one, whatever the tables make of its name. An equation
        // written between two halves of a sentence is the case: set as a
        // block it would cut the sentence in three.
        if element::says_inline(element.attribute("class")) {
            self.runs.marks(runs::of_element(element));
            return;
        }
        if element::is_preformatted(name) {
            self.code(element);
            return;
        }
        if element::is_quoted(name) {
            self.quote(element);
            return;
        }
        match name {
            "br" => self.runs.mark(Inline::Break),
            "hr" => {
                self.flush();
                self.out.push(Block::Rule);
            }
            "ol" | "ul" => self.list(element),
            "table" => self.table(element),
            "dt" => self.term(element),
            "dd" => self.quote(element),
            "figcaption" => self.caption(element),
            _ if element::is_block(name) => self.container(element),
            // An element nobody named is read as though it were not
            // there. Whether that leaves a line or a set of blocks is
            // decided by what is inside it, because that is the only
            // evidence there is once the stylesheet is gone.
            _ if element::face(name).is_none() && holds_block(element) => {
                self.nodes(&element.children);
            }
            _ => self.runs.marks(runs::of_element(element)),
        }
    }

    /// A heading, at the level its own number and the sections around it
    /// put it at.
    fn heading(&mut self, level: usize, element: &Element) {
        self.flush();
        let content = runs::inlines(&element.children);
        if content.is_empty() {
            return;
        }
        let level = if self.depth == 0 {
            level
        } else {
            level.saturating_add(self.depth).saturating_sub(1)
        };
        self.out.push(Block::Heading {
            level: level.clamp(1, 6),
            content,
        });
    }

    /// An element that only holds other things.
    fn container(&mut self, element: &Element) {
        self.flush();
        let blocks = self.inner(element);
        self.out.extend(blocks);
    }

    /// A quotation.
    fn quote(&mut self, element: &Element) {
        self.flush();
        let blocks = self.inner(element);
        if !blocks.is_empty() {
            self.out.push(Block::Quote(blocks));
        }
    }

    /// The term of a description list, which is set in bold above what it
    /// describes.
    fn term(&mut self, element: &Element) {
        self.flush();
        let content = runs::inlines(&element.children);
        if !content.is_empty() {
            self.out
                .push(Block::Paragraph(vec![Inline::Strong(content)]));
        }
    }

    /// The caption of a figure or a table.
    fn caption(&mut self, element: &Element) {
        self.flush();
        let content = runs::inlines(&element.children);
        if !content.is_empty() {
            self.out
                .push(Block::Paragraph(vec![Inline::Emphasis(content)]));
        }
    }

    /// A list, with one builder per item.
    fn list(&mut self, element: &Element) {
        self.flush();
        let mut items = Vec::new();
        let mut tight = true;
        for item in element.children_named("li") {
            if item.children_named("p").next().is_some() {
                tight = false;
            }
            items.push(self.inner(item));
        }
        if items.is_empty() {
            return;
        }
        self.out.push(Block::List {
            ordered: element.name == "ol",
            start: element
                .attribute("start")
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(1),
            tight,
            items,
        });
    }

    /// A table. A row of `th`, or any row inside a `thead`, is the header;
    /// a table that names no header row gives up its first row for one,
    /// because a table without a header is not a table this format has.
    fn table(&mut self, element: &Element) {
        self.flush();
        let mut found = Vec::new();
        collect_rows(element, false, &mut found);
        let mut header: Vec<&Element> = Vec::new();
        let mut rows: Vec<Vec<Vec<Inline>>> = Vec::new();
        for (is_header, cells) in found {
            if is_header && header.is_empty() && rows.is_empty() {
                header = cells;
            } else {
                rows.push(cells.iter().map(|cell| cell_content(cell)).collect());
            }
        }
        let mut head: Vec<Vec<Inline>> = header.iter().map(|cell| cell_content(cell)).collect();
        if head.is_empty() {
            if rows.is_empty() {
                return;
            }
            head = rows.remove(0);
        }
        let alignments = (0..head.len())
            .map(|column| {
                header
                    .get(column)
                    .and_then(|cell| alignment(cell))
                    .unwrap_or(Align::Left)
            })
            .collect();
        self.out.push(Block::Table {
            alignments,
            header: head,
            rows,
        });
    }

    /// A block of text kept line for line.
    fn code(&mut self, element: &Element) {
        self.flush();
        let mut text = String::new();
        write_code(element, element.name == "pre", &mut text);
        let lines = trim_lines(&text);
        if lines.is_empty() {
            return;
        }
        self.out.push(Block::Code {
            language: language(element),
            lines,
        });
    }

    /// The blocks inside one element, one section deeper if the element is
    /// one.
    fn inner(&self, element: &Element) -> Vec<Block> {
        let depth = if element::is_sectioning(&element.name) {
            self.depth.saturating_add(1)
        } else {
            self.depth
        };
        let mut builder = Builder::new(depth);
        builder.nodes(&element.children);
        builder.finish()
    }
}

/// Whether anything inside the element stands on its own line.
fn holds_block(element: &Element) -> bool {
    element.children.iter().any(|node| {
        let Node::Element(child) = node else {
            return false;
        };
        if element::is_dropped(&child.name) {
            return false;
        }
        element::is_block(&child.name) || holds_block(child)
    })
}

/// The rows of a table, and whether each is a header row.
fn collect_rows<'a>(element: &'a Element, head: bool, out: &mut Vec<(bool, Vec<&'a Element>)>) {
    for node in &element.children {
        let Node::Element(child) = node else { continue };
        match child.name.as_str() {
            "tr" => {
                let cells: Vec<&Element> = child
                    .children
                    .iter()
                    .filter_map(|node| match node {
                        Node::Element(cell) if cell.name == "td" || cell.name == "th" => Some(cell),
                        _ => None,
                    })
                    .collect();
                let header =
                    head || (!cells.is_empty() && cells.iter().all(|cell| cell.name == "th"));
                out.push((header, cells));
            }
            "thead" => collect_rows(child, true, out),
            "tbody" | "tfoot" => collect_rows(child, false, out),
            _ => {}
        }
    }
}

/// What one cell says. A cell holds runs and not blocks, so a paragraph
/// inside one is read for its text.
fn cell_content(cell: &Element) -> Vec<Inline> {
    runs::inlines(&cell.children)
}

/// The alignment a cell asks for, in its attribute or in its style.
fn alignment(cell: &Element) -> Option<Align> {
    if let Some(value) = cell.attribute("align") {
        match value.trim().to_ascii_lowercase().as_str() {
            "center" | "centre" => return Some(Align::Center),
            "right" => return Some(Align::Right),
            "left" => return Some(Align::Left),
            _ => {}
        }
    }
    let style = cell.attribute("style")?.to_ascii_lowercase();
    let (_, rest) = style.split_once("text-align")?;
    let value = rest.trim_start_matches([':', ' ']);
    if value.starts_with("center") {
        return Some(Align::Center);
    }
    if value.starts_with("right") {
        return Some(Align::Right);
    }
    Some(Align::Left)
}

/// The language of a code block, from the class of the element or of the
/// `code` inside it.
fn language(element: &Element) -> Option<String> {
    let own = element.attribute("class").map(str::to_owned);
    let inner = element
        .children_named("code")
        .next()
        .and_then(|code| code.attribute("class"))
        .map(str::to_owned);
    for classes in [own, inner].into_iter().flatten() {
        for class in classes.split_whitespace() {
            for prefix in ["language-", "lang-"] {
                if let Some(name) = class.strip_prefix(prefix)
                    && !name.is_empty()
                {
                    return Some(name.to_owned());
                }
            }
        }
    }
    None
}

/// Writes the text of a preformatted element. Inside a `pre` the text is
/// what it was; everywhere else the whitespace of the markup means
/// nothing, and the lines are the ones the structure asks for — one per
/// production, one per right-hand side, indented under it.
fn write_code(element: &Element, raw: bool, out: &mut String) {
    for node in &element.children {
        match node {
            Node::Text(text) => {
                if raw {
                    out.push_str(text);
                } else {
                    push_collapsed(out, text);
                }
            }
            Node::Element(child) => {
                if element::is_dropped(&child.name) {
                    continue;
                }
                match child.name.as_str() {
                    "br" | "emu-production" => newline(out),
                    "emu-rhs" => {
                        newline(out);
                        out.push_str("  ");
                    }
                    _ => {}
                }
                write_code(child, raw, out);
            }
        }
    }
}

/// Adds text with its whitespace collapsed, and no space where one is
/// already standing.
fn push_collapsed(out: &mut String, text: &str) {
    for (index, word) in text.split_whitespace().enumerate() {
        let spaced = index > 0 || text.starts_with(char::is_whitespace);
        if spaced && !out.is_empty() && !out.ends_with([' ', '\n']) {
            out.push(' ');
        }
        out.push_str(word);
    }
    if text.ends_with(char::is_whitespace) && !out.is_empty() && !out.ends_with([' ', '\n']) {
        out.push(' ');
    }
}

/// Begins a line, unless one has just begun.
fn newline(out: &mut String) {
    while out.ends_with(' ') {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

/// The lines of a block of text, without the empty ones at either end.
fn trim_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| line.trim_end().to_owned())
        .collect();
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines
}
