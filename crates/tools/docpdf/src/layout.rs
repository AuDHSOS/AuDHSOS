// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Markdown blocks onto pages.
//!
//! The layout walks the blocks in order and keeps one number: the height
//! the next thing goes at, counted from the bottom of the page as PDF
//! does. Anything that does not fit below it starts a page, and the two
//! decorations that outlive a page break — the bar beside a quotation and
//! the ground under a code block — are drawn per page rather than per
//! block, so a quotation that runs over a page break gets a bar on both
//! pages instead of one bar off the bottom of the first.

use doc_markdown::{Align, Block, Inline, plain};
use doc_pdf::Document;
use doc_pdf::font::Font;
use doc_pdf::page::{Link, LinkTarget, Page};
use doc_pdf::units::{Color, Mils, pt};

use crate::links::Links;
use crate::sources::Source;
use crate::text::{Line, Style, extent, lines};
use crate::theme;

/// A heading, on its way to the outline.
struct Bookmark {
    /// The depth, zero for the top level.
    level: usize,
    /// What the entry says.
    title: String,
    /// The page it points at.
    page: usize,
    /// The height on that page.
    top: Mils,
}

/// A run of one style, on its way onto the page.
struct Run {
    /// Where it starts.
    x: Mils,
    /// How wide it is.
    width: Mils,
    /// The text.
    text: String,
    /// How it is set.
    style: Style,
}

/// A bar down the left of a quotation, from where it started on this page.
struct Bar {
    /// The page it is on.
    page: usize,
    /// Its left edge.
    x: Mils,
    /// Where it started.
    top: Mils,
}

/// The state of a document being laid out.
pub(crate) struct Layout<'a> {
    /// The pages so far.
    pages: Vec<Page>,
    /// The headings, in the order they were met.
    bookmarks: Vec<Bookmark>,
    /// The quotation bars still open, innermost last.
    bars: Vec<Bar>,
    /// The height the next block goes at.
    y: Mils,
    /// The space left after a paragraph, which a tight list turns down.
    spacing: Mils,
    /// What is being laid out.
    source: &'a Source,
    /// The table that rewrites a link between two documents.
    links: &'a Links,
}

impl<'a> Layout<'a> {
    /// Starts a document with its title block.
    pub(crate) fn new(source: &'a Source, links: &'a Links) -> Self {
        let mut layout = Self {
            pages: Vec::new(),
            bookmarks: Vec::new(),
            bars: Vec::new(),
            y: 0,
            spacing: theme::PARAGRAPH,
            source,
            links,
        };
        layout.start_page();
        layout.title_block(&source.title, &source.origin);
        layout
    }

    /// Lays out a whole document, skipping a first heading that only
    /// repeats the title the page already carries.
    pub(crate) fn document(&mut self, blocks: &[Block]) {
        let mut blocks = blocks;
        if let Some(Block::Heading { level: 1, content }) = blocks.first()
            && plain(content).trim() == self.source.title.trim()
        {
            blocks = blocks.get(1..).unwrap_or_default();
        }
        self.blocks(blocks, 0);
    }

    /// The finished document.
    pub(crate) fn finish(self) -> Vec<u8> {
        let mut document = Document::new(self.source.title.clone());
        for page in self.pages {
            document.push(page);
        }
        for mark in self.bookmarks {
            document.outline(mark.level, mark.title, mark.page, mark.top);
        }
        document.finish()
    }

    /// How many pages the document has.
    pub(crate) const fn pages(&self) -> usize {
        self.pages.len()
    }

    /// A run of blocks at one indent.
    fn blocks(&mut self, blocks: &[Block], indent: Mils) {
        for block in blocks {
            self.block(block, indent);
        }
    }

    /// One block.
    fn block(&mut self, block: &Block, indent: Mils) {
        match block {
            Block::Heading { level, content } => self.heading(*level, content, indent),
            Block::Paragraph(content) => self.paragraph(content, indent),
            Block::Code { lines, .. } => self.code(lines, indent),
            Block::Quote(inner) => self.quote(inner, indent),
            Block::List {
                ordered,
                start,
                tight,
                items,
            } => self.list(*ordered, *start, *tight, items, indent),
            Block::Table {
                alignments,
                header,
                rows,
            } => self.table(alignments, header, rows, indent),
            Block::Rule => self.rule(indent),
        }
    }

    /// A heading, which is also an entry in the outline.
    fn heading(&mut self, level: usize, content: &[Inline], indent: Mils) {
        let size = theme::heading(level);
        self.advance(theme::space_before(level));
        // A heading alone at the foot of a page is a heading in the wrong
        // place: it takes its own height and two lines of what follows.
        self.ensure(
            size.leading
                .saturating_add(theme::space_after(level))
                .saturating_add(theme::BODY.leading.saturating_mul(2)),
        );
        self.bookmarks.push(Bookmark {
            level: level.saturating_sub(1),
            title: plain(content),
            page: self.pages.len().saturating_sub(1),
            top: self.y.saturating_add(pt(12)),
        });
        let style = Style::new(Font::Bold, size.size, theme::HEADING_INK);
        let measure = theme::MEASURE.saturating_sub(indent);
        let broken = lines(content, &style, measure);
        self.draw(&broken, indent, size.leading);
        self.advance(theme::space_after(level));
    }

    /// A paragraph.
    fn paragraph(&mut self, content: &[Inline], indent: Mils) {
        let style = Style::new(Font::Regular, theme::BODY.size, theme::INK);
        let measure = theme::MEASURE.saturating_sub(indent);
        let broken = lines(content, &style, measure);
        self.draw(&broken, indent, theme::BODY.leading);
        self.advance(self.spacing);
    }

    /// A code block: its ground is drawn per page, before the lines that
    /// sit on it, because a fill drawn afterwards would cover them.
    fn code(&mut self, body: &[String], indent: Mils) {
        let style = Style::new(Font::Mono, theme::CODE.size, theme::CODE_INK);
        let left = theme::SIDE.saturating_add(indent);
        let width = theme::MEASURE.saturating_sub(indent);
        let measure = width.saturating_sub(theme::PADDING.saturating_mul(2));
        let mut rows: Vec<String> = Vec::new();
        for line in body {
            let expanded = line.replace('\t', "    ");
            rows.extend(wrap_fixed(&expanded, &style, measure));
        }
        while rows.last().is_some_and(|line| line.trim().is_empty()) {
            rows.pop();
        }
        let mut rest = rows.as_slice();
        let mut first = true;
        while !rest.is_empty() {
            if first {
                self.ensure(theme::CODE.leading.saturating_add(theme::PADDING));
            } else {
                self.start_page();
            }
            let top = self.y;
            let room = top
                .saturating_sub(theme::BOTTOM)
                .saturating_sub(theme::PADDING.saturating_mul(2));
            let fits = room
                .checked_div(theme::CODE.leading)
                .and_then(|count| usize::try_from(count).ok())
                .unwrap_or(1)
                .max(1);
            let taken = fits.min(rest.len());
            let height = theme::CODE
                .leading
                .saturating_mul(Mils::try_from(taken).unwrap_or(0))
                .saturating_add(theme::PADDING.saturating_mul(2));
            self.fill(
                left,
                top.saturating_sub(height),
                width,
                height,
                theme::CODE_GROUND,
            );
            self.advance(theme::PADDING);
            for line in rest.get(..taken).unwrap_or_default() {
                let baseline = self.baseline(theme::CODE.size);
                self.set(left.saturating_add(theme::PADDING), baseline, line, &style);
                self.advance(theme::CODE.leading);
            }
            self.advance(theme::PADDING);
            rest = rest.get(taken..).unwrap_or_default();
            first = false;
        }
        self.advance(theme::PARAGRAPH);
    }

    /// A quotation: the blocks inside it, with a bar down the left.
    fn quote(&mut self, inner: &[Block], indent: Mils) {
        let x = theme::SIDE.saturating_add(indent);
        self.ensure(theme::BODY.leading.saturating_mul(2));
        self.bars.push(Bar {
            page: self.pages.len().saturating_sub(1),
            x,
            top: self.y,
        });
        self.blocks(inner, indent.saturating_add(theme::INDENT));
        if let Some(bar) = self.bars.pop() {
            self.draw_bar(&bar, self.y.saturating_add(theme::PARAGRAPH));
        }
    }

    /// A list, with its markers in the margin its items were indented by.
    ///
    /// A tight list is set with its items close together. The space after
    /// a paragraph is what would otherwise push them apart, so that is
    /// what is turned down, for as long as the list lasts and no longer —
    /// a loose list nested inside a tight one still gets its own spacing.
    fn list(&mut self, ordered: bool, start: u64, tight: bool, items: &[Vec<Block>], indent: Mils) {
        let outer = self.spacing;
        if tight {
            self.spacing = theme::TIGHT;
        }
        let style = Style::new(Font::Regular, theme::BODY.size, theme::INK);
        let inner = indent.saturating_add(theme::INDENT);
        for (offset, item) in items.iter().enumerate() {
            let number = start.saturating_add(u64::try_from(offset).unwrap_or(0));
            let marker = if ordered {
                format!("{}.", number.max(1))
            } else {
                "\u{2022}".to_owned()
            };
            self.ensure(theme::BODY.leading);
            let baseline = self.baseline(theme::BODY.size);
            let width = style.width(&marker);
            let x = theme::SIDE
                .saturating_add(inner)
                .saturating_sub(pt(6))
                .saturating_sub(width);
            self.set(x, baseline, &marker, &style);
            self.blocks(item, inner);
        }
        self.spacing = outer;
        if tight {
            self.advance(outer.saturating_sub(theme::TIGHT));
        }
    }

    /// A table. The header repeats on every page the table runs onto,
    /// because a table whose columns are only named on the first page is
    /// a table that has to be read backwards.
    fn table(
        &mut self,
        alignments: &[Align],
        header: &[Vec<Inline>],
        rows: &[Vec<Vec<Inline>>],
        indent: Mils,
    ) {
        let measure = theme::MEASURE.saturating_sub(indent);
        let widths = columns(header, rows, measure);
        let left = theme::SIDE.saturating_add(indent);
        self.ensure(theme::BODY.leading.saturating_mul(3));
        self.row(header, alignments, &widths, left, true);
        for row in rows {
            let height = Self::row_height(row, &widths);
            if self.y.saturating_sub(height) < theme::BOTTOM {
                self.start_page();
                self.row(header, alignments, &widths, left, true);
            }
            self.row(row, alignments, &widths, left, false);
        }
        self.advance(theme::PARAGRAPH);
    }

    /// How tall a row is once its cells are broken.
    fn row_height(cells: &[Vec<Inline>], widths: &[Mils]) -> Mils {
        let style = Style::new(Font::Regular, theme::BODY.size, theme::INK);
        let mut tallest = 1;
        for (index, cell) in cells.iter().enumerate() {
            let width = widths.get(index).copied().unwrap_or(0);
            let inner = width.saturating_sub(theme::PADDING.saturating_mul(2));
            tallest = tallest.max(lines(cell, &style, inner).len().max(1));
        }
        theme::BODY
            .leading
            .saturating_mul(Mils::try_from(tallest).unwrap_or(1))
            .saturating_add(pt(6))
    }

    /// One row of a table.
    fn row(
        &mut self,
        cells: &[Vec<Inline>],
        alignments: &[Align],
        widths: &[Mils],
        left: Mils,
        head: bool,
    ) {
        let font = if head { Font::Bold } else { Font::Regular };
        let style = Style::new(font, theme::BODY.size, theme::INK);
        let height = Self::row_height(cells, widths);
        let top = self.y;
        let total: Mils = widths.iter().copied().fold(0, Mils::saturating_add);
        if head {
            self.fill(
                left,
                top.saturating_sub(height),
                total,
                height,
                theme::TABLE_GROUND,
            );
        }
        let mut x = left;
        for (index, cell) in cells.iter().enumerate() {
            let width = widths.get(index).copied().unwrap_or(0);
            let inner = width.saturating_sub(theme::PADDING.saturating_mul(2));
            let broken = lines(cell, &style, inner);
            let align = alignments.get(index).copied().unwrap_or(Align::Left);
            let mut baseline = top
                .saturating_sub(pt(3))
                .saturating_sub(theme::BODY.size.saturating_mul(78).wrapping_div(100));
            for line in &broken {
                let slack = inner.saturating_sub(line.width);
                let shift = match align {
                    Align::Left => 0,
                    Align::Center => slack.wrapping_div(2),
                    Align::Right => slack,
                };
                self.place(
                    line,
                    x.saturating_add(theme::PADDING).saturating_add(shift),
                    baseline,
                );
                baseline = baseline.saturating_sub(theme::BODY.leading);
            }
            x = x.saturating_add(width);
        }
        self.y = top.saturating_sub(height);
        self.fill(left, self.y, total, 400, theme::RULE);
    }

    /// A horizontal rule.
    fn rule(&mut self, indent: Mils) {
        self.advance(pt(6));
        self.ensure(pt(8));
        let left = theme::SIDE.saturating_add(indent);
        let width = theme::MEASURE.saturating_sub(indent);
        self.fill(left, self.y, width, 500, theme::RULE);
        self.advance(pt(10));
    }

    /// Sets broken lines, starting a page whenever the next one would not
    /// fit.
    fn draw(&mut self, broken: &[Line], indent: Mils, leading: Mils) {
        for line in broken {
            self.ensure(leading);
            let baseline = self.baseline(line_size(line, leading));
            self.place(line, theme::SIDE.saturating_add(indent), baseline);
            self.advance(leading);
        }
    }

    /// Puts one broken line on the page.
    ///
    /// Neighbouring runs of the same style are set as one string. A line
    /// of prose is one text object rather than one per word, which is
    /// most of the difference between a readable content stream and a
    /// file three times the size.
    fn place(&mut self, line: &Line, x: Mils, baseline: Mils) {
        let mut run: Option<Run> = None;
        for piece in &line.pieces {
            let at = x.saturating_add(piece.offset);
            match run.as_mut() {
                Some(open)
                    if open.style == piece.style && open.x.saturating_add(open.width) == at =>
                {
                    open.text.push_str(&piece.text);
                    open.width = open.width.saturating_add(piece.width);
                }
                _ => {
                    if let Some(finished) = run.take() {
                        self.emit(&finished, baseline);
                    }
                    run = Some(Run {
                        x: at,
                        width: piece.width,
                        text: piece.text.clone(),
                        style: piece.style.clone(),
                    });
                }
            }
        }
        if let Some(finished) = run {
            self.emit(&finished, baseline);
        }
    }

    /// Sets one run, and underlines it if it is a link.
    fn emit(&mut self, run: &Run, baseline: Mils) {
        let text = run.text.trim_end();
        if text.is_empty() {
            return;
        }
        let width = run.style.width(text);
        let address = run
            .style
            .href
            .as_ref()
            .map(|href| self.links.resolve(self.source, href));
        let Some(page) = self.pages.last_mut() else {
            return;
        };
        page.text(
            run.style.font,
            run.style.size,
            run.x,
            baseline,
            run.style.color,
            text,
        );
        if let Some(href) = address {
            page.rect(
                run.x,
                baseline.saturating_sub(pt(2)),
                width,
                400,
                run.style.color,
            );
            page.link(Link {
                x: run.x,
                y: baseline.saturating_sub(pt(3)),
                width,
                height: run.style.size,
                target: LinkTarget::Uri(href),
            });
        }
    }

    /// Sets one run of fixed-pitch text.
    fn set(&mut self, x: Mils, baseline: Mils, text: &str, style: &Style) {
        if text.trim().is_empty() {
            return;
        }
        if let Some(page) = self.pages.last_mut() {
            page.text(style.font, style.size, x, baseline, style.color, text);
        }
    }

    /// Fills a rectangle on the current page.
    fn fill(&mut self, x: Mils, y: Mils, width: Mils, height: Mils, color: Color) {
        if let Some(page) = self.pages.last_mut() {
            page.rect(x, y, width, height, color);
        }
    }

    /// The baseline for a line of `size` at the current height.
    const fn baseline(&self, size: Mils) -> Mils {
        self.y
            .saturating_sub(size.saturating_mul(78).wrapping_div(100))
    }

    /// Moves down the page.
    const fn advance(&mut self, amount: Mils) {
        self.y = self.y.saturating_sub(amount);
    }

    /// Starts a page unless `height` still fits on this one.
    fn ensure(&mut self, height: Mils) {
        if self.y.saturating_sub(height) < theme::BOTTOM {
            self.start_page();
        }
    }

    /// Starts a page, closing every open bar on the one being left and
    /// reopening it at the top of the new one.
    fn start_page(&mut self) {
        let bottom = theme::BOTTOM;
        let bars: Vec<(usize, Mils, Mils)> = self
            .bars
            .iter()
            .map(|bar| (bar.page, bar.x, bar.top))
            .collect();
        for (page, x, top) in bars {
            self.draw_bar(&Bar { page, x, top }, bottom);
        }
        let number = self.pages.len().saturating_add(1);
        let mut page = Page::new(theme::PAGE);
        furniture(&mut page, &self.source.title, number);
        self.pages.push(page);
        self.y = theme::PAGE.height.saturating_sub(theme::TOP);
        let top = self.y;
        let page = self.pages.len().saturating_sub(1);
        for bar in &mut self.bars {
            bar.page = page;
            bar.top = top;
        }
    }

    /// Draws one bar from where it started down to `bottom`.
    fn draw_bar(&mut self, bar: &Bar, bottom: Mils) {
        let height = bar.top.saturating_sub(bottom);
        if height <= 0 {
            return;
        }
        if let Some(page) = self.pages.get_mut(bar.page) {
            page.rect(
                bar.x.saturating_sub(pt(2)),
                bottom,
                800,
                height,
                theme::RULE,
            );
        }
    }

    /// The head of the first page: the title, where the document came
    /// from, and a rule under both.
    fn title_block(&mut self, title: &str, origin: &str) {
        let style = Style::new(Font::Bold, theme::TITLE.size, theme::HEADING_INK);
        let broken = lines(&[Inline::Text(title.to_owned())], &style, theme::MEASURE);
        self.draw(&broken, 0, theme::TITLE.leading);
        let quiet = Style::new(Font::Mono, theme::FURNITURE.size, theme::QUIET);
        self.advance(pt(2));
        let baseline = self.baseline(theme::FURNITURE.size);
        self.set(theme::SIDE, baseline, origin, &quiet);
        self.advance(theme::FURNITURE.leading);
        self.advance(pt(6));
        self.fill(theme::SIDE, self.y, theme::MEASURE, 500, theme::RULE);
        self.advance(pt(18));
    }
}

/// The running head and the page number.
fn furniture(page: &mut Page, title: &str, number: usize) {
    let top = theme::PAGE.height.saturating_sub(pt(40));
    if number > 1 {
        page.text(
            Font::Regular,
            theme::FURNITURE.size,
            theme::SIDE,
            top,
            theme::QUIET,
            title,
        );
        page.rect(
            theme::SIDE,
            top.saturating_sub(pt(8)),
            theme::MEASURE,
            300,
            theme::RULE,
        );
    }
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

/// The size of the largest run on a line, for the baseline it sits on.
fn line_size(line: &Line, leading: Mils) -> Mils {
    line.pieces
        .iter()
        .map(|piece| piece.style.size)
        .max()
        .unwrap_or(leading)
}

/// Breaks a line of fixed-pitch text at the last character that fits.
fn wrap_fixed(line: &str, style: &Style, measure: Mils) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for character in line.chars() {
        let mut candidate = current.clone();
        candidate.push(character);
        if !current.is_empty() && style.width(&candidate) > measure {
            out.push(std::mem::take(&mut current));
        }
        current.push(character);
    }
    if !current.is_empty() || out.is_empty() {
        out.push(current);
    }
    out
}

/// The width of every column of a table.
///
/// A column asks for the width its widest cell would take unbroken, and
/// insists on the width of its longest single word: a column narrower than
/// that breaks words in half, which is how a table of two-word headings
/// ends up reading `Obje / ct`. When the columns together ask for more
/// than there is, the ones that can give way are cut back in proportion to
/// what they asked for, and a column already at its longest word gives
/// nothing. Only when even the longest words do not fit together is every
/// column cut below them, because at that point something has to break.
fn columns(header: &[Vec<Inline>], rows: &[Vec<Vec<Inline>>], measure: Mils) -> Vec<Mils> {
    let bold = Style::new(Font::Bold, theme::BODY.size, theme::INK);
    let body = Style::new(Font::Regular, theme::BODY.size, theme::INK);
    let padding = theme::PADDING.saturating_mul(2);
    let mut wanted: Vec<Mils> = Vec::new();
    let mut floors: Vec<Mils> = Vec::new();
    for cell in header {
        let (natural, longest) = extent(cell, &bold);
        wanted.push(natural.saturating_add(padding));
        floors.push(longest.saturating_add(padding));
    }
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            let (natural, longest) = extent(cell, &body);
            if let Some(slot) = wanted.get_mut(index) {
                *slot = (*slot).max(natural.saturating_add(padding));
            }
            if let Some(slot) = floors.get_mut(index) {
                *slot = (*slot).max(longest.saturating_add(padding));
            }
        }
    }
    let total = sum(&wanted);
    if total <= measure || total <= 0 {
        return wanted;
    }
    let needed = sum(&floors);
    if needed >= measure {
        // Nothing fits as it is; share what there is out by need.
        return floors
            .iter()
            .map(|floor| divide(floor.saturating_mul(measure), needed))
            .collect();
    }
    // Give every column its longest word, then hand out what is left in
    // proportion to what each column asked for beyond that.
    let spare = measure.saturating_sub(needed);
    let asked: Mils = wanted
        .iter()
        .zip(floors.iter())
        .map(|(want, floor)| want.saturating_sub(*floor))
        .fold(0, Mils::saturating_add);
    wanted
        .iter()
        .zip(floors.iter())
        .map(|(want, floor)| {
            let extra = want.saturating_sub(*floor);
            floor.saturating_add(divide(extra.saturating_mul(spare), asked))
        })
        .collect()
}

/// A division that answers zero rather than dividing by zero.
fn divide(value: Mils, by: Mils) -> Mils {
    value.checked_div(by).unwrap_or(0)
}

/// The sum of a list of lengths.
fn sum(widths: &[Mils]) -> Mils {
    widths.iter().copied().fold(0, Mils::saturating_add)
}
