// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

mod build;
mod element;
mod entity;
mod runs;
pub mod token;
pub mod tree;

#[cfg(test)]
mod tests;

use doc_markdown::Block;

/// Parses an HTML document into the blocks a document is made of.
#[must_use]
pub fn parse(html: &str) -> Vec<Block> {
    build::document(&tree::parse(html))
}

/// The title of the document, from its `title` element, with its
/// whitespace collapsed. A document without one has none: what a page is
/// called is not something to guess at from its first line of text.
#[must_use]
pub fn title(html: &str) -> Option<String> {
    let start = find_tag(html, "<title")?;
    let after = html.get(start..)?;
    let open = after.find('>')?;
    let body = after.get(open.saturating_add(1)..)?;
    let end = find_tag(body, "</title").unwrap_or(body.len());
    let text = entity::decode(body.get(..end)?);
    let collapsed = text.split_whitespace().collect::<Vec<&str>>().join(" ");
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

/// Where a tag begins, whatever case it was written in.
fn find_tag(html: &str, tag: &str) -> Option<usize> {
    html.match_indices('<')
        .map(|(at, _)| at)
        .find(|at| starts_with_tag(html.get(*at..).unwrap_or_default(), tag))
}

/// Whether `text` begins with `tag`, ignoring case, and the tag ends
/// there rather than being the front of a longer name.
fn starts_with_tag(text: &str, tag: &str) -> bool {
    let Some(rest) = text.get(..tag.len()) else {
        return false;
    };
    if !rest.eq_ignore_ascii_case(tag) {
        return false;
    }
    text.get(tag.len()..)
        .and_then(|tail| tail.chars().next())
        .is_none_or(|character| !character.is_ascii_alphanumeric())
}
