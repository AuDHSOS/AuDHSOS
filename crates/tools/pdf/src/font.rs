// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The fonts, their metrics, and the encoding of text into bytes.
//!
//! Only the standard fourteen fonts are used, and only six of them. A
//! viewer carries those itself, so nothing has to be embedded: a document
//! this crate writes needs no font file, no font parser, and no licence
//! beyond its own. Five of the six are encoded in `WinAnsiEncoding`, so a
//! document can say anything Latin-1 can say, plus the dashes, the
//! quotation marks, and the ellipsis that prose in this repository
//! actually uses.
//!
//! The sixth is Symbol, and it is there for what the other five cannot
//! say. A character outside `WinAnsiEncoding` is written three ways, in
//! this order: in Symbol, if Symbol has it — which covers the infinity
//! sign, the mathematical relations, the arrows, and the Greek alphabet;
//! as the letters it is read as, if it has such a reading — which is how
//! the double-struck letters of a specification's numeric operations come
//! out as `F`, `R`, and `Z`; and as a question mark if it has neither,
//! because a document that says `?` where it meant a Hangul syllable is
//! wrong in one glyph, and a document with a missing byte is wrong in
//! every line after it. A combining mark is dropped rather than written,
//! since a mark on nothing is worse than the letter without it.
//!
//! Text therefore does not go onto a page as one string. It goes as the
//! runs its characters need, each in its own font, and the width of the
//! run before it is what puts the next one in the right place — which is
//! why [`runs`] is here beside the metrics rather than in the writer.
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
    /// Symbol: the characters the other five have no place for. Nothing
    /// asks for it; it is reached through [`runs`].
    Symbol,
}

impl Font {
    /// Every font, in the order of their resource names.
    pub const ALL: [Font; 6] = [
        Font::Regular,
        Font::Bold,
        Font::Italic,
        Font::Mono,
        Font::MonoBold,
        Font::Symbol,
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
            Font::Symbol => "Symbol",
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
            Font::Symbol => "F6",
        }
    }

    /// Whether the font is encoded in `WinAnsiEncoding`. Symbol is not:
    /// it carries an encoding of its own, and a font dictionary that
    /// overrode it would ask a viewer for glyphs the font does not have
    /// under those codes.
    #[must_use]
    pub const fn is_winansi(self) -> bool {
        !matches!(self, Font::Symbol)
    }

    /// Whether the font is a fixed-pitch one.
    #[must_use]
    pub const fn is_mono(self) -> bool {
        matches!(self, Font::Mono | Font::MonoBold)
    }

    /// The bold face of the same family. Symbol has none, and is its
    /// own.
    #[must_use]
    pub const fn bold(self) -> Self {
        match self {
            Font::Regular | Font::Bold | Font::Italic => Font::Bold,
            Font::Mono | Font::MonoBold => Font::MonoBold,
            Font::Symbol => Font::Symbol,
        }
    }

    /// The advance of one encoded byte, in thousandths of the point size.
    #[must_use]
    pub fn advance(self, byte: u8) -> u16 {
        let table = match self {
            Font::Regular | Font::Italic => &HELVETICA,
            Font::Bold => &HELVETICA_BOLD,
            Font::Mono | Font::MonoBold => &COURIER,
            // Symbol is not a table of 256: only the codes this crate
            // writes have a width here, and they stand beside the
            // characters they are reached by.
            Font::Symbol => {
                return SYMBOLS
                    .iter()
                    .find(|(_, code, _)| *code == byte)
                    .map_or(SPACE_FALLBACK, |(_, _, width)| *width);
            }
        };
        table
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

    /// The width of `text` set at `size`, in the fonts its characters
    /// need. A line breaker measuring with this gets the width the page
    /// will actually have, Symbol runs and all.
    #[must_use]
    pub fn width_of_str(self, text: &str, size: Mils) -> Mils {
        let mut total: Mils = 0;
        for run in runs(self, text) {
            total = total.saturating_add(run.font.width_of(&run.bytes, size));
        }
        total
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
/// character that is in none of them becomes a question mark here. Text
/// on a page does not come through this function but through [`runs`],
/// which offers such a character the Symbol font and a reading in Latin-1
/// first; this is the encoder for text that has to be one string, such as
/// the title in the catalogue.
#[must_use]
pub fn encode(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(text.len());
    for character in text.chars() {
        bytes.push(encode_char(character));
    }
    bytes
}

/// One piece of text as it will be set: the font it needs, and the bytes
/// to set in that font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// The font the piece is set in.
    pub font: Font,
    /// The bytes, encoded for that font.
    pub bytes: Vec<u8>,
}

/// Splits `text` into the runs its characters need.
///
/// Everything `WinAnsiEncoding` can say stays in `font`. A character it
/// cannot say is written in Symbol if Symbol has it, as the letters it is
/// read as if it has such a reading, as nothing at all if it is a
/// combining mark, and as a question mark if it is none of those. Runs
/// that need the same font are one run: the fewer pieces a line is set
/// in, the fewer places a width can be got wrong.
#[must_use]
pub fn runs(font: Font, text: &str) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for character in text.chars() {
        if let Some(byte) = winansi(character) {
            push(&mut out, font, &[byte]);
        } else if let Some((_, code, _)) = SYMBOLS.iter().find(|(known, _, _)| *known == character)
        {
            push(&mut out, Font::Symbol, &[*code]);
        } else if let Some(reading) = reading(character) {
            push(&mut out, font, &encode(reading));
        } else if !is_combining(character) {
            push(&mut out, font, &[REPLACEMENT]);
        }
    }
    out
}

/// Adds bytes to the run at the end, or begins one.
fn push(out: &mut Vec<Run>, font: Font, bytes: &[u8]) {
    match out.last_mut() {
        Some(run) if run.font == font => run.bytes.extend_from_slice(bytes),
        _ => out.push(Run {
            font,
            bytes: bytes.to_vec(),
        }),
    }
}

/// What a character is read as when no font here has it.
fn reading(character: char) -> Option<&'static str> {
    READINGS
        .iter()
        .find(|(known, _)| *known == character)
        .map(|(_, reading)| *reading)
}

/// Whether the character is a mark that would sit on the letter before
/// it. Nothing here can place one, and a mark set beside a letter reads
/// worse than the letter alone.
fn is_combining(character: char) -> bool {
    matches!(u32::from(character), 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x20D0..=0x20FF)
}

/// The `WinAnsiEncoding` byte of one character.
#[must_use]
pub fn encode_char(character: char) -> u8 {
    winansi(character).unwrap_or(REPLACEMENT)
}

/// The `WinAnsiEncoding` byte of one character, if the encoding has it.
fn winansi(character: char) -> Option<u8> {
    let point = u32::from(character);
    if (0x20..0x7F).contains(&point) || (0xA0..=0xFF).contains(&point) {
        return u8::try_from(point).ok();
    }
    let byte = match character {
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
        _ => return None,
    };
    Some(byte)
}

/// The characters `WinAnsiEncoding` has no place for that the Symbol font
/// has: the character, its code in that font, and its advance in
/// thousandths of the point size, from the Adobe metrics.
const SYMBOLS: [(char, u8, u16); 40] = [
    ('∀', 0x22, 713),
    ('∃', 0x24, 549),
    ('≅', 0x40, 549),
    ('Δ', 0x44, 612),
    ('Ω', 0x57, 768),
    ('\u{2126}', 0x57, 768), // the ohm sign, which is that letter
    ('∴', 0x5C, 863),
    ('α', 0x61, 631),
    ('β', 0x62, 549),
    ('δ', 0x64, 494),
    ('ε', 0x65, 439),
    ('γ', 0x67, 411),
    ('λ', 0x6C, 549),
    ('μ', 0x6D, 576),
    ('π', 0x70, 549),
    ('θ', 0x71, 521),
    ('ρ', 0x72, 549),
    ('σ', 0x73, 603),
    ('τ', 0x74, 439),
    ('ω', 0x77, 686),
    ('≤', 0xA3, 549),
    ('∞', 0xA5, 713),
    ('↔', 0xAB, 1042),
    ('←', 0xAC, 987),
    ('↑', 0xAD, 603),
    ('→', 0xAE, 987),
    ('↓', 0xAF, 603),
    ('≥', 0xB3, 549),
    ('∝', 0xB5, 713),
    ('∂', 0xB6, 494),
    ('≠', 0xB9, 549),
    ('≡', 0xBA, 549),
    ('≈', 0xBB, 549),
    ('∅', 0xC6, 823),
    ('∩', 0xC7, 768),
    ('∪', 0xC8, 768),
    ('⊂', 0xCC, 713),
    ('⊆', 0xCD, 713),
    ('∈', 0xCE, 713),
    ('∉', 0xCF, 713),
];

/// The characters no font here has that are read as letters that are in
/// it. The double-struck ones are the numeric operations of a language
/// specification, and `F`, `R`, and `Z` are what everybody says when they
/// read them aloud.
const READINGS: [(char, &str); 12] = [
    ('\u{1D53D}', "F"), // double-struck F
    ('\u{1D539}', "B"), // double-struck B
    ('ℝ', "R"),
    ('ℤ', "Z"),
    ('ℕ', "N"),
    ('ℚ', "Q"),
    ('ℂ', "C"),
    ('ℍ', "H"),
    ('\u{212A}', "K"), // the kelvin sign, which is that letter
    ('\u{212B}', "Å"), // the angstrom sign, which is that letter
    ('ſ', "s"),        // the long s
    ('\u{0100}', "A"), // A with a macron, which Latin-1 has no place for
];

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
