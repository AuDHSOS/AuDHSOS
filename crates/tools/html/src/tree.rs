// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tokens in, a tree out.
//!
//! This module and [`crate::token`] are public because the markup they
//! read is not only HTML. An SVG is XML, and tags, attributes, text, and
//! a tree are all an SVG reader needs of a parser; `doc-svg` uses these
//! rather than carrying a second copy of them. What is not public is
//! everything that knows what an element *means*, which is HTML's alone.
//!
//! The tree is built with one stack and two rules for the tags nobody
//! bothers to close. An end tag closes the nearest open element of its
//! name, and everything still open inside it, which is what a browser
//! does; an end tag that names nothing open is dropped, because the
//! alternative is to close something the author did not mean. And a start
//! tag can close what is open before it: a paragraph is ended by the next
//! block, an item by the next item, a cell by the next cell. Those are the
//! only implied ends there are here, and they are the ones documents
//! actually rely on.

use crate::element;
use crate::token::{Token, tokens};

/// One node of the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// Text, with its character references already decoded.
    Text(String),
    /// An element and what is inside it.
    Element(Element),
}

/// An element, its attributes, and its children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    /// The name, in lower case.
    pub name: String,
    /// The attributes, in the order they were written.
    pub attributes: Vec<(String, String)>,
    /// What is inside it.
    pub children: Vec<Node>,
}

impl Element {
    /// The value of one attribute.
    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The child elements of the given name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Self> {
        self.children.iter().filter_map(move |node| match node {
            Node::Element(child) if child.name == name => Some(child),
            _ => None,
        })
    }
}

/// Parses `html` into a tree.
#[must_use]
pub fn parse(html: &str) -> Vec<Node> {
    let mut roots: Vec<Node> = Vec::new();
    let mut stack: Vec<Element> = Vec::new();
    for token in tokens(html) {
        match token {
            Token::Text { text } => push(&mut stack, &mut roots, Node::Text(text)),
            Token::Start {
                name,
                attributes,
                closed,
            } => {
                let inline = element::says_inline(
                    attributes
                        .iter()
                        .find(|(key, _)| key == "class")
                        .map(|(_, value)| value.as_str()),
                );
                while stack
                    .last()
                    .is_some_and(|open| implied_end(&open.name, &name, inline))
                {
                    close(&mut stack, &mut roots);
                }
                let element = Element {
                    name,
                    attributes,
                    children: Vec::new(),
                };
                if closed || element::is_void(&element.name) {
                    push(&mut stack, &mut roots, Node::Element(element));
                } else {
                    stack.push(element);
                }
            }
            Token::End { name } => {
                if stack.iter().any(|open| open.name == name) {
                    while stack.last().is_some_and(|open| open.name != name) {
                        close(&mut stack, &mut roots);
                    }
                    close(&mut stack, &mut roots);
                }
            }
        }
    }
    while !stack.is_empty() {
        close(&mut stack, &mut roots);
    }
    roots
}

/// Whether a start tag of `starting` ends an open element of `open`.
/// A tag that says it belongs in a line ends nothing: it is not the block
/// its name would otherwise make it.
fn implied_end(open: &str, starting: &str, inline: bool) -> bool {
    match open {
        "p" => element::is_block(starting) && !inline,
        "li" => starting == "li",
        "dt" | "dd" => matches!(starting, "dt" | "dd"),
        "td" | "th" => matches!(starting, "td" | "th" | "tr" | "thead" | "tbody" | "tfoot"),
        "tr" => matches!(starting, "tr" | "thead" | "tbody" | "tfoot"),
        _ => false,
    }
}

/// Adds a node to whatever is open, or to the document.
fn push(stack: &mut [Element], roots: &mut Vec<Node>, node: Node) {
    match stack.last_mut() {
        Some(open) => open.children.push(node),
        None => roots.push(node),
    }
}

/// Closes the innermost open element.
fn close(stack: &mut Vec<Element>, roots: &mut Vec<Node>) {
    if let Some(element) = stack.pop() {
        push(stack, roots, Node::Element(element));
    }
}
