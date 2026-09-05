// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The requests for comments, which are already typeset.
//!
//! An RFC arrives as fixed-pitch text of seventy-two columns, already
//! divided into pages by form feeds, already carrying a running head and a
//! page number of its own. There is nothing to lay out and nothing to
//! reflow: the only job is to put each of those pages onto a page, in the
//! face the text was written for, and to leave every column where the
//! author put it. A diagram of a packet header survives exactly as long as
//! nobody tries to be clever with it.
//!
//! What is added is navigation. The numbered section headings, which sit
//! in the first column, become the outline, so a reader can jump to
//! section 3.2 of RFC 8200 instead of scrolling through ninety pages.

use doc_pdf::Document;
use doc_pdf::font::Font;
use doc_pdf::page::Page;
use doc_pdf::units::{Mils, pt};

use crate::sources::Source;
use crate::text::Style;
use crate::theme;

/// The width of the text an RFC is written in.
const COLUMNS: i64 = 72;

/// Converts the text of an RFC.
pub(crate) fn render(source: &Source, text: &str) -> (Vec<u8>, usize) {
    let style = Style::new(Font::Mono, theme::RFC.size, theme::INK);
    let width = style.width(&" ".repeat(usize::try_from(COLUMNS).unwrap_or(72)));
    let left =
        theme::SIDE.saturating_add(theme::MEASURE.saturating_sub(width).max(0).wrapping_div(2));
    let mut document = Document::new(source.title.clone());
    let mut pages: Vec<Page> = Vec::new();
    let mut marks: Vec<(usize, String, usize, Mils)> = Vec::new();
    let mut page = Page::new(theme::PAGE);
    let mut y = header(&mut page, source);
    pages.push(page);

    let mut first = true;
    for sheet in text.split('\u{c}') {
        let body: Vec<&str> = trimmed(sheet);
        if body.is_empty() {
            continue;
        }
        // Every sheet of the source starts a page here too, which is what
        // keeps a header diagram whole. The first one goes under the title
        // block rather than after it, so that a page is not spent on six
        // lines of heading.
        if !first {
            pages.push(Page::new(theme::PAGE));
            y = theme::PAGE.height.saturating_sub(theme::TOP);
        }
        first = false;
        for line in body {
            if y.saturating_sub(theme::RFC.leading) < theme::BOTTOM {
                pages.push(Page::new(theme::PAGE));
                y = theme::PAGE.height.saturating_sub(theme::TOP);
            }
            if let Some((level, title)) = section(line) {
                marks.push((
                    level,
                    title,
                    pages.len().saturating_sub(1),
                    y.saturating_add(pt(10)),
                ));
            }
            let baseline = y.saturating_sub(theme::RFC.size.saturating_mul(78).wrapping_div(100));
            if !line.trim().is_empty()
                && let Some(page) = pages.last_mut()
            {
                page.text(
                    style.font,
                    style.size,
                    left,
                    baseline,
                    style.color,
                    &line.replace('\t', "    "),
                );
            }
            y = y.saturating_sub(theme::RFC.leading);
        }
    }

    let total = pages.len();
    for (number, mut page) in pages.into_iter().enumerate() {
        footer(&mut page, number.saturating_add(1));
        document.push(page);
    }
    let mut seen: Vec<String> = Vec::new();
    for (level, title, page, top) in marks {
        if seen.contains(&title) {
            continue;
        }
        seen.push(title.clone());
        document.outline(level, title, page, top);
    }
    (document.finish(), total)
}

/// The title block of the first page, and the height the text starts at.
fn header(page: &mut Page, source: &Source) -> Mils {
    let top = theme::PAGE.height.saturating_sub(theme::TOP);
    let baseline = top.saturating_sub(theme::TITLE.size.saturating_mul(78).wrapping_div(100));
    page.text(
        Font::Bold,
        theme::TITLE.size,
        theme::SIDE,
        baseline,
        theme::HEADING_INK,
        &source.title,
    );
    let mut y = top
        .saturating_sub(theme::TITLE.leading)
        .saturating_sub(pt(2));
    page.text(
        Font::Mono,
        theme::FURNITURE.size,
        theme::SIDE,
        y.saturating_sub(pt(6)),
        theme::QUIET,
        &source.origin,
    );
    y = y
        .saturating_sub(theme::FURNITURE.leading)
        .saturating_sub(pt(6));
    page.rect(theme::SIDE, y, theme::MEASURE, 500, theme::RULE);
    y.saturating_sub(pt(16))
}

/// The page number at the foot of a page.
fn footer(page: &mut Page, number: usize) {
    let label = number.to_string();
    let width = Font::Regular.width_of_str(&label, theme::FURNITURE.size);
    let centre = theme::SIDE.saturating_add(theme::MEASURE.saturating_sub(width).wrapping_div(2));
    page.text(
        Font::Regular,
        theme::FURNITURE.size,
        centre,
        pt(38),
        theme::QUIET,
        &label,
    );
}

/// The lines of one sheet, without the blank lines at either end.
fn trimmed(sheet: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = sheet.lines().collect();
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines
}

/// The heading a line announces, if it is a numbered section in the first
/// column. A line of the table of contents looks the same but ends in a
/// page number behind a run of dots, which is what tells the two apart.
fn section(line: &str) -> Option<(usize, String)> {
    if line.starts_with(' ') || line.trim().is_empty() || line.contains("..") {
        return None;
    }
    let (number, rest) = line.split_once(char::is_whitespace)?;
    let number = number.strip_suffix('.').unwrap_or(number);
    if number.is_empty() || !number.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    if !number.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let title = rest.trim();
    if title.is_empty() || title.len() > 72 {
        return None;
    }
    let level = number.split('.').count().saturating_sub(1);
    Some((level.min(5), format!("{number}. {title}")))
}
