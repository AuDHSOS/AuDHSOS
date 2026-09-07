// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! What runs inside a block, and the parser that finds it.
//!
//! The rules are the ones this repository writes by, not the whole of
//! CommonMark. Two of them are worth naming. An underscore only opens or
//! closes emphasis at a word boundary, so `saturating_add` and
//! `check_crate_roots` stay the identifiers they are instead of turning
//! half a sentence into italics. And a code span is delimited by however
//! many backticks opened it, so a span may itself contain a backtick.

/// One run inside a block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    /// Plain text.
    Text(String),
    /// A code span, set in the fixed-pitch face and never reflowed inside.
    Code(String),
    /// Emphasis.
    Emphasis(Vec<Inline>),
    /// Strong emphasis.
    Strong(Vec<Inline>),
    /// A link, and what it shows.
    Link {
        /// The text of the link.
        content: Vec<Inline>,
        /// Where it points.
        href: String,
    },
    /// A line break the author asked for.
    Break,
    /// A picture, by the file it is in and what it says it shows. What
    /// becomes of it is the renderer's to decide: a picture on a line of
    /// its own can be drawn, and one in the middle of a sentence has to be
    /// said rather than drawn.
    Image {
        /// Where the picture is, as the document wrote it.
        source: String,
        /// What the document says it shows.
        alt: String,
    },
}

/// The text of a run of inlines, with the marks dropped.
#[must_use]
pub fn plain(content: &[Inline]) -> String {
    let mut text = String::new();
    for item in content {
        match item {
            Inline::Text(value) | Inline::Code(value) => text.push_str(value),
            Inline::Emphasis(inner) | Inline::Strong(inner) => text.push_str(&plain(inner)),
            Inline::Link { content, .. } => text.push_str(&plain(content)),
            Inline::Break => text.push(' '),
            Inline::Image { source, alt } => {
                text.push_str(&described(source, alt));
            }
        }
    }
    text
}

/// What a picture is called where it cannot be drawn: what it says it
/// shows, or the file it is in when it says nothing.
#[must_use]
pub fn described(source: &str, alt: &str) -> String {
    let shown = if alt.trim().is_empty() { source } else { alt };
    format!("[image: {shown}]")
}

/// Parses the inline content of a block.
#[must_use]
pub fn parse(text: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut buffer = String::new();
    let mut at = 0;
    while let Some(character) = char_at(text, at) {
        let next = at.saturating_add(character.len_utf8());
        match character {
            '\\' => match char_at(text, next) {
                Some(escaped) if is_punctuation(escaped) => {
                    buffer.push(escaped);
                    at = next.saturating_add(escaped.len_utf8());
                }
                _ => {
                    buffer.push('\\');
                    at = next;
                }
            },
            '\n' => {
                let hard = buffer.ends_with("  ");
                while buffer.ends_with(' ') {
                    buffer.pop();
                }
                if hard {
                    flush(&mut buffer, &mut out);
                    out.push(Inline::Break);
                } else {
                    buffer.push(' ');
                }
                at = next;
            }
            '`' => {
                if let Some((code, end)) = code_span(text, at) {
                    flush(&mut buffer, &mut out);
                    out.push(Inline::Code(code));
                    at = end;
                } else {
                    buffer.push('`');
                    at = next;
                }
            }
            '*' | '_' => {
                if let Some((inline, end)) = emphasis(text, at, character) {
                    flush(&mut buffer, &mut out);
                    out.push(inline);
                    at = end;
                } else {
                    buffer.push(character);
                    at = next;
                }
            }
            '[' => {
                if let Some((inline, end)) = link(text, at) {
                    flush(&mut buffer, &mut out);
                    out.push(inline);
                    at = end;
                } else {
                    buffer.push('[');
                    at = next;
                }
            }
            '!' if char_at(text, next) == Some('[') => {
                if let Some((inline, end)) = link(text, next) {
                    flush(&mut buffer, &mut out);
                    out.push(image(inline));
                    at = end;
                } else {
                    buffer.push('!');
                    at = next;
                }
            }
            '<' => {
                if let Some((inline, end)) = autolink(text, at) {
                    flush(&mut buffer, &mut out);
                    out.push(inline);
                    at = end;
                } else {
                    buffer.push('<');
                    at = next;
                }
            }
            other => {
                buffer.push(other);
                at = next;
            }
        }
    }
    flush(&mut buffer, &mut out);
    out
}

/// Moves what has been collected into the output.
fn flush(buffer: &mut String, out: &mut Vec<Inline>) {
    if !buffer.is_empty() {
        out.push(Inline::Text(std::mem::take(buffer)));
    }
}

/// The character at a byte offset.
fn char_at(text: &str, at: usize) -> Option<char> {
    text.get(at..).and_then(|rest| rest.chars().next())
}

/// Whether a character may be escaped by a backslash.
fn is_punctuation(character: char) -> bool {
    "\\`*_{}[]()#+-.!|<>~\"'".contains(character)
}

/// Whether a character binds a word together, and so keeps an underscore
/// beside it from opening or closing emphasis.
fn is_word(character: Option<char>) -> bool {
    character.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// A code span starting at `at`, and where it ends.
fn code_span(text: &str, at: usize) -> Option<(String, usize)> {
    let rest = text.get(at..)?;
    let ticks = rest.chars().take_while(|c| *c == '`').count();
    let fence: String = "`".repeat(ticks);
    let after = at.saturating_add(ticks);
    let body = text.get(after..)?;
    let mut search = 0;
    let close = loop {
        let found = body.get(search..)?.find(&fence)?.saturating_add(search);
        let end = found.saturating_add(ticks);
        if char_at(body, end) == Some('`') {
            search = body
                .get(end..)?
                .chars()
                .take_while(|c| *c == '`')
                .count()
                .saturating_add(end);
            continue;
        }
        break found;
    };
    let raw = body.get(..close)?.replace('\n', " ");
    let trimmed = match (raw.starts_with(' '), raw.ends_with(' ')) {
        (true, true) if raw.trim().is_empty() => raw.clone(),
        (true, true) => raw
            .get(1..raw.len().saturating_sub(1))
            .unwrap_or(&raw)
            .to_owned(),
        _ => raw.clone(),
    };
    Some((trimmed, after.saturating_add(close).saturating_add(ticks)))
}

/// Emphasis or strong emphasis starting at `at`, and where it ends.
fn emphasis(text: &str, at: usize, marker: char) -> Option<(Inline, usize)> {
    let strong = char_at(text, at.saturating_add(marker.len_utf8())) == Some(marker);
    let width = if strong { 2 } else { 1 };
    let open = at.saturating_add(width);
    if marker == '_' && is_word(text.get(..at).and_then(|before| before.chars().next_back())) {
        return None;
    }
    let first = char_at(text, open)?;
    if first.is_whitespace() {
        return None;
    }
    let body = text.get(open..)?;
    let mut search = 0;
    let close = loop {
        let found = body
            .get(search..)?
            .find(&marker.to_string().repeat(width))?
            .saturating_add(search);
        let previous = body.get(..found).and_then(|head| head.chars().next_back());
        let after = found.saturating_add(width);
        let ends_word = marker == '_' && is_word(char_at(body, after));
        let doubled = !strong && char_at(body, after) == Some(marker);
        if found == 0 || previous.is_some_and(char::is_whitespace) || ends_word || doubled {
            search = after;
            continue;
        }
        break found;
    };
    let inner = parse(body.get(..close)?);
    let end = open.saturating_add(close).saturating_add(width);
    Some((
        if strong {
            Inline::Strong(inner)
        } else {
            Inline::Emphasis(inner)
        },
        end,
    ))
}

/// A link starting at the bracket at `at`, and where it ends.
fn link(text: &str, at: usize) -> Option<(Inline, usize)> {
    let rest = text.get(at.saturating_add(1)..)?;
    let mut depth = 1usize;
    let mut close = None;
    for (offset, character) in rest.char_indices() {
        match character {
            '[' => depth = depth.saturating_add(1),
            ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    close = Some(offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let label = rest.get(..close)?;
    let tail = rest.get(close.saturating_add(1)..)?;
    if !tail.starts_with('(') {
        return None;
    }
    let mut depth = 1usize;
    let mut end = None;
    for (offset, character) in tail.char_indices().skip(1) {
        match character {
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    end = Some(offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    let target = tail.get(1..end)?.trim();
    let href = target
        .split_once(char::is_whitespace)
        .map_or(target, |(address, _)| address)
        .trim_matches(|c| c == '<' || c == '>')
        .to_owned();
    let consumed = at
        .saturating_add(1)
        .saturating_add(close)
        .saturating_add(1)
        .saturating_add(end)
        .saturating_add(1);
    Some((
        Inline::Link {
            content: parse(label),
            href,
        },
        consumed,
    ))
}

/// An image renders as the link it is, showing its description.
fn image(inline: Inline) -> Inline {
    match inline {
        Inline::Link { content, href } => Inline::Image {
            source: href,
            alt: plain(&content),
        },
        other => other,
    }
}

/// An address in angle brackets, and where it ends.
fn autolink(text: &str, at: usize) -> Option<(Inline, usize)> {
    let rest = text.get(at.saturating_add(1)..)?;
    let close = rest.find('>')?;
    let inner = rest.get(..close)?;
    if inner.is_empty() || inner.contains(char::is_whitespace) {
        return None;
    }
    let is_address = inner.starts_with("http://")
        || inner.starts_with("https://")
        || (inner.contains('@') && !inner.contains('/'));
    if !is_address {
        return None;
    }
    let href = if inner.contains('@') && !inner.contains("://") {
        format!("mailto:{inner}")
    } else {
        inner.to_owned()
    };
    Some((
        Inline::Link {
            content: vec![Inline::Text(inner.to_owned())],
            href,
        },
        at.saturating_add(close).saturating_add(2),
    ))
}
