// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! Tests of `crate::font`.

use crate::font::{
    FIRST_GLYPH, GLYPH_COUNT, GLYPH_HEIGHT, GLYPH_WIDTH, GLYPHS, LAST_GLYPH, REPLACEMENT,
    draw_text, glyph,
};
use crate::format::{Color, PixelFormat};
use crate::rect::Rect;
use crate::surface::Surface;

/// The Fowler-Noll-Vo hash of the sixteen rows of a glyph. It is here so
/// that a glyph that changes fails a test that names it, which a table
/// compared against itself would not do.
fn checksum(rows: &[u8; 16]) -> u32 {
    let mut hash = 0x811c_9dc5_u32;
    for row in rows {
        hash = (hash ^ u32::from(*row)).wrapping_mul(0x0100_0193);
    }
    hash
}

/// One checksum per glyph, in the order of the table.
const CHECKSUMS: [u32; GLYPH_COUNT] = [
    0x6969_1905,
    0x3941_7605,
    0x8300_6365,
    0xC6CF_820D,
    0x7078_4155,
    0xFBA3_197D,
    0x53EC_EFD5,
    0xC592_FB45,
    0x2CEC_DB95,
    0x32A2_05CD,
    0x254F_8161,
    0x96C6_95E1,
    0xE520_D4AD,
    0x2C30_4A61,
    0x5EC7_2985,
    0xAC75_EF51,
    0x8918_D3E1,
    0x8783_B98D,
    0xDB5B_20F1,
    0x673E_F6F9,
    0x3AF7_FFC9,
    0x8B16_344D,
    0x2016_2D75,
    0x149C_0CDD,
    0x4D56_1C6D,
    0xDEEC_EC2D,
    0x1C05_FD85,
    0x1C19_5D45,
    0xAB7C_F4F5,
    0xA1DC_46D5,
    0xB7C5_0999,
    0xFF6D_DF8D,
    0x5FAD_A395,
    0xB060_5721,
    0x3478_7DAD,
    0x99A6_C015,
    0x5F71_7B29,
    0xD6A5_6E0D,
    0x77DE_CF41,
    0xFC04_E051,
    0x7A60_F021,
    0x3407_7BC5,
    0x0F1E_D6B5,
    0xF208_DCC5,
    0x7E22_7909,
    0x0CC2_7AB1,
    0xB2F0_BA61,
    0xEEE1_45C9,
    0x80B7_C59D,
    0xDFBC_F021,
    0x86A3_5901,
    0x2FF7_F629,
    0xA6F5_9E59,
    0xCEB7_E835,
    0x1D5A_BE39,
    0x6EC3_B671,
    0x9261_5845,
    0xE6C8_8D75,
    0x0C1B_CF81,
    0x50B9_4975,
    0xD0D9_FFD5,
    0x15FC_CFBD,
    0x4D2A_2199,
    0xBA83_FB89,
    0x4606_39D5,
    0x2059_79FD,
    0x4320_CE11,
    0x98C3_8FC9,
    0x74A8_FA99,
    0xE57F_9375,
    0x1F91_3D81,
    0xE5B5_3605,
    0x1728_6DBD,
    0x9067_DE9D,
    0xB957_E5F5,
    0xB1AB_0665,
    0x22F3_8A4D,
    0xB32C_0F4D,
    0xE048_CC3D,
    0xFF35_1891,
    0x3FCC_B8DD,
    0x5548_D301,
    0xB7F1_3B01,
    0x14B2_65F5,
    0x01F8_5F81,
    0x45FB_3B49,
    0xF0A9_46A1,
    0x06A1_7F1D,
    0xE1FC_6325,
    0x0876_A88D,
    0xEC33_B2DD,
    0x0F93_6C95,
    0x0956_44D5,
    0x329D_691D,
    0x4855_F095,
];

/// A surface of `width` by `height` pixels over `bytes`.
fn surface(bytes: &mut [u8], width: u32, height: u32) -> Surface<'_> {
    Surface::new(bytes, width, height, width, PixelFormat::Rgbx8888).unwrap()
}

/// The bytes of a surface of this size.
fn buffer(width: u32, height: u32) -> Vec<u8> {
    let pixels = usize::try_from(width)
        .unwrap()
        .saturating_mul(usize::try_from(height).unwrap());
    vec![0; pixels.saturating_mul(4)]
}

#[test]
fn the_table_holds_one_glyph_per_printable_character() {
    assert_eq!(GLYPHS.len(), GLYPH_COUNT);
    assert_eq!(GLYPH_COUNT, 95);
    assert_eq!(u32::from(LAST_GLYPH) - u32::from(FIRST_GLYPH) + 1, 95);
    assert_eq!((GLYPH_WIDTH, GLYPH_HEIGHT), (8, 16));
}

#[test]
fn every_glyph_matches_its_checksum() {
    for (index, rows) in GLYPHS.iter().enumerate() {
        let wanted = CHECKSUMS.get(index).copied().unwrap_or(0);
        let character =
            char::from_u32(u32::from(FIRST_GLYPH) + u32::try_from(index).unwrap()).unwrap_or('?');
        assert_eq!(checksum(rows), wanted, "the glyph of {character:?} changed");
    }
}

#[test]
fn every_printable_character_has_its_own_glyph() {
    for code in u32::from(FIRST_GLYPH)..=u32::from(LAST_GLYPH) {
        let character = char::from_u32(code).unwrap();
        let index = usize::try_from(code - u32::from(FIRST_GLYPH)).unwrap();
        assert_eq!(glyph(character), GLYPHS.get(index).unwrap());
    }
}

#[test]
fn a_glyph_stands_inside_its_cell() {
    for rows in &GLYPHS {
        assert_eq!(rows.len(), usize::try_from(GLYPH_HEIGHT).unwrap());
        for row in rows {
            assert_eq!(row & 0x01, 0, "a glyph reaches into the last column");
        }
    }
}

#[test]
fn a_character_the_table_has_no_glyph_for_draws_the_replacement() {
    for character in ['\n', '\u{7F}', 'ä', '\u{1F600}', '\0'] {
        assert_eq!(glyph(character), &REPLACEMENT);
    }
}

#[test]
fn the_space_is_blank_and_the_letters_are_not() {
    assert!(glyph(' ').iter().all(|row| *row == 0));
    assert!(glyph('A').iter().any(|row| *row != 0));
}

#[test]
fn text_is_drawn_one_cell_after_another() {
    let mut bytes = buffer(24, 16);
    let mut picture = surface(&mut bytes, 24, 16);
    let written = draw_text(&mut picture, 0, 0, "AB", Color::WHITE, None);
    assert_eq!(written, Rect::new(0, 0, 16, 16));
    let first = glyph('A');
    for (row, bits) in first.iter().enumerate() {
        for column in 0..GLYPH_WIDTH {
            let set = bits & (1 << (7 - column)) != 0;
            let wanted = if set { Color::WHITE } else { Color::BLACK };
            let y = u32::try_from(row).unwrap();
            assert_eq!(picture.pixel(column, y), Some(wanted), "at {column},{y}");
        }
    }
}

#[test]
fn a_background_fills_the_cell_around_every_glyph() {
    let mut bytes = buffer(8, 16);
    let mut picture = surface(&mut bytes, 8, 16);
    let background = Color::new(0x20, 0x20, 0x20);
    draw_text(&mut picture, 0, 0, "!", Color::WHITE, Some(background));
    assert_eq!(picture.pixel(0, 0), Some(background));
    assert_eq!(picture.pixel(3, 4), Some(Color::WHITE));
}

#[test]
fn a_string_longer_than_the_row_is_clipped_at_the_edge() {
    let mut bytes = buffer(12, 16);
    let mut picture = surface(&mut bytes, 12, 16);
    let written = draw_text(&mut picture, 0, 0, "AAA", Color::WHITE, None);
    assert!(
        written.right() <= 12,
        "{written:?} reaches past the surface"
    );
    assert_eq!(picture.width(), 12);
}

#[test]
fn text_that_begins_outside_the_surface_writes_nothing() {
    let mut bytes = buffer(8, 16);
    let mut picture = surface(&mut bytes, 8, 16);
    let written = draw_text(&mut picture, 8, 0, "A", Color::WHITE, None);
    assert_eq!(written, Rect::EMPTY);
    assert!(picture.bytes().iter().all(|byte| *byte == 0));
}

#[test]
fn empty_text_draws_nothing() {
    let mut bytes = buffer(8, 16);
    let mut picture = surface(&mut bytes, 8, 16);
    assert_eq!(
        draw_text(&mut picture, 0, 0, "", Color::WHITE, None),
        Rect::EMPTY
    );
    assert!(picture.damage().is_empty());
}
