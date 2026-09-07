// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The document, and how it becomes a file.
//!
//! Object numbers are handed out before anything is written: the catalogue,
//! the page tree, the information dictionary, the outline root, the five
//! fonts, then two objects per page and one per outline entry. Knowing the
//! numbers in advance is what lets a page refer to an outline entry and an
//! outline entry to a page without a second pass over the file.
//!
//! Nothing here records the time. Two runs over the same sources write
//! byte-identical files, so a rebuild that changes nothing is visible as a
//! rebuild that changed nothing.

use std::collections::BTreeMap;

use crate::outline::{Entry, Node, tree};
use crate::page::{LinkTarget, Page};
use crate::units::{Mils, number};

/// The catalogue.
const CATALOG: u32 = 1;
/// The page tree.
const PAGES: u32 = 2;
/// The information dictionary.
const INFO: u32 = 3;
/// The root of the outline.
const OUTLINE_ROOT: u32 = 4;
/// The first of the font dictionaries.
const FIRST_FONT: u32 = 5;
/// How many font dictionaries there are. It is the length of
/// [`crate::font::Font::ALL`], which a test holds it to: the pages are
/// numbered from after the fonts, so a font added without this number
/// would take an object number a page already has.
pub(crate) const FONTS: u32 = 6;
/// The first object of the first page, which follows the fonts.
const FIRST_PAGE: u32 = FIRST_FONT.saturating_add(FONTS);

/// A document under construction.
#[derive(Clone, Debug)]
pub struct Document {
    /// What the viewer shows as the title.
    title: String,
    /// The pages, in order.
    pages: Vec<Page>,
    /// The outline, flat, each entry carrying its level.
    outline: Vec<Entry>,
    /// Whether a content stream is deflated on the way into the file.
    compressed: bool,
}

impl Document {
    /// An empty document.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            pages: Vec::new(),
            outline: Vec::new(),
            compressed: false,
        }
    }

    /// Says whether the content streams are deflated on the way into the
    /// file.
    ///
    /// A file written without it can be read in a text editor, operator
    /// by operator, which is what these streams are written plainly for.
    /// A file written with it is about a third of the size, and what a
    /// reader sees instead is `FlateDecode` and a page of bytes.
    pub const fn compress(&mut self, compressed: bool) {
        self.compressed = compressed;
    }

    /// Appends a page and returns its index.
    /// Adds a page, and closes whatever it left open: a page that ended
    /// in the middle of a text object would be a stream no viewer accepts.
    pub fn push(&mut self, mut page: Page) -> usize {
        page.end_text();
        self.pages.push(page);
        self.pages.len().saturating_sub(1)
    }

    /// The index the next page will have.
    #[must_use]
    pub const fn next_index(&self) -> usize {
        self.pages.len()
    }

    /// How many pages the document has.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.pages.len()
    }

    /// Whether the document has no page at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Adds an outline entry. `level` starts at zero for a top-level
    /// entry; an entry deeper than one level below its predecessor is
    /// lifted to the level its predecessor allows, so a document whose
    /// headings skip a level still gets a well-formed outline.
    pub fn outline(&mut self, level: usize, title: impl Into<String>, page: usize, top: Mils) {
        let allowed = self
            .outline
            .last()
            .map_or(0, |last| last.level.saturating_add(1));
        self.outline.push(Entry {
            level: level.min(allowed),
            title: title.into(),
            page,
            top,
        });
    }

    /// Writes the document.
    #[must_use]
    pub fn finish(&self) -> Vec<u8> {
        let nodes = tree(&self.outline);
        let outline_base = FIRST_PAGE.saturating_add(count(self.pages.len()).saturating_mul(2));
        // The highest object number in use: the outline items run from
        // `outline_base`, so the last of them is one below the sum.
        let total = outline_base
            .saturating_add(count(nodes.len()))
            .saturating_sub(1);
        let mut file = File::new();
        file.header();
        Self::catalog(&mut file, &nodes);
        self.page_tree(&mut file);
        self.info(&mut file);
        Self::outline_root(&mut file, &nodes, outline_base);
        fonts(&mut file);
        for (index, page) in self.pages.iter().enumerate() {
            Self::page(&mut file, index, page, self.compressed);
        }
        for (index, node) in nodes.iter().enumerate() {
            Self::outline_item(&mut file, index, node, outline_base);
        }
        file.trailer(total);
        file.bytes
    }

    /// The catalogue, which names the page tree and, if there is one, the
    /// outline the viewer opens beside the page.
    fn catalog(file: &mut File, nodes: &[Node]) {
        file.object(CATALOG);
        file.push("<< /Type /Catalog /Pages ");
        file.reference(PAGES);
        if !nodes.is_empty() {
            file.push(" /Outlines ");
            file.reference(OUTLINE_ROOT);
            file.push(" /PageMode /UseOutlines");
        }
        file.push(" >>\n");
        file.end_object();
    }

    /// The page tree.
    fn page_tree(&self, file: &mut File) {
        file.object(PAGES);
        file.push("<< /Type /Pages /Count ");
        file.push(&self.pages.len().to_string());
        file.push(" /Kids [");
        for index in 0..self.pages.len() {
            file.push(" ");
            file.reference(page_object(index));
        }
        file.push(" ] >>\n");
        file.end_object();
    }

    /// The information dictionary.
    fn info(&self, file: &mut File) {
        file.object(INFO);
        file.push("<< /Title ");
        file.string(&self.title);
        file.push(" /Producer ");
        file.string("docpdf, part of AuDHSOS");
        file.push(" /Creator ");
        file.string("docpdf");
        file.push(" >>\n");
        file.end_object();
    }

    /// The outline root, written even when there is no entry so that the
    /// object numbers stay where every reference expects them.
    fn outline_root(file: &mut File, nodes: &[Node], base: u32) {
        file.object(OUTLINE_ROOT);
        file.push("<< /Type /Outlines");
        let top: Vec<usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.parent.is_none())
            .map(|(index, _)| index)
            .collect();
        if let (Some(first), Some(last)) = (top.first(), top.last()) {
            file.push(" /First ");
            file.reference(base.saturating_add(count(*first)));
            file.push(" /Last ");
            file.reference(base.saturating_add(count(*last)));
            file.push(" /Count ");
            file.push(&nodes.len().to_string());
        }
        file.push(" >>\n");
        file.end_object();
    }

    /// One page: its dictionary, its annotations, and its content stream.
    fn page(file: &mut File, index: usize, page: &Page, compressed: bool) {
        let size = page.size();
        file.object(page_object(index));
        file.push("<< /Type /Page /Parent ");
        file.reference(PAGES);
        file.push(" /MediaBox [0 0 ");
        file.push(&number(size.width));
        file.push(" ");
        file.push(&number(size.height));
        file.push("] /Resources << /Font << ");
        for (offset, font) in crate::font::Font::ALL.iter().enumerate() {
            file.push("/");
            file.push(font.resource());
            file.push(" ");
            file.reference(FIRST_FONT.saturating_add(count(offset)));
            file.push(" ");
        }
        file.push(">> >> /Contents ");
        file.reference(content_object(index));
        if !page.links().is_empty() {
            file.push(" /Annots [");
            for link in page.links() {
                file.push(" << /Type /Annot /Subtype /Link /Border [0 0 0] /Rect [");
                file.push(&number(link.x));
                file.push(" ");
                file.push(&number(link.y));
                file.push(" ");
                file.push(&number(link.x.saturating_add(link.width)));
                file.push(" ");
                file.push(&number(link.y.saturating_add(link.height)));
                file.push("] ");
                match &link.target {
                    LinkTarget::Uri(uri) => {
                        file.push("/A << /S /URI /URI ");
                        file.string(uri);
                        file.push(" >>");
                    }
                    LinkTarget::Page { index, top } => {
                        file.push("/Dest [");
                        file.reference(page_object(*index));
                        file.push(" /XYZ null ");
                        file.push(&number(*top));
                        file.push(" null]");
                    }
                }
                file.push(" >>");
            }
            file.push(" ]");
        }
        file.push(" >>\n");
        file.end_object();

        file.object(content_object(index));
        let stream = if compressed {
            deflate(page.content())
        } else {
            None
        };
        let bytes = stream.as_deref().unwrap_or_else(|| page.content());
        file.push("<< /Length ");
        file.push(&bytes.len().to_string());
        if stream.is_some() {
            file.push(" /Filter /FlateDecode");
        }
        file.push(" >>\nstream\n");
        file.bytes.extend_from_slice(bytes);
        file.push("\nendstream\n");
        file.end_object();
    }

    /// One outline entry.
    fn outline_item(file: &mut File, index: usize, node: &Node, base: u32) {
        let id = base.saturating_add(count(index));
        file.object(id);
        file.push("<< /Title ");
        file.string(&node.entry.title);
        file.push(" /Parent ");
        match node.parent {
            Some(parent) => file.reference(base.saturating_add(count(parent))),
            None => file.reference(OUTLINE_ROOT),
        }
        if let Some(previous) = node.previous {
            file.push(" /Prev ");
            file.reference(base.saturating_add(count(previous)));
        }
        if let Some(next) = node.next {
            file.push(" /Next ");
            file.reference(base.saturating_add(count(next)));
        }
        if let (Some(first), Some(last)) = (node.first, node.last) {
            file.push(" /First ");
            file.reference(base.saturating_add(count(first)));
            file.push(" /Last ");
            file.reference(base.saturating_add(count(last)));
            file.push(" /Count ");
            file.push(&node.descendants.to_string());
        }
        file.push(" /Dest [");
        file.reference(page_object(node.entry.page));
        file.push(" /XYZ null ");
        file.push(&number(node.entry.top));
        file.push(" null] >>\n");
        file.end_object();
    }
}

/// The font dictionaries. Symbol is the one without an `/Encoding`: it
/// carries its own, and overriding it would ask a viewer for glyphs the
/// font does not have under those codes.
fn fonts(file: &mut File) {
    for (offset, font) in crate::font::Font::ALL.iter().enumerate() {
        file.object(FIRST_FONT.saturating_add(count(offset)));
        file.push("<< /Type /Font /Subtype /Type1 /BaseFont /");
        file.push(font.base_name());
        if font.is_winansi() {
            file.push(" /Encoding /WinAnsiEncoding");
        }
        file.push(" >>\n");
        file.end_object();
    }
}

/// The object number of the page at `index`.
fn page_object(index: usize) -> u32 {
    FIRST_PAGE.saturating_add(count(index).saturating_mul(2))
}

/// A content stream, deflated, or nothing when deflating it would not
/// make it smaller — an empty page, or one whose operators are already
/// shorter than a stream that carries them.
///
/// What `FlateDecode` names is the format of RFC 1951 in the wrapper of
/// RFC 1950, not the bare stream: a viewer reads the two header bytes and
/// checks the four at the end, and a stream without them is one it
/// refuses.
fn deflate(content: &[u8]) -> Option<Vec<u8>> {
    if content.is_empty() {
        return None;
    }
    let mut scratch = Box::new(audhsos_deflate::Scratch::new());
    let mut out = vec![0u8; audhsos_deflate::bound_zlib(content.len())];
    let written = audhsos_deflate::compress_zlib(content, &mut out, &mut scratch).ok()?;
    if written >= content.len() {
        return None;
    }
    out.truncate(written);
    Some(out)
}

/// The object number of the content stream of the page at `index`.
fn content_object(index: usize) -> u32 {
    page_object(index).saturating_add(1)
}

/// A count as an object number, saturating rather than wrapping on a
/// document no machine could hold anyway.
fn count(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// The file as it is written, and where every object begins in it.
struct File {
    /// The bytes so far.
    bytes: Vec<u8>,
    /// Where each object begins, by object number.
    offsets: BTreeMap<u32, usize>,
}

impl File {
    /// A file with nothing written yet.
    const fn new() -> Self {
        Self {
            bytes: Vec::new(),
            offsets: BTreeMap::new(),
        }
    }

    /// The header. The second line is the comment with high bytes that
    /// tells any tool moving the file that it is not text.
    fn header(&mut self) {
        self.push("%PDF-1.7\n");
        self.bytes
            .extend_from_slice(&[b'%', 0xE2, 0xE3, 0xCF, 0xD3, b'\n']);
    }

    /// Begins an object, recording where it starts.
    fn object(&mut self, id: u32) {
        self.offsets.insert(id, self.bytes.len());
        self.push(&format!("{id} 0 obj\n"));
    }

    /// Ends an object.
    fn end_object(&mut self) {
        self.push("endobj\n");
    }

    /// An indirect reference.
    fn reference(&mut self, id: u32) {
        self.push(&format!("{id} 0 R"));
    }

    /// A literal string, escaped.
    fn string(&mut self, text: &str) {
        self.bytes.push(b'(');
        for byte in crate::font::encode(text) {
            crate::page::escape(byte, &mut self.bytes);
        }
        self.bytes.push(b')');
    }

    /// The cross-reference table and the trailer.
    fn trailer(&mut self, total: u32) {
        let start = self.bytes.len();
        let entries = total.saturating_add(1);
        self.push(&format!("xref\n0 {entries}\n"));
        self.push("0000000000 65535 f \n");
        for id in 1..=total {
            let offset = self.offsets.get(&id).copied().unwrap_or(0);
            self.push(&format!("{offset:010} 00000 n \n"));
        }
        self.push(&format!(
            "trailer\n<< /Size {entries} /Root {CATALOG} 0 R /Info {INFO} 0 R >>\nstartxref\n{start}\n%%EOF\n"
        ));
    }

    /// Appends text.
    fn push(&mut self, text: &str) {
        self.bytes.extend_from_slice(text.as_bytes());
    }
}
