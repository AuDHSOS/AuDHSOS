// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The fonts, their metrics, and the encoding of text into bytes.
//!
//! Only the standard fourteen fonts are used, and only five of them. A
//! viewer carries those itself, so nothing has to be embedded: a document
//! this crate writes needs no font file, no font parser, and no licence
//! beyond its own. The price is the character set. Every font here is
//! encoded in `WinAnsiEncoding`, so a document can say anything Latin-1
//! can say, plus the dashes, the quotation marks, and the ellipsis that
//! prose in this repository actually uses. Anything else becomes a
//! question mark rather than a hole in the page.
//!
//! The width tables are the ones from the Adobe font metrics, in
//! thousandths of the point size. They are here because the line breaker
//! needs them before a single byte is written; the font dictionaries carry
//! no `/Widths` array, so a viewer measures with its own copy of the same
//! numbers.

use crate::units::Mils;

/// One of the fonts this crate can write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Font {
    /// Helvetica: the text face.
    Regular,
    /// Helvetica-Bold: headings and strong spans.
    Bold,
    /// Helvetica-Oblique: emphasised spans.
    Italic,
    /// Courier: code, and every line of an RFC.
    Mono,
    /// Courier-Bold: a heading inside code.
    MonoBold,
}

impl Font {
    /// Every font, in the order of their resource names.
    pub const ALL: [Font; 5] = [
        Font::Regular,
        Font::Bold,
        Font::Italic,
        Font::Mono,
        Font::MonoBold,
    ];

    /// The `BaseFont` name of the font dictionary.
    #[must_use]
    pub const fn base_name(self) -> &'static str {
        match self {
            Font::Regular => "Helvetica",
            Font::Bold => "Helvetica-Bold",
            Font::Italic => "Helvetica-Oblique",
            Font::Mono => "Courier",
            Font::MonoBold => "Courier-Bold",
        }
    }

    /// The name the content stream selects the font by.
    #[must_use]
    pub const fn resource(self) -> &'static str {
        match self {
            Font::Regular => "F1",
            Font::Bold => "F2",
            Font::Italic => "F3",
            Font::Mono => "F4",
            Font::MonoBold => "F5",
        }
    }

    /// Whether the font is a fixed-pitch one.
    #[must_use]
    pub const fn is_mono(self) -> bool {
        matches!(self, Font::Mono | Font::MonoBold)
    }

    /// The bold face of the same family.
    #[must_use]
    pub const fn bold(self) -> Self {
        match self {
            Font::Regular | Font::Bold | Font::Italic => Font::Bold,
            Font::Mono | Font::MonoBold => Font::MonoBold,
        }
    }

    /// The width table of the font.
    const fn widths(self) -> &'static [u16; 256] {
        match self {
            Font::Regular | Font::Italic => &HELVETICA,
            Font::Bold => &HELVETICA_BOLD,
            Font::Mono | Font::MonoBold => &COURIER,
        }
    }

    /// The advance of one encoded byte, in thousandths of the point size.
    #[must_use]
    pub fn advance(self, byte: u8) -> u16 {
        self.widths()
            .get(usize::from(byte))
            .copied()
            .unwrap_or(SPACE_FALLBACK)
    }

    /// The width of already encoded text set at `size`.
    #[must_use]
    pub fn width_of(self, encoded: &[u8], size: Mils) -> Mils {
        let mut total: Mils = 0;
        for byte in encoded {
            total = total.saturating_add(Mils::from(self.advance(*byte)));
        }
        total.saturating_mul(size).wrapping_div(1000)
    }

    /// The width of `text` set at `size`, encoding it on the way.
    #[must_use]
    pub fn width_of_str(self, text: &str, size: Mils) -> Mils {
        let mut total: Mils = 0;
        for byte in encode(text) {
            total = total.saturating_add(Mils::from(self.advance(byte)));
        }
        total.saturating_mul(size).wrapping_div(1000)
    }
}

/// The advance used for a byte no table has, and for the replacement of an
/// unrepresentable character.
const SPACE_FALLBACK: u16 = 278;

/// The byte a character outside `WinAnsiEncoding` is written as.
const REPLACEMENT: u8 = b'?';

/// Encodes text into `WinAnsiEncoding`.
///
/// The lower half is ASCII, the range from `0xA0` up is Latin-1, and the
/// range between the two holds the punctuation Windows put there. A
/// character that is in none of them becomes a question mark: a document
/// that says `?` where it meant `→` is wrong in one glyph, and a document
/// with a missing byte is wrong in every line after it.
#[must_use]
pub fn encode(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    for character in text.chars() {
        bytes.push(encode_char(character));
    }
    bytes
}

/// The `WinAnsiEncoding` byte of one character.
#[must_use]
pub fn encode_char(character: char) -> u8 {
    let point = u32::from(character);
    if (0x20..0x7F).contains(&point) || (0xA0..=0xFF).contains(&point) {
        return u8::try_from(point).unwrap_or(REPLACEMENT);
    }
    match character {
        '\u{20AC}' => 0x80, // euro
        '\u{201A}' => 0x82, // single low quotation mark
        '\u{0192}' => 0x83, // florin
        '\u{201E}' => 0x84, // double low quotation mark
        '\u{2026}' => 0x85, // ellipsis
        '\u{2020}' => 0x86, // dagger
        '\u{2021}' => 0x87, // double dagger
        '\u{02C6}' => 0x88, // circumflex
        '\u{2030}' => 0x89, // per mille
        '\u{0160}' => 0x8A, // S with caron
        '\u{2039}' => 0x8B, // single left angle quotation mark
        '\u{0152}' => 0x8C, // OE
        '\u{017D}' => 0x8E, // Z with caron
        '\u{2018}' => 0x91, // left single quotation mark
        '\u{2019}' => 0x92, // right single quotation mark
        '\u{201C}' => 0x93, // left double quotation mark
        '\u{201D}' => 0x94, // right double quotation mark
        '\u{2022}' => 0x95, // bullet
        '\u{2013}' => 0x96, // en dash
        '\u{2014}' => 0x97, // em dash
        '\u{02DC}' => 0x98, // small tilde
        '\u{2122}' => 0x99, // trade mark
        '\u{0161}' => 0x9A, // s with caron
        '\u{203A}' => 0x9B, // single right angle quotation mark
        '\u{0153}' => 0x9C, // oe
        '\u{017E}' => 0x9E, // z with caron
        '\u{0178}' => 0x9F, // Y with diaeresis
        // Three arrows and two comparisons that WinAnsi has no glyph
        // for, written as the ASCII they are read as anyway.
        '\u{2192}' | '\u{2265}' => b'>',
        '\u{2190}' | '\u{2264}' => b'<',
        _ => REPLACEMENT,
    }
}

/// Helvetica, and Helvetica-Oblique, which has the same advances.
const HELVETICA: [u16; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x00
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x10
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // 0x20
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, // 0x30
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, // 0x40
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, // 0x50
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, // 0x60
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584, 0, // 0x70
    556, 0, 222, 556, 333, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0, // 0x80
    0, 222, 222, 333, 333, 350, 556, 1000, 333, 1000, 500, 333, 944, 0, 500, 667, // 0x90
    278, 333, 556, 556, 556, 556, 260, 556, 333, 737, 370, 556, 584, 333, 737, 333, // 0xA0
    400, 584, 333, 333, 333, 556, 537, 278, 333, 333, 365, 556, 834, 834, 834, 611, // 0xB0
    667, 667, 667, 667, 667, 667, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278, // 0xC0
    722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611, // 0xD0
    556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556, 556, 278, 278, 278, 278, // 0xE0
    556, 556, 556, 556, 556, 556, 556, 584, 611, 556, 556, 556, 556, 500, 556, 500, // 0xF0
];

/// Helvetica-Bold.
const HELVETICA_BOLD: [u16; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x00
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x10
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, // 0x20
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, // 0x30
    975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, // 0x40
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556, // 0x50
    333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, // 0x60
    611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584, 0, // 0x70
    556, 0, 278, 556, 500, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0, // 0x80
    0, 278, 278, 500, 500, 350, 556, 1000, 333, 1000, 556, 333, 944, 0, 500, 667, // 0x90
    278, 333, 556, 556, 556, 556, 280, 556, 333, 737, 370, 556, 584, 333, 737, 333, // 0xA0
    400, 584, 333, 333, 333, 611, 556, 278, 333, 333, 365, 556, 834, 834, 834, 611, // 0xB0
    722, 722, 722, 722, 722, 722, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278, // 0xC0
    722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667, 611, // 0xD0
    556, 556, 556, 556, 556, 556, 889, 556, 556, 556, 556, 556, 278, 278, 278, 278, // 0xE0
    611, 611, 611, 611, 611, 611, 611, 584, 611, 611, 611, 611, 611, 556, 611, 556, // 0xF0
];

/// Courier and Courier-Bold: one advance for every glyph the font
/// has, and none for the positions `WinAnsiEncoding` leaves empty.
const COURIER: [u16; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x00
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 0x10
    600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, // 0x20
    600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, // 0x30
    600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, // 0x40
    600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, // 0x50
    600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, // 0x60
    600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 0, // 0x70
    600, 0, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 0, 600, 0, // 0x80
    0, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 0, 600, 600, // 0x90
    600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, // 0xA0
    600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, // 0xB0
    600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, // 0xC0
    600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, // 0xD0
    600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, // 0xE0
    600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, 600, // 0xF0
];
