// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Text in, tags and text out.
//!
//! The tokenizer is the whole of the syntax this crate knows: a tag with
//! its attributes, an end tag, and the text between them. Comments, the
//! doctype, and processing instructions are read and dropped, because
//! nothing downstream has a use for them. The two elements whose content
//! is not markup — `script` and `style` — are read to their end tag and
//! come out as one piece of text, which is what a `<` inside them is: the
//! blocks decide what to do with it, and for a page of prose that is to
//! drop it. A stylesheet is not always waste, though: an SVG carries its
//! own inside a `style` element, and something has to be able to read
//! it.
//!
//! A `<` that begins nothing is text. Browsers do the same, and the
//! documents here contain enough prose about generics and comparisons for
//! it to matter.

use crate::entity;

/// One piece of the input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    /// An opening tag.
    Start {
        /// The element name, in lower case.
        name: String,
        /// The attributes, in the order they were written, with their
        /// names in lower case and their values decoded.
        attributes: Vec<(String, String)>,
        /// Whether the tag closed itself, as `<br/>` does.
        closed: bool,
    },
    /// A closing tag.
    End {
        /// The element name, in lower case.
        name: String,
    },
    /// Text, with its character references decoded.
    Text {
        /// What it says.
        text: String,
    },
}

/// The elements whose content is read as text and thrown away with them.
const RAW: [&str; 2] = ["script", "style"];

/// Cuts `html` into tokens.
#[must_use]
pub fn tokens(html: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(rest) = html.get(at..).filter(|rest| !rest.is_empty()) {
        let Some(offset) = rest.find('<') else {
            push_text(&mut out, rest);
            break;
        };
        if offset > 0 {
            push_text(&mut out, rest.get(..offset).unwrap_or_default());
            at = at.saturating_add(offset);
            continue;
        }
        if let Some(next) = skipped(rest) {
            at = at.saturating_add(next);
            continue;
        }
        let Some((token, length)) = tag(rest) else {
            push_text(&mut out, "<");
            at = at.saturating_add(1);
            continue;
        };
        at = at.saturating_add(length);
        let raw = match &token {
            Token::Start { name, closed, .. } if !closed && RAW.contains(&name.as_str()) => {
                Some(name.clone())
            }
            _ => None,
        };
        out.push(token);
        if let Some(name) = raw {
            let rest = html.get(at..).unwrap_or_default();
            let (text, length) = raw_content(rest, &name);
            push_raw(&mut out, text);
            out.push(Token::End { name });
            at = at.saturating_add(length);
        }
    }
    out
}

/// Adds text, unless it is empty.
fn push_text(out: &mut Vec<Token>, text: &str) {
    if text.is_empty() {
        return;
    }
    out.push(Token::Text {
        text: entity::decode(text),
    });
}

/// Adds the raw content of a `script` or a `style`, which is text and not
/// markup: nothing in it is a character reference, so nothing is decoded.
fn push_raw(out: &mut Vec<Token>, text: &str) {
    if text.is_empty() {
        return;
    }
    out.push(Token::Text {
        text: text.to_owned(),
    });
}

/// The length of a comment, a doctype, or a processing instruction at the
/// front of `rest`, all of which are read and dropped.
fn skipped(rest: &str) -> Option<usize> {
    if let Some(body) = rest.strip_prefix("<!--") {
        let end = body.find("-->").map_or(rest.len(), |offset| {
            offset.saturating_add(7) // `<!--` and `-->`
        });
        return Some(end);
    }
    if rest.starts_with("<!") || rest.starts_with("<?") {
        let end = rest
            .find('>')
            .map_or(rest.len(), |offset| offset.saturating_add(1));
        return Some(end);
    }
    None
}

/// One tag at the front of `rest`, and how many bytes it took.
fn tag(rest: &str) -> Option<(Token, usize)> {
    if let Some(body) = rest.strip_prefix("</") {
        let (name, length) = name(body)?;
        let tail = body.get(length..).unwrap_or_default();
        let end = tail
            .find('>')
            .map_or(tail.len(), |offset| offset.saturating_add(1));
        let taken = 2usize.saturating_add(length).saturating_add(end);
        return Some((Token::End { name }, taken));
    }
    let body = rest.strip_prefix('<')?;
    let (name, length) = name(body)?;
    let (attributes, closed, taken) = attributes(body.get(length..).unwrap_or_default());
    Some((
        Token::Start {
            name,
            attributes,
            closed,
        },
        1usize.saturating_add(length).saturating_add(taken),
    ))
}

/// An element or attribute name at the front of `text`, lower-cased, and
/// how many bytes it took.
fn name(text: &str) -> Option<(String, usize)> {
    let first = text.chars().next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    let end = text
        .find(|character: char| !is_name(character))
        .unwrap_or(text.len());
    let found = text.get(..end)?;
    Some((found.to_ascii_lowercase(), end))
}

/// Whether a character may stand in a name.
const fn is_name(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':' | '.')
}

/// The attributes of a tag, whether it closed itself, and how many bytes
/// were read, counting the `>`.
fn attributes(text: &str) -> (Vec<(String, String)>, bool, usize) {
    let mut out = Vec::new();
    let mut closed = false;
    let mut at = 0usize;
    loop {
        at = at.saturating_add(space(text.get(at..).unwrap_or_default()));
        let rest = text.get(at..).unwrap_or_default();
        if rest.is_empty() {
            return (out, closed, at);
        }
        if rest.starts_with("/>") {
            return (out, true, at.saturating_add(2));
        }
        if rest.starts_with('>') {
            return (out, closed, at.saturating_add(1));
        }
        if rest.starts_with('/') {
            closed = true;
            at = at.saturating_add(1);
            continue;
        }
        let Some((key, length)) = name(rest) else {
            // Something that is not a name and not an end: step over it
            // rather than stand still.
            at = at.saturating_add(step(rest));
            continue;
        };
        at = at.saturating_add(length);
        let after = space(text.get(at..).unwrap_or_default());
        let rest = text.get(at.saturating_add(after)..).unwrap_or_default();
        if let Some(assigned) = rest.strip_prefix('=') {
            let lead = space(assigned);
            let (value, taken) = value(assigned.get(lead..).unwrap_or_default());
            out.push((key, value));
            at = at
                .saturating_add(after)
                .saturating_add(1)
                .saturating_add(lead)
                .saturating_add(taken);
        } else {
            out.push((key, String::new()));
        }
    }
}

/// One attribute value, quoted or bare, and how many bytes it took.
fn value(text: &str) -> (String, usize) {
    for quote in ['"', '\''] {
        if let Some(body) = text.strip_prefix(quote) {
            let end = body.find(quote).unwrap_or(body.len());
            let found = body.get(..end).unwrap_or_default();
            let taken = end.saturating_add(2).min(text.len());
            return (entity::decode(found), taken);
        }
    }
    let end = text
        .find(|character: char| character.is_whitespace() || character == '>')
        .unwrap_or(text.len());
    let found = text.get(..end).unwrap_or_default();
    (entity::decode(found), end)
}

/// How much whitespace is at the front of `text`.
fn space(text: &str) -> usize {
    text.find(|character: char| !character.is_whitespace())
        .unwrap_or(text.len())
}

/// One character, in bytes, so that a loop over something unexpected still
/// gets somewhere.
fn step(text: &str) -> usize {
    text.chars().next().map_or(1, char::len_utf8)
}

/// The content of a raw element, and how much of `rest` it and its end
/// tag took.
fn raw_content<'a>(rest: &'a str, name: &str) -> (&'a str, usize) {
    let closing = format!("</{name}");
    let Some(offset) = rest.to_ascii_lowercase().find(&closing) else {
        return (rest, rest.len());
    };
    let text = rest.get(..offset).unwrap_or_default();
    let tail = rest.get(offset..).unwrap_or_default();
    let end = tail
        .find('>')
        .map_or(tail.len(), |found| found.saturating_add(1));
    (text, offset.saturating_add(end))
}
