// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Turning inline runs into lines that fit.
//!
//! Inline content becomes a flat list of words and spaces, each carrying
//! the face, size, colour, and link it inherited from the marks around it.
//! Breaking is greedy: a word goes on the current line if it fits, and
//! starts a new one if it does not. There is no justification — a ragged
//! right edge is easier to read than the rivers that justified prose grows
//! at this measure, and it needs no second pass.
//!
//! A word wider than the whole measure is cut at the last character that
//! fits. That is not hyphenation, and it is not meant to be: the words this
//! happens to are file paths and identifiers, where a break anywhere is
//! better than a line running off the page.

use doc_markdown::Inline;
use doc_pdf::font::Font;
use doc_pdf::units::{Color, Mils};

use crate::theme;

/// How a run of text is set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Style {
    /// The face.
    pub(crate) font: Font,
    /// The point size.
    pub(crate) size: Mils,
    /// The colour.
    pub(crate) color: Color,
    /// Where the run links to, if it does.
    pub(crate) href: Option<String>,
}

impl Style {
    /// A style with no link.
    pub(crate) const fn new(font: Font, size: Mils, color: Color) -> Self {
        Self {
            font,
            size,
            color,
            href: None,
        }
    }

    /// The width of `text` in this style.
    pub(crate) fn width(&self, text: &str) -> Mils {
        self.font.width_of_str(text, self.size)
    }
}

/// A word, a space, or a break the author asked for.
#[derive(Clone, Debug)]
enum Token {
    /// A run with no space in it.
    Word {
        /// The text.
        text: String,
        /// How it is set.
        style: Style,
    },
    /// The space between two words.
    Space {
        /// How it is set, which is what gives the space its width.
        style: Style,
    },
    /// A line break.
    Break,
}

/// One run of one style, placed on a line.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    /// The distance from the start of the line.
    pub(crate) offset: Mils,
    /// How wide the run is.
    pub(crate) width: Mils,
    /// The text.
    pub(crate) text: String,
    /// How it is set.
    pub(crate) style: Style,
}

/// One line of set text.
#[derive(Clone, Debug, Default)]
pub(crate) struct Line {
    /// The runs on the line, in order.
    pub(crate) pieces: Vec<Piece>,
    /// How wide the line is, without the space it ends on.
    pub(crate) width: Mils,
}

/// Breaks inline content into lines no wider than `measure`.
pub(crate) fn lines(content: &[Inline], base: &Style, measure: Mils) -> Vec<Line> {
    wrap(&tokens(content, base), measure)
}

/// Flattens inline content into words and spaces.
fn tokens(content: &[Inline], base: &Style) -> Vec<Token> {
    let mut out = Vec::new();
    push_inlines(content, base, &mut out);
    out
}

/// Walks the inline tree, carrying the style down.
fn push_inlines(content: &[Inline], style: &Style, out: &mut Vec<Token>) {
    for item in content {
        match item {
            Inline::Text(text) => push_words(text, style, out),
            Inline::Code(text) => {
                let mut code = style.clone();
                code.font = if style.font == Font::Bold {
                    Font::MonoBold
                } else {
                    Font::Mono
                };
                // A fixed-pitch face at the size of the prose around it
                // looks a size too large, because its letters are wider.
                code.size = style.size.saturating_mul(92).wrapping_div(100);
                if style.href.is_none() {
                    code.color = theme::CODE_INK;
                }
                push_words(text, &code, out);
            }
            Inline::Emphasis(inner) => {
                let mut italic = style.clone();
                italic.font = match style.font {
                    Font::Bold | Font::MonoBold => style.font,
                    Font::Mono => Font::Mono,
                    Font::Regular | Font::Italic => Font::Italic,
                };
                push_inlines(inner, &italic, out);
            }
            Inline::Strong(inner) => {
                let mut bold = style.clone();
                bold.font = style.font.bold();
                push_inlines(inner, &bold, out);
            }
            Inline::Link { content, href } => {
                let mut link = style.clone();
                link.color = theme::LINK;
                link.href = Some(href.clone());
                push_inlines(content, &link, out);
            }
            Inline::Break => out.push(Token::Break),
        }
    }
}

/// Splits text at its spaces.
fn push_words(text: &str, style: &Style, out: &mut Vec<Token>) {
    let mut word = String::new();
    for character in text.chars() {
        if character.is_whitespace() {
            if !word.is_empty() {
                out.push(Token::Word {
                    text: std::mem::take(&mut word),
                    style: style.clone(),
                });
            }
            if !matches!(out.last(), Some(Token::Space { .. }) | None) {
                out.push(Token::Space {
                    style: style.clone(),
                });
            }
        } else {
            word.push(character);
        }
    }
    if !word.is_empty() {
        out.push(Token::Word {
            text: word,
            style: style.clone(),
        });
    }
}

/// Places the tokens on lines.
fn wrap(tokens: &[Token], measure: Mils) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut line = Line::default();
    let mut pending: Option<Piece> = None;
    for token in tokens {
        match token {
            Token::Break => {
                pending = None;
                lines.push(std::mem::take(&mut line));
            }
            Token::Space { style } => {
                if !line.pieces.is_empty() {
                    let width = style.width(" ");
                    pending = Some(Piece {
                        offset: line.width,
                        width,
                        text: " ".to_owned(),
                        style: style.clone(),
                    });
                }
            }
            Token::Word { text, style } => {
                for (index, part) in split(text, style, measure).into_iter().enumerate() {
                    let width = style.width(&part);
                    let gap = pending.as_ref().map_or(0, |space| space.width);
                    let taken = line.width.saturating_add(gap).saturating_add(width);
                    if index > 0 || (taken > measure && !line.pieces.is_empty()) {
                        pending = None;
                        lines.push(std::mem::take(&mut line));
                    }
                    if let Some(space) = pending.take() {
                        line.width = line.width.saturating_add(space.width);
                        line.pieces.push(space);
                    }
                    line.pieces.push(Piece {
                        offset: line.width,
                        width,
                        text: part,
                        style: style.clone(),
                    });
                    line.width = line.width.saturating_add(width);
                }
            }
        }
    }
    if !line.pieces.is_empty() {
        lines.push(line);
    }
    for line in &mut lines {
        while line.pieces.last().is_some_and(|piece| piece.text == " ") {
            if let Some(space) = line.pieces.pop() {
                line.width = line.width.saturating_sub(space.width);
            }
        }
    }
    lines
}

/// Cuts a word that is wider than the measure into parts that are not.
fn split(text: &str, style: &Style, measure: Mils) -> Vec<String> {
    if style.width(text) <= measure || measure <= 0 {
        return vec![text.to_owned()];
    }
    let mut parts = Vec::new();
    let mut part = String::new();
    for character in text.chars() {
        let mut candidate = part.clone();
        candidate.push(character);
        if !part.is_empty() && style.width(&candidate) > measure {
            parts.push(std::mem::take(&mut part));
        }
        part.push(character);
    }
    if !part.is_empty() {
        parts.push(part);
    }
    parts
}

/// How wide inline content would be set unbroken, and how wide its widest
/// single word is.
///
/// Both numbers are measured through the same tokens the breaker uses, so
/// a code span inside a table cell is measured in the fixed-pitch face it
/// will actually be set in, and not in the face of the prose around it.
pub(crate) fn extent(content: &[Inline], base: &Style) -> (Mils, Mils) {
    let mut natural: Mils = 0;
    let mut longest: Mils = 0;
    for token in tokens(content, base) {
        match token {
            Token::Word { text, style } => {
                let width = style.width(&text);
                natural = natural.saturating_add(width);
                longest = longest.max(width);
            }
            Token::Space { style } => natural = natural.saturating_add(style.width(" ")),
            Token::Break => {}
        }
    }
    (natural, longest)
}
