// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The block parser: lines in, blocks out.
//!
//! It works one line at a time and never looks further ahead than the next
//! line, except where the format itself demands it — a fenced block reads
//! until its fence closes, a table needs the row under its header, and a
//! setext heading is only a heading because of what follows it.
//!
//! A list item is parsed by cutting its lines out, removing the indent the
//! marker created, and running the whole parser over what is left. That is
//! why an item may hold a paragraph, a code block, and another list,
//! without any of those cases appearing here.

use crate::block::{Align, Block};
use crate::inline;

/// The width a tab is expanded to.
const TAB: usize = 4;

/// Parses a document into blocks.
#[must_use]
pub fn document(text: &str) -> Vec<Block> {
    let lines: Vec<String> = text.lines().map(expand_tabs).collect();
    blocks(&lines)
}

/// Expands leading tabs, so that indentation can be counted in spaces.
fn expand_tabs(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_indent = true;
    for character in line.chars() {
        match character {
            '\t' if in_indent => {
                let pad = TAB.saturating_sub(out.len().wrapping_rem(TAB));
                for _ in 0..pad {
                    out.push(' ');
                }
            }
            ' ' => out.push(' '),
            other => {
                in_indent = false;
                out.push(other);
            }
        }
    }
    out
}

/// Parses a run of lines into blocks.
fn blocks(lines: &[String]) -> Vec<Block> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(line) = lines.get(at) {
        if line.trim().is_empty() {
            at = at.saturating_add(1);
            continue;
        }
        if let Some((block, next)) = fenced_code(lines, at)
            .or_else(|| heading(lines, at))
            .or_else(|| thematic_break(lines, at))
            .or_else(|| quote(lines, at))
            .or_else(|| list(lines, at))
            .or_else(|| table(lines, at))
            .or_else(|| indented_code(lines, at))
        {
            out.push(block);
            at = next;
            continue;
        }
        let (block, next) = paragraph(lines, at);
        out.push(block);
        at = next;
    }
    out
}

/// The indentation of a line, in spaces.
fn indent(line: &str) -> usize {
    line.len().saturating_sub(line.trim_start().len())
}

/// A fenced code block.
fn fenced_code(lines: &[String], at: usize) -> Option<(Block, usize)> {
    let line = lines.get(at)?;
    let trimmed = line.trim_start();
    let offset = indent(line);
    if offset >= TAB {
        return None;
    }
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let width = trimmed.chars().take_while(|c| *c == marker).count();
    if width < 3 {
        return None;
    }
    let info = trimmed.get(width..)?.trim();
    if marker == '`' && info.contains('`') {
        return None;
    }
    let language = if info.is_empty() {
        None
    } else {
        Some(
            info.split_whitespace()
                .next()
                .unwrap_or(info)
                .trim_matches(|c| c == '{' || c == '}')
                .to_owned(),
        )
    };
    let mut body = Vec::new();
    let mut cursor = at.saturating_add(1);
    while let Some(line) = lines.get(cursor) {
        let closing = line.trim();
        if closing.chars().all(|c| c == marker) && closing.chars().count() >= width {
            cursor = cursor.saturating_add(1);
            return Some((
                Block::Code {
                    language,
                    lines: body,
                },
                cursor,
            ));
        }
        body.push(strip_indent(line, offset));
        cursor = cursor.saturating_add(1);
    }
    // A fence that never closes ends at the end of the document, which is
    // what a reader of the source sees too.
    Some((
        Block::Code {
            language,
            lines: body,
        },
        cursor,
    ))
}

/// Removes up to `width` spaces from the start of a line.
fn strip_indent(line: &str, width: usize) -> String {
    let removable = indent(line).min(width);
    line.get(removable..).unwrap_or("").to_owned()
}

/// An ATX heading, or a setext heading under its text.
fn heading(lines: &[String], at: usize) -> Option<(Block, usize)> {
    let (level, text) = heading_of(lines.get(at)?)?;
    Some((
        Block::Heading {
            level,
            content: inline::parse(text),
        },
        at.saturating_add(1),
    ))
}

/// The level and the text of an ATX heading, if the line is one.
fn heading_of(line: &str) -> Option<(usize, &str)> {
    if indent(line) >= TAB {
        return None;
    }
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = trimmed.get(level..)?;
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((level, rest.trim().trim_end_matches('#').trim()))
}

/// A horizontal rule.
fn thematic_break(lines: &[String], at: usize) -> Option<(Block, usize)> {
    let line = lines.get(at)?;
    if indent(line) >= TAB || !is_thematic_break(line) {
        return None;
    }
    Some((Block::Rule, at.saturating_add(1)))
}

/// Whether a line is a run of three or more of one rule character.
fn is_thematic_break(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(marker @ ('-' | '*' | '_')) = trimmed.chars().next() else {
        return false;
    };
    let marks = trimmed.chars().filter(|c| *c == marker).count();
    marks >= 3 && trimmed.chars().all(|c| c == marker || c == ' ')
}

/// A quotation, and everything that continues it.
fn quote(lines: &[String], at: usize) -> Option<(Block, usize)> {
    let line = lines.get(at)?;
    if indent(line) >= TAB || !line.trim_start().starts_with('>') {
        return None;
    }
    let mut inner = Vec::new();
    let mut cursor = at;
    while let Some(line) = lines.get(cursor) {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix('>') {
            inner.push(rest.strip_prefix(' ').unwrap_or(rest).to_owned());
        } else if trimmed.is_empty() || starts_block(line) {
            break;
        } else {
            // Lazy continuation: a quoted paragraph may run on without
            // the marker.
            inner.push(line.clone());
        }
        cursor = cursor.saturating_add(1);
    }
    Some((Block::Quote(blocks(&inner)), cursor))
}

/// The marker of a list item: how wide it is, whether it is numbered, and
/// the number it carries.
fn marker(line: &str) -> Option<(usize, bool, u64)> {
    let trimmed = line.trim_start();
    let offset = indent(line);
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        if is_thematic_break(line) {
            return None;
        }
        let spaces = rest.len().saturating_sub(rest.trim_start().len()).min(3);
        return Some((offset.saturating_add(2).saturating_add(spaces), false, 0));
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let number: u64 = trimmed.get(..digits)?.parse().ok()?;
    let rest = trimmed.get(digits..)?;
    let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
    let rest = rest.strip_prefix(' ')?;
    let spaces = rest.len().saturating_sub(rest.trim_start().len()).min(3);
    Some((
        offset
            .saturating_add(digits)
            .saturating_add(2)
            .saturating_add(spaces),
        true,
        number,
    ))
}

/// A list, with every item parsed as a document of its own.
///
/// The width of a marker is the column its text starts in. Cutting that
/// many characters off every line of an item leaves exactly the document
/// the item contains, indent and all, which is then parsed by the same
/// code that parsed the page around it.
fn list(lines: &[String], at: usize) -> Option<(Block, usize)> {
    let first = lines.get(at)?;
    if indent(first) >= TAB {
        return None;
    }
    let (mut width, ordered, start) = marker(first)?;
    let mut items: Vec<Vec<Block>> = Vec::new();
    let mut item: Vec<String> = Vec::new();
    let mut cursor = at;
    let mut blanks = 0usize;
    let mut tight = true;
    while let Some(line) = lines.get(cursor) {
        if line.trim().is_empty() {
            blanks = blanks.saturating_add(1);
            if blanks > 1 {
                break;
            }
            item.push(String::new());
            cursor = cursor.saturating_add(1);
            continue;
        }
        let next = marker(line).filter(|_| indent(line) < width);
        if let Some((next_width, next_ordered, _)) = next {
            if next_ordered != ordered {
                break;
            }
            if cursor > at {
                items.push(blocks(&item));
                item = Vec::new();
            }
            width = next_width;
            item.push(line.get(width..).unwrap_or("").to_owned());
        } else if indent(line) >= width {
            item.push(strip_indent(line, width));
        } else if blanks > 0 || starts_block(line) {
            break;
        } else {
            item.push(line.trim_start().to_owned());
        }
        // A blank line only makes a list loose if the list goes on after
        // it, which is known here and nowhere earlier: every line that
        // ends the list has already left the loop.
        if blanks > 0 {
            tight = false;
        }
        blanks = 0;
        cursor = cursor.saturating_add(1);
    }
    items.push(blocks(&item));
    items.retain(|item| !item.is_empty());
    if items.is_empty() {
        return None;
    }
    Some((
        Block::List {
            ordered,
            start,
            tight,
            items,
        },
        cursor,
    ))
}

/// A table: a header row, a row of dashes, and the body.
fn table(lines: &[String], at: usize) -> Option<(Block, usize)> {
    let header_line = lines.get(at)?;
    if !header_line.contains('|') {
        return None;
    }
    let delimiter = lines.get(at.saturating_add(1))?;
    let alignments = alignments(delimiter)?;
    let header: Vec<Vec<crate::inline::Inline>> = cells(header_line)
        .into_iter()
        .map(|cell| inline::parse(&cell))
        .collect();
    if header.len() != alignments.len() {
        return None;
    }
    let mut rows = Vec::new();
    let mut cursor = at.saturating_add(2);
    while let Some(line) = lines.get(cursor) {
        if !line.contains('|') || line.trim().is_empty() {
            break;
        }
        let mut row: Vec<Vec<crate::inline::Inline>> = cells(line)
            .into_iter()
            .map(|cell| inline::parse(&cell))
            .collect();
        row.resize_with(alignments.len(), Vec::new);
        row.truncate(alignments.len());
        rows.push(row);
        cursor = cursor.saturating_add(1);
    }
    Some((
        Block::Table {
            alignments,
            header,
            rows,
        },
        cursor,
    ))
}

/// The alignments a delimiter row declares, if it is one.
fn alignments(line: &str) -> Option<Vec<Align>> {
    if !line.contains('-') || !line.contains('|') {
        return None;
    }
    let mut out = Vec::new();
    for cell in cells(line) {
        let cell = cell.trim();
        if cell.is_empty() || !cell.chars().all(|c| c == '-' || c == ':') {
            return None;
        }
        out.push(match (cell.starts_with(':'), cell.ends_with(':')) {
            (true, true) => Align::Center,
            (false, true) => Align::Right,
            _ => Align::Left,
        });
    }
    if out.is_empty() { None } else { Some(out) }
}

/// The cells of a table row, with the outer pipes dropped.
fn cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let trimmed = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix('|').unwrap_or(trimmed);
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    for character in trimmed.chars() {
        match character {
            '\\' if !escaped => {
                escaped = true;
                cell.push('\\');
            }
            '|' if !escaped => out.push(std::mem::take(&mut cell).trim().to_owned()),
            other => {
                escaped = false;
                cell.push(other);
            }
        }
    }
    out.push(cell.trim().to_owned());
    out
}

/// A block of code marked by four spaces of indentation.
fn indented_code(lines: &[String], at: usize) -> Option<(Block, usize)> {
    let line = lines.get(at)?;
    if indent(line) < TAB || line.trim().is_empty() {
        return None;
    }
    let mut body = Vec::new();
    let mut cursor = at;
    let mut pending = 0usize;
    while let Some(line) = lines.get(cursor) {
        if line.trim().is_empty() {
            pending = pending.saturating_add(1);
            cursor = cursor.saturating_add(1);
            continue;
        }
        if indent(line) < TAB {
            break;
        }
        for _ in 0..pending {
            body.push(String::new());
        }
        pending = 0;
        body.push(strip_indent(line, TAB));
        cursor = cursor.saturating_add(1);
    }
    Some((
        Block::Code {
            language: None,
            lines: body,
        },
        cursor.saturating_sub(pending),
    ))
}

/// A paragraph, or the heading a line of `=` or `-` under it makes of it.
fn paragraph(lines: &[String], at: usize) -> (Block, usize) {
    let mut text = String::new();
    let mut cursor = at;
    while let Some(line) = lines.get(cursor) {
        if line.trim().is_empty() {
            break;
        }
        if cursor > at {
            if let Some(level) = setext(line) {
                return (
                    Block::Heading {
                        level,
                        content: inline::parse(&text),
                    },
                    cursor.saturating_add(1),
                );
            }
            if starts_block(line) {
                break;
            }
            text.push('\n');
        }
        text.push_str(line.trim_start());
        cursor = cursor.saturating_add(1);
    }
    if let Some(level) = lines.get(cursor).and_then(|line| setext(line)) {
        return (
            Block::Heading {
                level,
                content: inline::parse(&text),
            },
            cursor.saturating_add(1),
        );
    }
    (Block::Paragraph(inline::parse(&text)), cursor)
}

/// The level of a setext underline, if the line is one.
fn setext(line: &str) -> Option<usize> {
    let trimmed = line.trim();
    if trimmed.len() < 2 {
        return None;
    }
    if trimmed.chars().all(|c| c == '=') {
        return Some(1);
    }
    if trimmed.chars().all(|c| c == '-') {
        return Some(2);
    }
    None
}

/// Whether a line opens a block, and so ends the paragraph or the
/// quotation before it.
fn starts_block(line: &str) -> bool {
    if indent(line) >= TAB {
        return false;
    }
    let trimmed = line.trim_start();
    trimmed.starts_with('>')
        || trimmed.starts_with("```")
        || trimmed.starts_with("~~~")
        || heading_of(line).is_some()
        || is_thematic_break(line)
        || marker(line).is_some()
}
