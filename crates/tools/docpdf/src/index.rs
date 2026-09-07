// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The index: one page that lists what was written, with a link to each.
//!
//! A directory of PDFs is not a document. The index is what turns the
//! output into one: it names every file, says how long it is and where it
//! came from, and links to it. It is written last, from what the run
//! actually produced, so a document that failed is missing from it rather
//! than listed as something that is not there.

use std::path::PathBuf;

use doc_pdf::Document;
use doc_pdf::font::Font;
use doc_pdf::page::{Link, LinkTarget, Page};
use doc_pdf::units::{Mils, pt};

use crate::sources::{SECTIONS, Source};
use crate::theme;

/// Writes the index over the documents that were produced.
pub(crate) fn render(
    sources: &[Source],
    written: &[(PathBuf, usize)],
    compress: bool,
) -> (Vec<u8>, usize) {
    let mut document = Document::new("AuDHSOS documentation");
    document.compress(compress);
    let mut pages: Vec<Page> = Vec::new();
    let mut marks: Vec<(usize, String, usize, Mils)> = Vec::new();
    let mut page = Page::new(theme::PAGE);
    let mut y = heading(&mut page, written.len());
    pages.push(page);

    for section in SECTIONS {
        let mut entries: Vec<(&Source, usize)> = Vec::new();
        for source in sources {
            let matches = source
                .target
                .parent()
                .is_some_and(|parent| parent.as_os_str() == section);
            if !matches {
                continue;
            }
            let Some((_, count)) = written.iter().find(|(path, _)| *path == source.target) else {
                continue;
            };
            entries.push((source, *count));
        }
        if entries.is_empty() {
            continue;
        }
        if y.saturating_sub(pt(60)) < theme::BOTTOM {
            pages.push(Page::new(theme::PAGE));
            y = theme::PAGE.height.saturating_sub(theme::TOP);
        }
        y = y.saturating_sub(pt(12));
        marks.push((
            0,
            section.to_owned(),
            pages.len().saturating_sub(1),
            y.saturating_add(pt(14)),
        ));
        if let Some(page) = pages.last_mut() {
            page.text(
                Font::Bold,
                theme::HEADING_3.size,
                theme::SIDE,
                y.saturating_sub(pt(10)),
                theme::HEADING_INK,
                section,
            );
        }
        y = y.saturating_sub(pt(18));
        for (source, count) in entries {
            if y.saturating_sub(theme::BODY.leading) < theme::BOTTOM {
                pages.push(Page::new(theme::PAGE));
                y = theme::PAGE.height.saturating_sub(theme::TOP);
            }
            if let Some(page) = pages.last_mut() {
                entry(page, source, count, y.saturating_sub(pt(8)));
            }
            y = y.saturating_sub(theme::BODY.leading);
        }
    }

    for page in pages {
        document.push(page);
    }
    for (level, title, page, top) in marks {
        document.outline(level, title, page, top);
    }
    let count = document.len();
    (document.finish(), count)
}

/// One line of the index: the file, what it is called, and how long it is.
fn entry(page: &mut Page, source: &Source, count: usize, baseline: Mils) {
    let href = source.target.to_string_lossy().replace('\\', "/");
    let label = file_name(&href);
    let width = Font::Regular.width_of_str(&label, theme::BODY.size);
    page.text(
        Font::Regular,
        theme::BODY.size,
        theme::SIDE,
        baseline,
        theme::LINK,
        &label,
    );
    page.rect(
        theme::SIDE,
        baseline.saturating_sub(pt(2)),
        width,
        400,
        theme::LINK,
    );
    page.link(Link {
        x: theme::SIDE,
        y: baseline.saturating_sub(pt(3)),
        width,
        height: theme::BODY.size,
        target: LinkTarget::Uri(href),
    });
    page.text(
        Font::Regular,
        theme::FURNITURE.size,
        theme::SIDE.saturating_add(NAME_COLUMN),
        baseline,
        theme::QUIET,
        &clip(
            &source.title,
            TITLE_COLUMN,
            Font::Regular,
            theme::FURNITURE.size,
        ),
    );
    let pages = count.to_string();
    let pages_width = Font::Regular.width_of_str(&pages, theme::FURNITURE.size);
    page.text(
        Font::Regular,
        theme::FURNITURE.size,
        theme::SIDE
            .saturating_add(theme::MEASURE)
            .saturating_sub(pages_width),
        baseline,
        theme::QUIET,
        &pages,
    );
}

/// How wide the column of file names is.
const NAME_COLUMN: Mils = pt(200);
/// How wide the column of titles is.
const TITLE_COLUMN: Mils = pt(230);

/// Shortens a label to fit a column, ending it in an ellipsis when it did
/// not.
fn clip(text: &str, width: Mils, font: Font, size: Mils) -> String {
    if font.width_of_str(text, size) <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    for character in text.chars() {
        let mut candidate = out.clone();
        candidate.push(character);
        candidate.push('\u{2026}');
        if font.width_of_str(&candidate, size) > width {
            out.push('\u{2026}');
            return out;
        }
        out.push(character);
    }
    out
}

/// The title block of the index.
fn heading(page: &mut Page, count: usize) -> Mils {
    let top = theme::PAGE.height.saturating_sub(theme::TOP);
    page.text(
        Font::Bold,
        theme::TITLE.size,
        theme::SIDE,
        top.saturating_sub(pt(16)),
        theme::HEADING_INK,
        "AuDHSOS documentation",
    );
    let subtitle = format!("{count} documents, written by docpdf");
    page.text(
        Font::Regular,
        theme::FURNITURE.size,
        theme::SIDE,
        top.saturating_sub(pt(30)),
        theme::QUIET,
        &subtitle,
    );
    let y = top.saturating_sub(pt(40));
    page.rect(theme::SIDE, y, theme::MEASURE, 500, theme::RULE);
    y
}

/// The file name of a path.
fn file_name(path: &str) -> String {
    path.rsplit_once('/')
        .map_or(path, |(_, name)| name)
        .to_owned()
}
